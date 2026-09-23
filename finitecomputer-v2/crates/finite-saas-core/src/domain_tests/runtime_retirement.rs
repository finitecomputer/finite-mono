use super::*;

/// Every phase pair is classified exactly by rank: forward moves and
/// same-phase restatements are allowed, any backward move is refused.
#[test]
fn offboarding_phase_transitions_are_forward_only() {
    let ordered = [
        OffboardingPhase::RetirementRequested,
        OffboardingPhase::ReceiptVerified,
        OffboardingPhase::ComputeRemoved,
        OffboardingPhase::LinkDeactivated,
        OffboardingPhase::Archived,
    ];
    assert!(OffboardingPhase::transition_allowed(None, ordered[0]));
    assert!(OffboardingPhase::transition_allowed(
        None,
        *ordered.last().unwrap()
    ));
    for (from_index, current) in ordered.iter().enumerate() {
        for (to_index, attempted) in ordered.iter().enumerate() {
            assert_eq!(
                OffboardingPhase::transition_allowed(Some(*current), *attempted),
                from_index <= to_index,
                "{current} -> {attempted}",
            );
            assert_eq!(current.reached(*attempted), from_index >= to_index);
        }
    }
}

/// The 0020 backfill mapping, mirrored by `from_legacy_facts`, over every
/// legacy flag combination. A verified receipt dominates (the destroy
/// completed, so compute is gone); an inactive link with no receipt and no
/// surviving project link is the archived-unrecoverable shape; an inactive
/// link superseded by another active link of the same project is a
/// relocation leftover, not an offboarding.
#[test]
fn offboarding_phase_maps_every_legacy_flag_combination() {
    use OffboardingPhase::*;
    let expected = |has_verified_receipt,
                    destroy_request_active,
                    link_active,
                    any_link_exists,
                    project_has_active_link| {
        OffboardingPhase::from_legacy_facts(
            has_verified_receipt,
            destroy_request_active,
            link_active,
            any_link_exists,
            project_has_active_link,
        )
    };
    for destroy_request_active in [false, true] {
        for any_link_exists in [false, true] {
            for project_has_active_link in [false, true] {
                // The half-retired ghost: receipt stored, link still active.
                assert_eq!(
                    expected(
                        true,
                        destroy_request_active,
                        true,
                        any_link_exists,
                        project_has_active_link
                    ),
                    Some(ComputeRemoved),
                );
                // Completed retirement: receipt stored, link deactivated.
                assert_eq!(
                    expected(
                        true,
                        destroy_request_active,
                        false,
                        any_link_exists,
                        project_has_active_link
                    ),
                    Some(Archived),
                );
                // Live runtime, with or without an in-flight destroy.
                assert_eq!(
                    expected(false, false, true, any_link_exists, project_has_active_link),
                    None,
                );
                assert_eq!(
                    expected(false, true, true, any_link_exists, project_has_active_link),
                    Some(RetirementRequested),
                );
                // Never linked: no offboarding evidence at all.
                assert_eq!(
                    expected(
                        false,
                        destroy_request_active,
                        false,
                        false,
                        project_has_active_link
                    ),
                    None,
                );
                // Inactive link but the project has another active
                // runtime: superseded by relocation, not offboarded.
                assert_eq!(
                    expected(false, destroy_request_active, false, true, true),
                    None,
                );
                // Inactive link, no receipt, no surviving project link:
                // unrecoverable archive or legacy offboard.
                assert_eq!(
                    expected(false, destroy_request_active, false, true, false),
                    Some(Archived),
                );
            }
        }
    }
}

#[tokio::test]
async fn retirement_requires_exact_immutable_receipt_and_retries_same_request() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_id = complete_self_serve_agent(
            &db,
            "new@finite.vip",
            "user_workos_new",
            "retirement-submit",
            "oslo-agent-retire",
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
        let retirement_capable =
            serde_json::to_string(&RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                runtime_retirement: true,
                ..*kata_runtime_capabilities().v1()
            }))
            .unwrap();
        db.exec(&format!(
            "UPDATE agent_runtimes SET runtime_capabilities = '{retirement_capable}'::jsonb \
             WHERE id = '{runtime_id}'"
        ))
        .await;
        let request = db
            .request_runtime_destroy(RequestRuntimeDestroyInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id: project_id.clone(),
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .await
            .unwrap();
        let capacity = RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::Kata],
            runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                runtime_retirement: true,
                ..*kata_runtime_capabilities().v1()
            })),
            ..RunnerLeaseCapacity::default()
        };
        let first = db
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "destroy-lease-1".to_string(),
                lease_seconds: Some(60),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(capacity.clone()),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        let spec = runtime_spec_v1(first.runtime_spec.as_ref().unwrap());
        let receipt = RuntimeRetirementSnapshotReceipt {
            schema: RUNTIME_RETIREMENT_SNAPSHOT_SCHEMA.to_string(),
            request_id: request.id.clone(),
            project_id: project_id.clone(),
            agent_runtime_id: runtime_id.clone(),
            durable_state_id: spec.durable_state_id.clone(),
            runtime_artifact_id: spec.runtime_artifact_id.clone(),
            backend: RUNTIME_RETIREMENT_BACKEND_BORG.to_string(),
            locator: runtime_retirement_archive_locator(&request.id),
            zip_bytes: 4096,
            zip_sha256: "a".repeat(64),
            manifest_sha256: "b".repeat(64),
            created_at: "2026-05-25T13:04:10Z".to_string(),
            verified_at: "2026-05-25T13:04:20Z".to_string(),
            recovery_authority_id: "finite-assisted-v1".to_string(),
            retention_policy: RUNTIME_RETIREMENT_RETENTION_INDEFINITE.to_string(),
        };

        let bare = db
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "destroy-lease-1".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: None,
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: None,
                now: Some("2026-05-25T13:04:25Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(bare, CoreError::RuntimeRetirementSnapshotMismatch));
        assert!(db.all("runtime_retirement_snapshots").await.is_empty());
        assert!(db.active_runtime_for_project(&project_id).await.is_some());

        db.renew_runtime_control_request(RenewRuntimeControlRequestInput {
            request_id: request.id.clone(),
            runner_id: "runner-oslo-1".to_string(),
            lease_token: "destroy-lease-1".to_string(),
            lease_seconds: Some(60),
            now: Some("2026-05-25T13:04:30Z".to_string()),
        })
        .await
        .unwrap();
        let retry = db
            .retry_runtime_control_request(RetryRuntimeControlRequestInput {
                request_id: request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "destroy-lease-1".to_string(),
                failure_message: "synthetic upload interruption".to_string(),
                now: Some("2026-05-25T13:04:40Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(retry.id, request.id);
        assert_eq!(retry.status, RuntimeControlRequestStatus::Requested);
        let second = db
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "destroy-lease-2".to_string(),
                lease_seconds: Some(60),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(capacity),
                now: Some("2026-05-25T13:04:45Z".to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.request.id, request.id);

        let completion = CompleteRuntimeControlRequestInput {
            request_id: request.id.clone(),
            runner_id: "runner-oslo-1".to_string(),
            lease_token: "destroy-lease-2".to_string(),
            runtime_artifact_id: None,
            state_schema_version: None,
            runtime_capabilities: None,
            runtime_host: None,
            published_app_urls: None,
            retirement_snapshot: Some(receipt.clone()),
            now: Some("2026-05-25T13:05:00Z".to_string()),
        };
        let completed = db
            .complete_runtime_control_request(completion.clone())
            .await
            .unwrap();
        // Retirement confirms into the Stopped terminal, never Succeeded:
        // a stopped runtime must not read as a ready one.
        assert_eq!(completed.status, RuntimeControlRequestStatus::Stopped);
        let snapshot = db
            .row("runtime_retirement_snapshots", &request.id)
            .await
            .expect("a completed retirement persists its snapshot");
        assert_eq!(snapshot["zip_sha256"], receipt.zip_sha256);
        assert_eq!(snapshot["manifest_sha256"], receipt.manifest_sha256);
        assert_eq!(snapshot["locator"], receipt.locator);
        assert_eq!(snapshot["backend"], receipt.backend);
        assert!(db.active_runtime_for_project(&project_id).await.is_none());
        assert_eq!(
            db.visible_projects_for_user(completed.requested_by_user_id.as_deref().unwrap())
                .await
                .len(),
            0
        );

        let replay = db
            .complete_runtime_control_request(completion)
            .await
            .expect("identical completion replay is idempotent");
        assert_eq!(replay.id, request.id);
        let mut conflicting = receipt;
        conflicting.zip_sha256 = "c".repeat(64);
        let conflict = db
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "destroy-lease-2".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: None,
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: Some(conflicting),
                now: Some("2026-05-25T13:05:01Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            conflict,
            CoreError::RuntimeRetirementSnapshotConflict
        ));
        assert_eq!(
            db.row("runtime_retirement_snapshots", &request.id)
                .await
                .unwrap()["zip_sha256"],
            "a".repeat(64),
            "a conflicting replay must not replace the immutable receipt"
        );
    })
    .await;
}

#[tokio::test]
async fn admin_runtime_retirement_requires_the_exact_active_binding() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_id = complete_self_serve_agent(
            &db,
            "owner@finite.vip",
            "user_workos_owner_retire",
            "retire-submit",
            "oslo-agent-retire",
            "artifact-v1",
            "2026-07-22T15:00:00Z",
        )
        .await;
        let project_id = db
            .agent_runtime(&runtime_id)
            .await
            .unwrap()
            .project_id
            .clone();
        let retirement_capable =
            serde_json::to_string(&RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                runtime_retirement: true,
                ..*kata_runtime_capabilities().v1()
            }))
            .unwrap();
        db.exec(&format!(
            "UPDATE agent_runtimes SET runtime_capabilities = '{retirement_capable}'::jsonb \
             WHERE id = '{runtime_id}'"
        ))
        .await;

        let changed_binding = db
            .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "user_workos_admin_retire".to_string(),
                project_id: project_id.clone(),
                expected_agent_runtime_id: "runtime-replaced-after-review".to_string(),
                expected_source_host_id: "oslo-host-1".to_string(),
                expected_source_machine_id: "oslo-agent-retire".to_string(),
                now: Some("2026-07-22T15:01:00Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(changed_binding, CoreError::RuntimeSpecMismatch));
        assert!(db.all_runtime_control_requests().await.is_empty());

        let retirement = db
            .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                admin_verified_email: "Admin@Finite.VIP".to_string(),
                admin_workos_user_id: "user_workos_admin_retire".to_string(),
                project_id,
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: "oslo-host-1".to_string(),
                expected_source_machine_id: "oslo-agent-retire".to_string(),
                now: Some("2026-07-22T15:02:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(retirement.kind, RuntimeControlKind::Destroy);
        assert_eq!(retirement.agent_runtime_id, runtime_id);
        assert_eq!(retirement.status, RuntimeControlRequestStatus::Requested);
        assert!(
            db.finite_private_admin_audit_events()
                .await
                .unwrap()
                .iter()
                .any(|event| {
                    event.action == "runtime.admin_destroy"
                        && event.actor == "admin@finite.vip"
                        && event.target_id == retirement.agent_runtime_id
                })
        );
    })
    .await;
}

#[tokio::test]
async fn unrecoverable_runtime_archive_is_exact_fail_closed_and_retains_history() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_id = complete_self_serve_agent(
            &db,
            "owner@finite.vip",
            "user_workos_owner_archive",
            "archive-submit",
            "legacy-agent-001",
            "artifact-v1",
            "2026-07-21T20:00:00Z",
        )
        .await;
        let project_id = db
            .agent_runtime(&runtime_id)
            .await
            .unwrap()
            .project_id
            .clone();
        let input = |compute_absent: bool| AdminArchiveUnrecoverableRuntimeInput {
            admin_verified_email: "admin@finite.vip".to_string(),
            admin_workos_user_id: "user_workos_admin_archive".to_string(),
            project_id: project_id.clone(),
            expected_agent_runtime_id: runtime_id.clone(),
            expected_source_host_id: "oslo-host-1".to_string(),
            expected_source_machine_id: "legacy-agent-001".to_string(),
            expected_owner_email: "owner@finite.vip".to_string(),
            operator_observed_compute_absent: compute_absent,
            operator_observed_durable_state_absent: true,
            owner_acknowledged_unrecoverable: true,
            now: Some("2026-07-21T20:10:00Z".to_string()),
        };

        // Compute must be observed absent.
        assert!(matches!(
            db.admin_archive_unrecoverable_runtime(input(false)).await,
            Err(CoreError::UnrecoverableRuntimeArchiveAcknowledgementRequired)
        ));
        assert!(db.active_runtime_for_project(&project_id).await.is_some());

        // The binding must match exactly.
        let mut wrong_binding_input = input(true);
        wrong_binding_input.expected_source_machine_id = "replacement-agent".to_string();
        assert!(matches!(
            db.admin_archive_unrecoverable_runtime(wrong_binding_input)
                .await,
            Err(CoreError::RuntimeSpecMismatch)
        ));

        // Provider metadata means the runtime is not actually unreachable.
        db.exec(&format!(
            "UPDATE agent_runtimes \
             SET contact_endpoint = 'https://legacy-agent.example.test/contact' \
             WHERE id = '{runtime_id}'"
        ))
        .await;
        assert!(matches!(
            db.admin_archive_unrecoverable_runtime(input(true)).await,
            Err(CoreError::UnrecoverableRuntimeArchiveProviderMetadataPresent)
        ));
        db.exec(&format!(
            "UPDATE agent_runtimes SET contact_endpoint = NULL WHERE id = '{runtime_id}'"
        ))
        .await;

        // An in-flight control operation blocks the archive until it settles.
        let in_flight = db
            .admin_request_runtime_restart(AdminRuntimeControlInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "user_workos_admin_archive".to_string(),
                project_id: project_id.clone(),
                now: Some("2026-07-21T20:05:00Z".to_string()),
            })
            .await
            .unwrap();
        assert!(matches!(
            db.admin_archive_unrecoverable_runtime(input(true)).await,
            Err(CoreError::RuntimeControlOperationConflict)
        ));
        db.exec(&format!(
            "UPDATE runtime_control_requests SET status = 'succeeded' WHERE id = '{}'",
            in_flight.id
        ))
        .await;

        let receipt = db
            .admin_archive_unrecoverable_runtime(input(true))
            .await
            .unwrap();
        assert_eq!(receipt.project_id, project_id);
        assert_eq!(receipt.agent_runtime_id, runtime_id);
        assert_eq!(receipt.owner_email, "owner@finite.vip");
        assert_eq!(receipt.revoked_finite_private_key_count, 0);

        // History is retained: the Project and Runtime rows survive, the
        // room membership is archived, and the action is audited.
        assert!(db.active_runtime_for_project(&project_id).await.is_none());
        assert!(db.project(&project_id).await.is_some());
        assert!(db.agent_runtime(&runtime_id).await.is_some());
        assert!(
            db.all("project_room_memberships")
                .await
                .iter()
                .any(|membership| {
                    membership["project_id"] == project_id.as_str()
                        && !membership["archived_at"].is_null()
                })
        );
        let events = db.finite_private_admin_audit_events().await.unwrap();
        assert!(events.iter().any(|event| {
            event.action == "runtime.admin_archive_unrecoverable"
                && event.target_id == runtime_id
                && event.actor == "admin@finite.vip"
        }));
    })
    .await;
}

#[tokio::test]
async fn offboard_retired_runtime_is_exact_fail_closed_and_keeps_the_receipt() {
    with_isolated_postgres(|db| async move {
        let (project_id, runtime_id, destroy_id) = stage_retired_offboard_anomaly(
            &db,
            "owner@finite.vip",
            "user_workos_owner_offboard",
            "offboard-submit",
            "retired-agent-001",
        )
        .await;
        let input = |compute_absent: bool| AdminOffboardRetiredRuntimeInput {
            admin_verified_email: "admin@finite.vip".to_string(),
            admin_workos_user_id: "user_workos_admin_offboard".to_string(),
            project_id: project_id.clone(),
            expected_agent_runtime_id: runtime_id.clone(),
            expected_source_host_id: "oslo-host-1".to_string(),
            expected_source_machine_id: "retired-agent-001".to_string(),
            expected_owner_email: "owner@finite.vip".to_string(),
            operator_observed_compute_absent: compute_absent,
            now: Some("2026-07-21T20:10:00Z".to_string()),
        };

        // Compute must be observed absent.
        assert!(matches!(
            db.admin_offboard_retired_runtime(input(false)).await,
            Err(CoreError::RetiredRuntimeOffboardAcknowledgementRequired)
        ));
        assert!(db.active_runtime_for_project(&project_id).await.is_some());

        // The binding must match exactly.
        let mut wrong_binding_input = input(true);
        wrong_binding_input.expected_source_machine_id = "replacement-agent".to_string();
        assert!(matches!(
            db.admin_offboard_retired_runtime(wrong_binding_input).await,
            Err(CoreError::RuntimeSpecMismatch)
        ));

        // The owner must match exactly.
        let mut wrong_owner_input = input(true);
        wrong_owner_input.expected_owner_email = "other@finite.vip".to_string();
        assert!(matches!(
            db.admin_offboard_retired_runtime(wrong_owner_input).await,
            Err(CoreError::RetiredRuntimeOffboardOwnerMismatch)
        ));

        let receipt_row_before = db
            .row("runtime_retirement_snapshots", &destroy_id)
            .await
            .expect("staged receipt must read back");
        let receipt = db
            .admin_offboard_retired_runtime(input(true))
            .await
            .unwrap();
        assert_eq!(receipt.project_id, project_id);
        assert_eq!(receipt.agent_runtime_id, runtime_id);
        assert_eq!(receipt.retirement_request_id, destroy_id);
        assert_eq!(
            receipt.retirement_locator,
            runtime_retirement_archive_locator(&destroy_id)
        );

        // Offboarding completed: the link is inactive and the membership
        // archived, while Project, Runtime, and receipt rows survive.
        assert!(db.active_runtime_for_project(&project_id).await.is_none());
        assert!(db.project(&project_id).await.is_some());
        assert!(db.agent_runtime(&runtime_id).await.is_some());
        assert!(
            db.all("project_room_memberships")
                .await
                .iter()
                .any(|membership| {
                    membership["project_id"] == project_id.as_str()
                        && !membership["archived_at"].is_null()
                })
        );
        assert_eq!(
            db.row("runtime_retirement_snapshots", &destroy_id)
                .await
                .unwrap(),
            receipt_row_before,
            "the repair must not touch the stored receipt"
        );
        let events = db.finite_private_admin_audit_events().await.unwrap();
        assert!(events.iter().any(|event| {
            event.action == "runtime.admin_offboard_retired"
                && event.target_id == runtime_id
                && event.actor == "admin@finite.vip"
        }));

        // A rerun fails closed on the inactive link.
        assert!(matches!(
            db.admin_offboard_retired_runtime(input(true)).await,
            Err(CoreError::ProjectRuntimeNotFound)
        ));
    })
    .await;
}
