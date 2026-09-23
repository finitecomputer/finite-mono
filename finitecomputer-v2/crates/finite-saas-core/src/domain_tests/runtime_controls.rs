use super::*;
use std::collections::BTreeSet;

#[tokio::test]
async fn user_can_request_and_runner_can_complete_oci_runtime_restart() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_id = complete_self_serve_agent(
            &db,
            "new@finite.vip",
            "user_workos_new",
            "first-submit",
            "oslo-agent-001",
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
        assert_eq!(restart.source_host_id, "oslo-host-1");
        assert_eq!(restart.source_machine_id, "oslo-agent-001");
        assert_eq!(restart.kind, RuntimeControlKind::Restart);
        assert_eq!(restart.status, RuntimeControlRequestStatus::Requested);

        let duplicate = db
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id: restart.project_id.clone(),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(duplicate.id, restart.id);

        let lease = db
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "restart-lease-1".to_string(),
                lease_seconds: Some(60),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .await
            .unwrap()
            .expect("restart request should lease");

        assert_eq!(lease.request.id, restart.id);
        assert_eq!(lease.request.status, RuntimeControlRequestStatus::Launching);
        assert_eq!(lease.runtime.source_machine_id, "oslo-agent-001");

        let stale_complete = db
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: restart.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "wrong-token".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: None,
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: None,
                now: Some("2026-05-25T13:04:30Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            stale_complete,
            CoreError::RuntimeControlRequestLeaseConflict
        ));

        let forbidden_refresh = db
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: restart.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "restart-lease-1".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                    RuntimeCapabilitiesV1 {
                        recover_known_good_chat: true,
                        ..*kata_runtime_capabilities().v1()
                    },
                )),
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: None,
                now: Some("2026-05-25T13:04:45Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            forbidden_refresh,
            CoreError::RuntimeUpgradeCompletionMismatch
        ));

        let completed = db
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: restart.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "restart-lease-1".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: None,
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: None,
                now: Some("2026-05-25T13:05:00Z".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(completed.status, RuntimeControlRequestStatus::Succeeded);
        assert!(completed.lease_token.is_none());
        assert_eq!(
            db.agent_runtime(&runtime_id)
                .await
                .unwrap()
                .host_facts
                .runtime_status,
            RuntimeSummaryStatus::Online
        );
        assert!(
            db.lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "restart-lease-2".to_string(),
                lease_seconds: Some(60),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some("2026-05-25T13:06:00Z".to_string()),
            })
            .await
            .unwrap()
            .is_none()
        );
    })
    .await;
}

#[tokio::test]
async fn restart_readiness_failure_is_named_and_never_reads_succeeded() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_id = complete_self_serve_agent(
            &db,
            "new@finite.vip",
            "user_workos_new",
            "first-submit",
            "oslo-agent-001",
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

        // The 2026-08-18 postmortem shape: a restart whose readiness wait
        // expires must end in a named failed state, never in succeeded.
        let restart = db
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id: project_id.clone(),
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .await
            .unwrap();
        db.lease_runtime_control_request(LeaseRuntimeControlRequestInput {
            runner_id: "runner-oslo-1".to_string(),
            lease_token: "restart-lease-1".to_string(),
            lease_seconds: Some(60),
            source_host_id: Some("oslo-host-1".to_string()),
            runner_capacity: Some(RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Kata],
                runtime_capabilities: Some(kata_runtime_capabilities()),
                ..RunnerLeaseCapacity::default()
            }),
            now: Some("2026-05-25T13:04:00Z".to_string()),
        })
        .await
        .unwrap()
        .expect("restart request should lease");
        let failed = db
            .fail_runtime_control_request(FailRuntimeControlRequestInput {
                request_id: restart.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "restart-lease-1".to_string(),
                failure_message: "runtime /healthz did not become ready within 180s".to_string(),
                failure_stage: Some(RuntimeLifecycleStage::Readiness),
                now: Some("2026-05-25T13:07:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(failed.status, RuntimeControlRequestStatus::Failed);
        assert_eq!(failed.failure_stage, Some(RuntimeLifecycleStage::Readiness));
        assert!(failed.completed_at.is_some());
        assert_eq!(
            db.agent_runtime(&runtime_id)
                .await
                .unwrap()
                .host_facts
                .runtime_status,
            RuntimeSummaryStatus::Stale
        );
        // The terminal row leaves the one-active index: a fresh request
        // is a new row, and the failed one cannot be leased again.
        let retry_restart = db
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id,
                now: Some("2026-05-25T13:08:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_ne!(retry_restart.id, failed.id);

        // An N-1 Runner names no stage; the failure still lands, marked
        // unknown rather than silently laundered into a real stage.
        db.lease_runtime_control_request(LeaseRuntimeControlRequestInput {
            runner_id: "runner-oslo-1".to_string(),
            lease_token: "restart-lease-2".to_string(),
            lease_seconds: Some(60),
            source_host_id: Some("oslo-host-1".to_string()),
            runner_capacity: Some(RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Kata],
                runtime_capabilities: Some(kata_runtime_capabilities()),
                ..RunnerLeaseCapacity::default()
            }),
            now: Some("2026-05-25T13:08:30Z".to_string()),
        })
        .await
        .unwrap()
        .expect("the fresh restart should lease");
        let legacy_failed = db
            .fail_runtime_control_request(FailRuntimeControlRequestInput {
                request_id: retry_restart.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "restart-lease-2".to_string(),
                failure_message: "n-1 runner failure".to_string(),
                failure_stage: None,
                now: Some("2026-05-25T13:09:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(legacy_failed.status, RuntimeControlRequestStatus::Failed);
        assert_eq!(
            legacy_failed.failure_stage,
            Some(RuntimeLifecycleStage::Unknown)
        );
    })
    .await;
}

#[tokio::test]
async fn stop_confirms_into_the_stopped_terminal() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_id = complete_self_serve_agent(
            &db,
            "new@finite.vip",
            "user_workos_new",
            "first-submit",
            "oslo-agent-001",
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

        let stop = db
            .request_runtime_stop(RequestRuntimeStopInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id,
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .await
            .unwrap();
        db.lease_runtime_control_request(LeaseRuntimeControlRequestInput {
            runner_id: "runner-oslo-1".to_string(),
            lease_token: "stop-lease-1".to_string(),
            lease_seconds: Some(60),
            source_host_id: Some("oslo-host-1".to_string()),
            runner_capacity: Some(RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Kata],
                runtime_capabilities: Some(kata_runtime_capabilities()),
                ..RunnerLeaseCapacity::default()
            }),
            now: Some("2026-05-25T13:04:00Z".to_string()),
        })
        .await
        .unwrap()
        .expect("stop request should lease");
        let stopped = db
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: stop.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "stop-lease-1".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: None,
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: None,
                now: Some("2026-05-25T13:05:00Z".to_string()),
            })
            .await
            .unwrap();
        // A stopped runtime never displays as ready/succeeded: its
        // terminal is Stopped and its host facts read offline.
        assert_eq!(stopped.status, RuntimeControlRequestStatus::Stopped);
        assert_eq!(stopped.failure_stage, None);
        assert_eq!(
            db.agent_runtime(&runtime_id)
                .await
                .unwrap()
                .host_facts
                .runtime_status,
            RuntimeSummaryStatus::Offline
        );
        // A replayed completion against the terminal row is refused.
        let replay = db
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: stop.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "stop-lease-1".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: None,
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: None,
                now: Some("2026-05-25T13:06:00Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            replay,
            CoreError::RuntimeRetirementSnapshotConflict
        ));
    })
    .await;
}

#[tokio::test]
async fn known_good_chat_recovery_is_fail_closed_until_a_real_recovery_path_exists() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_id = complete_self_serve_agent(
            &db,
            "new@finite.vip",
            "user_workos_new",
            "first-submit",
            "oslo-agent-001",
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

        let error = db
            .request_runtime_recover_known_good_chat(RequestRuntimeRecoverKnownGoodChatInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id,
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .await
            .unwrap_err();

        assert!(matches!(error, CoreError::RuntimeControlUnsupported));
        assert!(db.all_runtime_control_requests().await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn stop_is_supported_but_runtime_retirement_is_fail_closed() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_id = complete_self_serve_agent(
            &db,
            "new@finite.vip",
            "user_workos_new",
            "first-submit",
            "oslo-agent-001",
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
        let unrelated_runtime_id = complete_self_serve_agent(
            &db,
            "new@finite.vip",
            "user_workos_new",
            "second-submit",
            "oslo-agent-002",
            "artifact-v1",
            "2026-05-25T13:02:10Z",
        )
        .await;
        let unrelated_project_id = db
            .agent_runtime(&unrelated_runtime_id)
            .await
            .unwrap()
            .project_id
            .clone();
        let user_id = db
            .all_users()
            .await
            .iter()
            .find(|user| user.workos_user_id.as_deref() == Some("user_workos_new"))
            .unwrap()
            .id
            .clone();
        assert_eq!(db.visible_projects_for_user(&user_id).await.len(), 2);
        // A legacy relay credential row: nothing writes these anymore, but
        // destroy still clears any left behind by earlier Core generations.
        let relay_hash = "ab".repeat(32);
        db.exec(&format!(
            "INSERT INTO runtime_relay_credentials \
             (agent_runtime_id, token_hash, created_at, updated_at) \
             VALUES ('{runtime_id}', '{relay_hash}', \
             '2026-05-25T13:02:15Z', '2026-05-25T13:02:15Z')"
        ))
        .await;
        assert!(
            !db.query_json(
                "SELECT to_jsonb(t) FROM runtime_relay_credentials t \
             WHERE t.agent_runtime_id = $1",
                &[&runtime_id],
            )
            .await
            .is_empty()
        );
        let grant = db
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: Some("user_workos_new".to_string()),
                limit_profile_id: None,
                now: Some("2026-05-25T13:02:30Z".to_string()),
            })
            .await
            .unwrap();
        let runtime_key = db
            .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id,
                raw_key: "fpk_live_destroy_test".to_string(),
                project_id: Some(project_id.clone()),
                agent_runtime_id: Some(runtime_id.clone()),
                now: Some("2026-05-25T13:02:31Z".to_string()),
            })
            .await
            .unwrap();
        db.exec(&format!(
            "UPDATE agent_runtimes SET host_facts = jsonb_set(host_facts, \
             '{{published_app_urls}}', \
             '[\"https://oslo-agent.example.com/contact\"]'::jsonb) \
             WHERE id = '{runtime_id}'"
        ))
        .await;

        let stop = db
            .request_runtime_stop(RequestRuntimeStopInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id: project_id.clone(),
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .await
            .unwrap();
        let stop_lease = db
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "stop-lease-1".to_string(),
                lease_seconds: Some(60),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .await
            .unwrap()
            .expect("stop request should lease");
        assert_eq!(stop_lease.request.kind, RuntimeControlKind::Stop);
        db.complete_runtime_control_request(CompleteRuntimeControlRequestInput {
            request_id: stop.id,
            runner_id: "runner-oslo-1".to_string(),
            lease_token: "stop-lease-1".to_string(),
            runtime_artifact_id: None,
            state_schema_version: None,
            runtime_capabilities: None,
            runtime_host: None,
            published_app_urls: None,
            retirement_snapshot: None,
            now: Some("2026-05-25T13:05:00Z".to_string()),
        })
        .await
        .unwrap();
        let stopped_runtime = &db.agent_runtime(&runtime_id).await.unwrap();
        assert_eq!(
            stopped_runtime.host_facts.runtime_status,
            RuntimeSummaryStatus::Offline
        );
        assert_eq!(
            stopped_runtime.host_facts.published_app_urls,
            vec!["https://oslo-agent.example.com/contact".to_string()]
        );

        let destroy_error = db
            .request_runtime_destroy(RequestRuntimeDestroyInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id: project_id.clone(),
                now: Some("2026-05-25T13:06:00Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            destroy_error,
            CoreError::RuntimeControlUnsupported
        ));

        // A stale N-1 request cannot bypass the persisted-runtime and worker
        // capability intersection at lease time.
        let stale_destroy_id = "runtime_ctl_stale_destroy".to_string();
        db.exec(&format!(
            "INSERT INTO runtime_control_requests \
             (id, project_id, agent_runtime_id, source_host_id, source_machine_id, \
              requested_by_user_id, kind, status, created_at, updated_at) \
             VALUES ('{stale_destroy_id}', '{project_id}', '{runtime_id}', \
             'oslo-host-1', 'oslo-agent-001', '{user_id}', 'destroy', 'requested', \
             '2026-05-25T13:06:30Z', '2026-05-25T13:06:30Z')"
        ))
        .await;
        let stale_destroy_lease = db
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "destroy-lease-1".to_string(),
                lease_seconds: Some(60),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some("2026-05-25T13:07:00Z".to_string()),
            })
            .await
            .unwrap();
        assert!(stale_destroy_lease.is_none());
        assert_eq!(
            db.runtime_control_request(&stale_destroy_id)
                .await
                .unwrap()
                .status,
            RuntimeControlRequestStatus::Requested
        );
        assert!(
            !db.query_json(
                "SELECT to_jsonb(t) FROM runtime_relay_credentials t \
             WHERE t.agent_runtime_id = $1",
                &[&runtime_id],
            )
            .await
            .is_empty()
        );
        assert!(
            db.all("project_runtime_links")
                .await
                .iter()
                .any(|link| link["agent_runtime_id"] == runtime_id.as_str()
                    && link["active"] == true)
        );
        assert_eq!(
            db.finite_private_api_key(&runtime_key.id)
                .await
                .unwrap()
                .status,
            FinitePrivateApiKeyStatus::Active
        );
        assert!(
            !db.finite_private_admin_audit_events()
                .await
                .unwrap()
                .iter()
                .any(|event| event.action == "finite_private.runtime.destroy_revoke_keys")
        );
        let visible_project_ids = db
            .visible_projects_for_user(&user_id)
            .await
            .into_iter()
            .map(|visible| visible.project.id)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            visible_project_ids,
            BTreeSet::from([project_id.clone(), unrelated_project_id.clone()]),
            "unsupported retirement cannot hide either project"
        );
        assert!(
            db.project(&project_id).await.is_some(),
            "destroy retains the project row"
        );
        assert!(
            db.agent_runtime(&runtime_id).await.is_some(),
            "destroy retains the runtime row"
        );
        assert!(
            db.all("project_room_memberships")
                .await
                .iter()
                .find(|membership| membership["project_id"] == project_id.as_str())
                .unwrap()["archived_at"]
                .is_null()
        );
        assert!(
            db.all("project_room_memberships")
                .await
                .iter()
                .find(|membership| membership["project_id"] == unrelated_project_id.as_str())
                .unwrap()["archived_at"]
                .is_null(),
            "unrelated membership remains active"
        );
        assert!(db.agent_runtime(&unrelated_runtime_id).await.is_some());
    })
    .await;
}

#[tokio::test]
async fn admin_runtime_control_skips_owner_check_and_matches_runner_lease_shape() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_id = complete_self_serve_agent(
            &db,
            "owner@finite.vip",
            "user_workos_owner",
            "first-submit",
            "oslo-agent-001",
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

        // The owner-scoped path rejects non-owners outright.
        let denied = db
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "admin@finite.vip".to_string(),
                workos_user_id: "user_workos_admin".to_string(),
                project_id: project_id.clone(),
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(denied, CoreError::ProjectNotFound));

        // The admin path creates the request without owning the project.
        let restart = db
            .admin_request_runtime_restart(AdminRuntimeControlInput {
                admin_verified_email: "Admin@Finite.VIP".to_string(),
                admin_workos_user_id: "user_workos_admin".to_string(),
                project_id: project_id.clone(),
                now: Some("2026-05-25T13:03:30Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(restart.project_id, project_id);
        assert_eq!(restart.agent_runtime_id, runtime_id);
        assert_eq!(restart.source_host_id, "oslo-host-1");
        assert_eq!(restart.source_machine_id, "oslo-agent-001");
        assert_eq!(restart.kind, RuntimeControlKind::Restart);
        assert_eq!(restart.status, RuntimeControlRequestStatus::Requested);
        assert_eq!(
            restart.requested_by_user_id,
            Some(db.user_by_email("admin@finite.vip").await.unwrap().id)
        );

        // Idempotent while an equivalent request is pending, like the owner path.
        let duplicate = db
            .admin_request_runtime_restart(AdminRuntimeControlInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "user_workos_admin".to_string(),
                project_id: project_id.clone(),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(duplicate.id, restart.id);

        // The runner consumes it through the exact same lease machinery.
        let lease = db
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "admin-restart-lease-1".to_string(),
                lease_seconds: Some(60),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some("2026-05-25T13:04:30Z".to_string()),
            })
            .await
            .unwrap()
            .expect("admin restart request should lease");
        assert_eq!(lease.request.id, restart.id);
        assert_eq!(lease.request.status, RuntimeControlRequestStatus::Launching);
        assert_eq!(lease.runtime.source_machine_id, "oslo-agent-001");
        let completed = db
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: restart.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "admin-restart-lease-1".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: None,
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: None,
                now: Some("2026-05-25T13:05:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(completed.status, RuntimeControlRequestStatus::Succeeded);

        // Recovery is not restart-by-another-name: until a genuine recovery
        // implementation exists, even the admin path is fail closed.
        let recover_error = db
            .admin_request_runtime_recover_known_good_chat(AdminRuntimeControlInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "user_workos_admin".to_string(),
                project_id,
                now: Some("2026-05-25T13:06:00Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            recover_error,
            CoreError::RuntimeControlUnsupported
        ));

        let actions = db
            .finite_private_admin_audit_events()
            .await
            .unwrap()
            .iter()
            .map(|event| (event.action.clone(), event.actor.clone()))
            .collect::<Vec<_>>();
        assert!(actions.contains(&(
            "runtime.admin_restart".to_string(),
            "admin@finite.vip".to_string()
        )));
        assert!(
            !actions
                .iter()
                .any(|(action, _)| action == "runtime.admin_recover_known_good_chat")
        );
    })
    .await;
}

#[tokio::test]
async fn admin_runtime_overviews_assemble_provisioned_box_facts() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_id = complete_self_serve_agent(
            &db,
            "owner@finite.vip",
            "user_workos_owner",
            "first-submit",
            "oslo-agent-001",
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
        let grant = db
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "owner@finite.vip".to_string(),
                workos_user_id: Some("user_workos_owner".to_string()),
                limit_profile_id: None,
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap();
        db.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
            grant_id: grant.id.clone(),
            raw_key: "fpk_live_overview_key_material_00000004".to_string(),
            project_id: Some(project_id.clone()),
            agent_runtime_id: Some(runtime_id.clone()),
            now: Some(LATER.to_string()),
        })
        .await
        .unwrap();

        let overviews = db.admin_runtime_overviews().await.unwrap();
        assert_eq!(overviews.len(), 1);
        let overview = &overviews[0];
        assert_eq!(overview.project_id, project_id);
        assert_eq!(overview.agent_runtime_id, runtime_id);
        assert_eq!(overview.owner_email.as_deref(), Some("owner@finite.vip"));
        assert_eq!(overview.source_host_id, "oslo-host-1");
        assert_eq!(overview.source_machine_id, "oslo-agent-001");
        assert_eq!(overview.runtime_artifact_id.as_deref(), Some("artifact-v1"));
        assert_eq!(
            overview.runtime_artifact_version_label.as_deref(),
            Some("v1")
        );
        // The launch latched `online`; nothing has reported on the box
        // yet, so the user-facing status is the named unknown state.
        assert_eq!(overview.lifecycle_status, RuntimeSummaryStatus::Online);
        assert_eq!(overview.runtime_status, RuntimeSummaryStatus::Unknown);
        assert_eq!(overview.runtime_health.status, RuntimeHealthStatus::Unknown);
        assert_eq!(overview.hermes_available, Some(true));
        assert_eq!(overview.active_finite_private_key_count, 1);
        assert!(overview.runtime_link_active);
        assert_eq!(
            overview.runtime_capabilities,
            Some(*kata_runtime_capabilities().v1())
        );
    })
    .await;
}
