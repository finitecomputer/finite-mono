use super::*;

/// Fail-closed repair for a Runtime whose destroy stored a verified
/// retirement receipt but whose offboarding never ran: the compute-absent
/// attestation, exact binding, and owner must match, an in-flight control
/// blocks the repair, a missing receipt refuses, and success completes the
/// normal offboarding boundary (link, membership, relay credential, keys,
/// departure fact, audit) without touching the receipt row.
#[tokio::test]
async fn postgres_admin_offboard_retired_runtime_completes_verified_retirement() {
    with_isolated_postgres(|store| async move {
        let run = "offboard-retired";
        let owner_email = format!("{run}-owner@finite.vip");
        let admin_email = format!("{run}-admin@finite.vip");
        let machine_id = format!("{run}-agent-001");
        let host = "offboard-retired-host";
        let launch_code = issue_test_launch_code(&store, "2026-07-21T12:00:00Z").await;
        store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: "artifact-offboard-retired-v1".to_string(),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/finite-agent-runtime:offboard-retired-v1@sha256:{}",
                    "4".repeat(64)
                ),
                version_label: "offboard-retired-v1".to_string(),
                source_git_sha: None,
                finitec_version: None,
                hermes_source_ref: None,
                finite_platform_plugin_ref: None,
                state_schema_version: "state-v1".to_string(),
                base_image: None,
                recover_known_good_chat: false,
                promoted: true,
                now: None,
            })
            .await
            .unwrap();
        let created = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: owner_email.clone(),
                workos_user_id: format!("workos_{run}_owner"),
                display_name: "Offboard Retired Agent".to_string(),
                launch_code: launch_code.clone(),
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
            .expect("offboard-retired request should lease");
        assert_eq!(lease.request.id, created.request.id);
        let provisioned = store
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id: lease.request.id.clone(),
                runner_id: format!("runner-{run}"),
                lease_token: format!("lease-{run}"),
                source_host_id: Some(host.to_string()),
                source_machine_id: Some(machine_id.clone()),
                now: None,
            })
            .await
            .unwrap();
        let completed = store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: lease.request.id.clone(),
                runner_id: format!("runner-{run}"),
                lease_token: format!("lease-{run}"),
                source_host_id: host.to_string(),
                source_machine_id: machine_id.clone(),
                runtime_artifact_id: Some("artifact-offboard-retired-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://127.0.0.1:41002/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Offboard Retired Agent".to_string()),
                hostname: None,
                runtime_host: Some(host.to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: vec!["http://127.0.0.1:41002/contact".to_string()],
                agent_npub: None,
                now: None,
            })
            .await
            .unwrap();
        let runtime_id = completed.request.agent_runtime_id.clone().unwrap();
        let project_id = completed.project.id.clone();

        // Stage the anomaly: a destroy whose verified receipt is stored but
        // whose offboarding never ran (link still active, endpoint set).
        let retirement_capable =
            serde_json::to_string(&RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                runtime_retirement: true,
                ..*kata_runtime_capabilities().v1()
            }))
            .unwrap();
        store
            .exec(&format!(
                "UPDATE agent_runtimes SET runtime_capabilities = '{retirement_capable}'::jsonb \
                     WHERE id = '{runtime_id}'"
            ))
            .await;
        let destroy = store
            .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                admin_verified_email: admin_email.clone(),
                admin_workos_user_id: format!("workos_{run}_admin"),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: host.to_string(),
                expected_source_machine_id: machine_id.clone(),
                now: Some("2026-07-21T12:01:00Z".to_string()),
            })
            .await
            .unwrap();
        let destroy_lease = store
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: format!("runner-{run}"),
                lease_token: format!("ctl-destroy-{run}"),
                lease_seconds: Some(60),
                source_host_id: Some(host.to_string()),
                runner_capacity: Some(crate::RunnerLeaseCapacity {
                    runner_classes: vec![crate::RunnerClass::Kata],
                    runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                        RuntimeCapabilitiesV1 {
                            runtime_retirement: true,
                            ..*kata_runtime_capabilities().v1()
                        },
                    )),
                    ..crate::RunnerLeaseCapacity::default()
                }),
                now: Some("2026-07-21T12:01:30Z".to_string()),
            })
            .await
            .unwrap()
            .expect("retirement should lease to a capable Kata runner");
        assert_eq!(destroy_lease.request.id, destroy.id);
        let destroy_spec = runtime_spec_v1(destroy_lease.runtime_spec.as_ref().unwrap());

        let input = |compute_absent: bool| AdminOffboardRetiredRuntimeInput {
            admin_verified_email: admin_email.clone(),
            admin_workos_user_id: format!("workos_{run}_admin"),
            project_id: project_id.clone(),
            expected_agent_runtime_id: runtime_id.clone(),
            expected_source_host_id: host.to_string(),
            expected_source_machine_id: machine_id.clone(),
            expected_owner_email: owner_email.clone(),
            operator_observed_compute_absent: compute_absent,
            now: Some("2026-07-21T12:05:00Z".to_string()),
        };

        // The compute-absent attestation is required.
        assert!(matches!(
            store
                .admin_offboard_retired_runtime(input(false))
                .await
                .unwrap_err(),
            CoreError::RetiredRuntimeOffboardAcknowledgementRequired
        ));
        assert!(
            store
                .active_runtime_for_project(&project_id)
                .await
                .is_some()
        );

        // An in-flight control operation blocks the repair.
        assert!(matches!(
            store
                .admin_offboard_retired_runtime(input(true))
                .await
                .unwrap_err(),
            CoreError::RuntimeControlOperationConflict
        ));
        // The older Core completed the destroy without offboarding.
        store
            .exec(&format!(
                "UPDATE runtime_control_requests \
                     SET status = 'stopped', lease_token = NULL, lease_expires_at = NULL, \
                         completed_at = CURRENT_TIMESTAMP \
                     WHERE id = '{}'",
                destroy.id
            ))
            .await;

        // Without a stored receipt the repair refuses.
        assert!(matches!(
            store
                .admin_offboard_retired_runtime(input(true))
                .await
                .unwrap_err(),
            CoreError::RetiredRuntimeOffboardReceiptMissing
        ));

        // The verified receipt the destroy stored before its offboarding
        // was lost.
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
                destroy_spec.durable_state_id,
                destroy_spec.runtime_artifact_id,
                crate::runtime_retirement_archive_locator(&destroy.id),
                "a".repeat(64),
                "b".repeat(64),
            ))
            .await;
        let receipt_row_before = store
            .row("runtime_retirement_snapshots", &destroy.id)
            .await
            .expect("staged receipt must read back");

        // The owner and the exact binding must match.
        let mut wrong_owner = input(true);
        wrong_owner.expected_owner_email = "someone-else@finite.vip".to_string();
        assert!(matches!(
            store
                .admin_offboard_retired_runtime(wrong_owner)
                .await
                .unwrap_err(),
            CoreError::RetiredRuntimeOffboardOwnerMismatch
        ));
        let mut wrong_binding = input(true);
        wrong_binding.expected_source_machine_id = "replacement-agent".to_string();
        assert!(matches!(
            store
                .admin_offboard_retired_runtime(wrong_binding)
                .await
                .unwrap_err(),
            CoreError::RuntimeSpecMismatch
        ));

        let receipt = store
            .admin_offboard_retired_runtime(input(true))
            .await
            .unwrap();
        assert_eq!(receipt.project_id, project_id);
        assert_eq!(receipt.agent_runtime_id, runtime_id);
        assert_eq!(receipt.retirement_request_id, destroy.id);
        assert_eq!(
            receipt.retirement_locator,
            crate::runtime_retirement_archive_locator(&destroy.id)
        );
        assert_eq!(receipt.revoked_finite_private_key_count, 1);

        // Offboarding completed: the link is inactive, the membership is
        // archived, and the runtime-scoped key is revoked.
        assert!(
            store
                .active_runtime_for_project(&project_id)
                .await
                .is_none()
        );
        assert!(store.project(&project_id).await.is_some());
        assert!(store.agent_runtime(&runtime_id).await.is_some());
        assert!(
            store
                .all("project_room_memberships")
                .await
                .iter()
                .any(|membership| {
                    membership["project_id"] == project_id.as_str()
                        && !membership["archived_at"].is_null()
                })
        );
        let key_after = store
            .finite_private_admin_state()
            .await
            .unwrap()
            .api_keys
            .into_iter()
            .find(|key| key.id == provisioned.api_key.id)
            .unwrap();
        assert_eq!(key_after.status, FinitePrivateApiKeyStatus::Revoked);

        // The receipt row is untouched.
        let receipt_row_after = store
            .row("runtime_retirement_snapshots", &destroy.id)
            .await
            .unwrap();
        assert_eq!(receipt_row_after, receipt_row_before);

        // The repair and the key revocation are audited.
        let events = store.finite_private_admin_audit_events().await.unwrap();
        assert!(events.iter().any(|event| {
            event.action == "runtime.admin_offboard_retired"
                && event.actor == admin_email
                && event.target_id == runtime_id
        }));
        assert!(events.iter().any(|event| {
            event.action == "finite_private.runtime.offboard_retired_revoke_keys"
                && event.target_id == runtime_id
        }));

        // A repair rerun fails closed on the inactive link.
        assert!(matches!(
            store
                .admin_offboard_retired_runtime(input(true))
                .await
                .unwrap_err(),
            CoreError::ProjectRuntimeNotFound
        ));
    })
    .await;
}

/// A verified retirement receipt is terminal: once a Runtime's destroy
/// stored its receipt, no registration path may flip its retired link back
/// to active. Launch and re-registration of a live Runtime (no receipt)
/// are unaffected.
#[tokio::test]
async fn postgres_verified_retirement_receipt_blocks_link_reactivation() {
    with_isolated_postgres(|store| async move {
        let run = "reactivate-retired";
        let owner_email = format!("{run}-owner@finite.vip");
        let admin_email = format!("{run}-admin@finite.vip");
        let machine_id = format!("{run}-agent-001");
        let host = "reactivate-retired-host";
        let launch_code = issue_test_launch_code(&store, "2026-07-21T12:00:00Z").await;
        store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: "artifact-reactivate-retired-v1".to_string(),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/finite-agent-runtime:reactivate-retired-v1@sha256:{}",
                    "5".repeat(64)
                ),
                version_label: "reactivate-retired-v1".to_string(),
                source_git_sha: None,
                finitec_version: None,
                hermes_source_ref: None,
                finite_platform_plugin_ref: None,
                state_schema_version: "state-v1".to_string(),
                base_image: None,
                recover_known_good_chat: false,
                promoted: true,
                now: None,
            })
            .await
            .unwrap();
        let created = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: owner_email.clone(),
                workos_user_id: format!("workos_{run}_owner"),
                display_name: "Reactivate Retired Agent".to_string(),
                launch_code: launch_code.clone(),
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
            .expect("reactivate-retired request should lease");
        let completed = store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: lease.request.id.clone(),
                runner_id: format!("runner-{run}"),
                lease_token: format!("lease-{run}"),
                source_host_id: host.to_string(),
                source_machine_id: machine_id.clone(),
                runtime_artifact_id: Some("artifact-reactivate-retired-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://127.0.0.1:41003/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Reactivate Retired Agent".to_string()),
                hostname: None,
                runtime_host: Some(host.to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: vec!["http://127.0.0.1:41003/contact".to_string()],
                agent_npub: None,
                now: None,
            })
            .await
            .unwrap();
        let runtime_id = completed.request.agent_runtime_id.clone().unwrap();
        let project_id = completed.project.id.clone();

        // Launch with no receipt activates the link.
        assert!(
            store
                .active_runtime_for_project(&project_id)
                .await
                .is_some()
        );

        // Re-registration of the live Runtime (a runner retry while the
        // request is still launching) is unaffected.
        let register_input = |lease_token: String, now: &str| RegisterAgentCreationRuntimeInput {
            request_id: created.request.id.clone(),
            runner_id: format!("runner-{run}"),
            lease_token,
            source_host_id: host.to_string(),
            source_machine_id: machine_id.clone(),
            runtime_artifact_id: Some("artifact-reactivate-retired-v1".to_string()),
            state_schema_version: Some("state-v1".to_string()),
            provider_runtime_handle: None,
            contact_endpoint: None,
            runtime_capabilities: Some(kata_runtime_capabilities()),
            display_name: Some("Reactivate Retired Agent".to_string()),
            hostname: None,
            runtime_host: Some(host.to_string()),
            runtime_status: Some(RuntimeSummaryStatus::Online),
            active_inference_profile: Some("finite-private".to_string()),
            hermes_available: Some(true),
            published_app_urls: Vec::new(),
            now: Some(now.to_string()),
        };
        store
            .exec(&format!(
                "UPDATE agent_creation_requests \
                     SET status = 'launching', lease_token = 'lease-reactivate-{run}', \
                         lease_expires_at = NULL \
                     WHERE id = '{}'",
                created.request.id
            ))
            .await;
        store
            .register_agent_creation_runtime(register_input(
                format!("lease-reactivate-{run}"),
                "2026-07-21T12:06:00Z",
            ))
            .await
            .unwrap();
        assert!(
            store
                .active_runtime_for_project(&project_id)
                .await
                .is_some()
        );
        store
            .exec(&format!(
                "UPDATE agent_creation_requests \
                     SET status = 'running', lease_token = NULL, lease_expires_at = NULL \
                     WHERE id = '{}'",
                created.request.id
            ))
            .await;

        // Retire the Runtime: a verified receipt is stored and offboarding
        // completes, deactivating the link.
        let retirement_capable =
            serde_json::to_string(&RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                runtime_retirement: true,
                ..*kata_runtime_capabilities().v1()
            }))
            .unwrap();
        store
            .exec(&format!(
                "UPDATE agent_runtimes SET runtime_capabilities = '{retirement_capable}'::jsonb \
                     WHERE id = '{runtime_id}'"
            ))
            .await;
        let destroy = store
            .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                admin_verified_email: admin_email.clone(),
                admin_workos_user_id: format!("workos_{run}_admin"),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: host.to_string(),
                expected_source_machine_id: machine_id.clone(),
                now: Some("2026-07-21T12:07:00Z".to_string()),
            })
            .await
            .unwrap();
        let destroy_lease = store
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: format!("runner-{run}"),
                lease_token: format!("ctl-destroy-{run}"),
                lease_seconds: Some(60),
                source_host_id: Some(host.to_string()),
                runner_capacity: Some(crate::RunnerLeaseCapacity {
                    runner_classes: vec![crate::RunnerClass::Kata],
                    runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                        RuntimeCapabilitiesV1 {
                            runtime_retirement: true,
                            ..*kata_runtime_capabilities().v1()
                        },
                    )),
                    ..crate::RunnerLeaseCapacity::default()
                }),
                now: Some("2026-07-21T12:07:30Z".to_string()),
            })
            .await
            .unwrap()
            .expect("retirement should lease to a capable Kata runner");
        let destroy_spec = runtime_spec_v1(destroy_lease.runtime_spec.as_ref().unwrap());
        store
            .exec(&format!(
                "UPDATE runtime_control_requests \
                     SET status = 'stopped', lease_token = NULL, lease_expires_at = NULL, \
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
                       8192, '{}', '{}', '2026-07-21T12:08:00Z',
                       '2026-07-21T12:08:30Z', 'finite-assisted-test',
                       'indefinite_until_purge', CURRENT_TIMESTAMP
                     )",
                destroy.id,
                project_id,
                runtime_id,
                destroy_spec.durable_state_id,
                destroy_spec.runtime_artifact_id,
                crate::runtime_retirement_archive_locator(&destroy.id),
                "a".repeat(64),
                "b".repeat(64),
            ))
            .await;
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
                now: Some("2026-07-21T12:08:45Z".to_string()),
            })
            .await
            .unwrap();
        assert!(
            store
                .active_runtime_for_project(&project_id)
                .await
                .is_none()
        );

        // A late registration retry for the retired Runtime fails closed:
        // the whole registration rolls back and the link stays retired.
        store
            .exec(&format!(
                "UPDATE agent_creation_requests \
                     SET status = 'launching', lease_token = 'lease-reactivate-{run}', \
                         lease_expires_at = NULL \
                     WHERE id = '{}'",
                created.request.id
            ))
            .await;
        let runtime_row_before = store.row("agent_runtimes", &runtime_id).await.unwrap();
        // The retired Runtime advertises its retirement capability; the
        // retry must match it to reach the link-activation guard.
        let mut retry = register_input(format!("lease-reactivate-{run}"), "2026-07-21T12:09:00Z");
        retry.runtime_capabilities = Some(RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
            runtime_retirement: true,
            ..*kata_runtime_capabilities().v1()
        }));
        let error = store
            .register_agent_creation_runtime(retry)
            .await
            .unwrap_err();
        assert!(
            matches!(error, CoreError::RuntimeRetirementSnapshotConflict),
            "unexpected error: {error:?}"
        );
        assert!(
            store
                .active_runtime_for_project(&project_id)
                .await
                .is_none()
        );
        assert_eq!(
            store.row("agent_runtimes", &runtime_id).await.unwrap(),
            runtime_row_before,
            "the refused registration must roll back its runtime upsert too"
        );
    })
    .await;
}
