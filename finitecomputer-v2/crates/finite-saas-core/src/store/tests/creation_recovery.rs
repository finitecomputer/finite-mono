use super::*;

#[tokio::test]
async fn postgres_persisted_machine_named_durable_state_id_is_repaired_on_read() {
    with_isolated_postgres(|store| async move {
        let run = "machine-named-spec";
        let host = "machine-named-host";
        let target_host = "machine-named-target";
        let machine = "finite-kata-machine-named";
        let email = format!("{run}@finite.vip");
        let workos = format!("workos-{run}");
        let launch_code = issue_test_launch_code(&store, "2026-07-25T12:00:00Z").await;
        let capacity = RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::Kata],
            runtime_capabilities: Some(kata_runtime_capabilities()),
            ..RunnerLeaseCapacity::default()
        };

        store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: "artifact-machine-named-v1".to_string(),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/agent-runtime:machine-named-v1@sha256:{}",
                    "5".repeat(64)
                ),
                version_label: "machine-named-v1".to_string(),
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
        store
            .request_agent_creation_configured(
                RequestAgentCreationInput {
                    verified_email: email.clone(),
                    workos_user_id: workos.clone(),
                    display_name: "Machine-Named Spec Canary".to_string(),
                    launch_code,
                    idempotency_key: format!("{run}-create"),
                    now: None,
                },
                AgentCreationConfiguration {
                    placement: Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
                    requested_hosting_tier: None,
                    profile_picture_url: None,
                    owner_chat_account_id: None,
                },
            )
            .await
            .unwrap();
        let created = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: format!("runner-{host}"),
                source_host_id: Some(host.to_string()),
                lease_token: "create-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(capacity.clone()),
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        let completed = store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: created.request.id.clone(),
                runner_id: format!("runner-{host}"),
                lease_token: "create-lease".to_string(),
                source_host_id: host.to_string(),
                source_machine_id: machine.to_string(),
                runtime_artifact_id: Some("artifact-machine-named-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://127.0.0.1:4205/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Machine-Named Spec Canary".to_string()),
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
        let project_id = completed.project.id;
        let runtime_id = completed.request.agent_runtime_id.unwrap();
        let creation_id = created.request.id;
        assert_ne!(runtime_id, machine);
        async fn persisted_durable_state_id(store: &TestDb, creation_id: &str) -> Value {
            store
                .query_json(
                    "SELECT runtime_spec->'spec'->'durableStateId'
                         FROM agent_creation_requests WHERE id = $1",
                    &[&creation_id],
                )
                .await
                .pop()
                .unwrap()
        }
        // The production shape this repair exists for: a spec synthesized
        // before the durable root was named by the runtime id, persisted
        // with the source machine as its durable state id.
        async fn poison(store: &TestDb, creation_id: &str, machine: &str) -> Vec<Value> {
            store
                .query_json(
                    "UPDATE agent_creation_requests
                         SET runtime_spec = jsonb_set(
                             runtime_spec, '{spec,durableStateId}', to_jsonb($2::text))
                         WHERE id = $1
                         RETURNING runtime_spec->'spec'->'durableStateId'",
                    &[&creation_id, &machine],
                )
                .await
        }
        assert_eq!(
            persisted_durable_state_id(&store, &creation_id).await,
            serde_json::json!(runtime_id)
        );
        assert_eq!(
            poison(&store, &creation_id, machine).await,
            vec![serde_json::json!(machine)]
        );

        // A control lease hands the Runner the runtime-id root and writes
        // the repaired spec back in the same transaction.
        let restart = store
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: email.clone(),
                workos_user_id: workos.clone(),
                project_id: project_id.clone(),
                now: None,
            })
            .await
            .unwrap();
        let restart_lease = store
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: format!("runner-{host}"),
                lease_token: "restart-lease".to_string(),
                lease_seconds: Some(60),
                source_host_id: Some(host.to_string()),
                runner_capacity: Some(capacity.clone()),
                now: None,
            })
            .await
            .unwrap()
            .expect("restart should lease");
        assert_eq!(restart_lease.request.id, restart.id);
        assert_eq!(
            runtime_spec_v1(restart_lease.runtime_spec.as_ref().unwrap()).durable_state_id,
            runtime_id,
            "the lease names the durable root by the Agent Runtime id, never the source machine"
        );
        assert_eq!(
            persisted_durable_state_id(&store, &creation_id).await,
            serde_json::json!(runtime_id),
            "the repair is persisted, not re-derived on every read"
        );
        store
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: restart.id.clone(),
                runner_id: format!("runner-{host}"),
                lease_token: "restart-lease".to_string(),
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

        // A relocation envelope is minted from the repaired spec too, and
        // the current creation row is repaired alongside it.
        assert_eq!(
            poison(&store, &creation_id, machine).await,
            vec![serde_json::json!(machine)]
        );
        let relocation = store
            .admin_request_runtime_relocate_exact(AdminRuntimeRelocateExactInput {
                admin_verified_email: "relocate-admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-relocate-admin".to_string(),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: host.to_string(),
                expected_source_machine_id: machine.to_string(),
                target_source_host_id: target_host.to_string(),
                expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                durable_state_manifest_sha256: "e".repeat(64),
                operator_observed_compute_absent: true,
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(
            runtime_spec_v1(relocation.runtime_spec.as_ref().unwrap()).durable_state_id,
            runtime_id
        );
        assert_eq!(
            persisted_durable_state_id(&store, &creation_id).await,
            serde_json::json!(runtime_id)
        );
    })
    .await;
}

#[tokio::test]
async fn postgres_failed_launch_atomically_revokes_its_provisioned_key() {
    with_isolated_postgres(|store| async move {
        let launch_code = issue_test_launch_code(&store, "2026-05-28T11:00:00Z").await;
        let created = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "failed-launch-key@finite.vip".to_string(),
                workos_user_id: "workos_failed_launch_key".to_string(),
                display_name: "Failed Launch Agent".to_string(),
                launch_code,
                idempotency_key: "failed-launch-key-submit".to_string(),
                now: Some("2026-05-28T11:01:00Z".to_string()),
            })
            .await
            .unwrap();
        let lease = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-failed-launch-key".to_string(),
                source_host_id: None,
                lease_token: "lease-failed-launch-key".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some("2026-05-28T11:02:00Z".to_string()),
            })
            .await
            .unwrap()
            .expect("failed-launch request should lease");
        assert_eq!(lease.request.id, created.request.id);
        let provisioned = store
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-failed-launch-key".to_string(),
                lease_token: "lease-failed-launch-key".to_string(),
                source_host_id: Some("failed-launch-host".to_string()),
                source_machine_id: Some("failed-launch-agent".to_string()),
                now: Some("2026-05-28T11:03:00Z".to_string()),
            })
            .await
            .unwrap();

        let failed = store
            .fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id: lease.request.id,
                runner_id: "runner-failed-launch-key".to_string(),
                lease_token: "lease-failed-launch-key".to_string(),
                failure_message: "runtime did not become ready".to_string(),
                provisioned_finite_private_api_key_id: Some(provisioned.api_key.id.clone()),
                now: Some("2026-05-28T11:04:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(failed.status, AgentCreationRequestStatus::Failed);

        let admin_state = store.finite_private_admin_state().await.unwrap();
        let key = admin_state
            .api_keys
            .iter()
            .find(|key| key.id == provisioned.api_key.id)
            .expect("provisioned key remains in metadata");
        assert_eq!(key.status, FinitePrivateApiKeyStatus::Revoked);
    })
    .await;
}

#[tokio::test]
async fn postgres_phala_capacity_reservation_counts_provider_and_core_in_flight() {
    with_isolated_postgres(|store| async move {
        let mut request_ids = Vec::new();
        for index in 0..2 {
            let launch_code = issue_confidential_test_launch_code(&store).await;
            let created = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: format!("postgres-phala-{index}@finite.vip"),
                    workos_user_id: format!("workos_postgres_phala_{index}"),
                    display_name: format!("Postgres Phala {index}"),
                    launch_code,
                    idempotency_key: format!("postgres-phala-submit-{index}"),
                    now: Some("2026-07-23T12:00:00Z".to_string()),
                })
                .await
                .unwrap();
            request_ids.push(created.request.id);
        }

        let first = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "postgres-phala-runner-a".to_string(),
                source_host_id: Some("phala-host".to_string()),
                lease_token: "postgres-phala-lease-a".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(phala_runner_capacity(0)),
                now: Some("2026-07-23T12:01:00Z".to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        assert!(request_ids.contains(&first.request.id));
        let reservation = first.in_flight_capacity_reservation.as_ref().unwrap().v1();
        assert_eq!(reservation.provider_inventory_count, 0);
        assert_eq!(reservation.core_in_flight_count, 1);
        assert_eq!(reservation.max_sandbox_count, 1);

        let second = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "postgres-phala-runner-b".to_string(),
                source_host_id: Some("phala-host".to_string()),
                lease_token: "postgres-phala-lease-b".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(phala_runner_capacity(0)),
                now: Some("2026-07-23T12:01:00Z".to_string()),
            })
            .await
            .unwrap();
        assert!(second.is_none());

        let resumed = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "postgres-phala-runner-c".to_string(),
                source_host_id: Some("phala-host".to_string()),
                lease_token: "postgres-phala-lease-c".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(phala_runner_capacity(1)),
                now: Some("2026-07-23T13:00:00Z".to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resumed.request.id, first.request.id);
        let reservation = resumed
            .in_flight_capacity_reservation
            .as_ref()
            .unwrap()
            .v1();
        assert_eq!(reservation.provider_inventory_count, 1);
        assert_eq!(reservation.core_in_flight_count, 1);
    })
    .await;
}
