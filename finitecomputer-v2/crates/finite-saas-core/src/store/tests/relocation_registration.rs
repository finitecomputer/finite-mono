use super::*;

#[tokio::test]
async fn postgres_cold_relocation_migration_reapplies_and_preserves_primary_creation_fence() {
    with_isolated_postgres(|store| async move {
        let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });

        raw.batch_execute(include_str!(
            "../../../migrations/0016_runtime_cold_relocation.sql"
        ))
        .await
        .unwrap();
        raw.batch_execute(include_str!(
            "../../../migrations/0016_runtime_cold_relocation.sql"
        ))
        .await
        .unwrap();

        let relocation_column_count: i64 = raw
            .query_one(
                "SELECT count(*)
                     FROM information_schema.columns
                     WHERE table_schema = 'public'
                       AND table_name = 'agent_creation_requests'
                       AND column_name = 'relocation_spec'",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(relocation_column_count, 1);

        let indexes = raw
            .query(
                "SELECT indexname, indexdef
                     FROM pg_indexes
                     WHERE schemaname = 'public'
                       AND tablename = 'agent_creation_requests'
                       AND indexname IN (
                         'agent_creation_requests_one_primary_creation_per_project',
                         'agent_creation_requests_one_active_relocation_per_runtime'
                       )
                     ORDER BY indexname",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(indexes.len(), 2);
        let definitions = indexes
            .iter()
            .map(|row| row.get::<_, String>("indexdef"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(definitions.contains("(project_id) WHERE (relocation_spec IS NULL)"));
        assert!(definitions.contains("(agent_runtime_id) WHERE"));
        assert!(definitions.contains("relocation_spec IS NOT NULL"));

        let old_project_unique_constraint: i64 = raw
            .query_one(
                "SELECT count(*)
                     FROM pg_constraint
                     WHERE conrelid = 'agent_creation_requests'::regclass
                       AND conname = 'agent_creation_requests_project_id_key'",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(old_project_unique_constraint, 0);

        drop(raw);
        connection.abort();
    })
    .await;
}

#[tokio::test]
async fn postgres_cold_relocation_routes_exactly_and_register_failure_keeps_source() {
    with_isolated_postgres(|store| async move {
        let run = "cold-relocate";
        let source_host = "relocate-source";
        let target_host = "relocate-target";
        let machine = "finite-kata-relocate";
        let email = format!("{run}@finite.vip");
        let workos = format!("workos-{run}");
        let launch_code = issue_test_launch_code(&store, "2026-07-25T12:00:00Z").await;

        store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: "artifact-relocate-v1".to_string(),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/agent-runtime:relocate-v1@sha256:{}",
                    "4".repeat(64)
                ),
                version_label: "relocate-v1".to_string(),
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
            .request_agent_creation_configured(
                RequestAgentCreationInput {
                    verified_email: email.clone(),
                    workos_user_id: workos.clone(),
                    display_name: "Relocation Canary".to_string(),
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
        let creation = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: format!("runner-{source_host}"),
                source_host_id: Some(source_host.to_string()),
                lease_token: "create-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        let completed = store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: creation.request.id,
                runner_id: format!("runner-{source_host}"),
                lease_token: "create-lease".to_string(),
                source_host_id: source_host.to_string(),
                source_machine_id: machine.to_string(),
                runtime_artifact_id: Some("artifact-relocate-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://127.0.0.1:4201/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Relocation Canary".to_string()),
                hostname: None,
                runtime_host: Some(source_host.to_string()),
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

        let stop = store
            .request_runtime_stop(RequestRuntimeStopInput {
                verified_email: email,
                workos_user_id: workos,
                project_id: project_id.clone(),
                now: None,
            })
            .await
            .unwrap();
        let stop_lease = store
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: format!("runner-{source_host}"),
                lease_token: "stop-lease".to_string(),
                lease_seconds: Some(300),
                source_host_id: Some(source_host.to_string()),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..RunnerLeaseCapacity::default()
                }),
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stop_lease.request.id, stop.id);
        store
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: stop.id,
                runner_id: format!("runner-{source_host}"),
                lease_token: "stop-lease".to_string(),
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

        let relocation = store
            .admin_request_runtime_relocate_exact(AdminRuntimeRelocateExactInput {
                admin_verified_email: "relocate-admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-relocate-admin".to_string(),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: source_host.to_string(),
                expected_source_machine_id: machine.to_string(),
                target_source_host_id: target_host.to_string(),
                expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                durable_state_manifest_sha256: "b".repeat(64),
                operator_observed_compute_absent: false,
                now: None,
            })
            .await
            .unwrap();
        assert!(
            store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: format!("runner-{source_host}"),
                    source_host_id: Some(source_host.to_string()),
                    lease_token: "wrong-host".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: None,
                })
                .await
                .unwrap()
                .is_none()
        );
        let relocation_lease = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: format!("runner-{target_host}"),
                source_host_id: Some(target_host.to_string()),
                lease_token: "relocation-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(relocation_lease.request.id, relocation.id);

        store
            .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                request_id: relocation.id.clone(),
                runner_id: format!("runner-{target_host}"),
                lease_token: "relocation-lease".to_string(),
                source_host_id: target_host.to_string(),
                source_machine_id: machine.to_string(),
                runtime_artifact_id: Some("artifact-relocate-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://127.0.0.1:4202/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Relocation Canary".to_string()),
                hostname: None,
                runtime_host: Some(target_host.to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Unknown),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                now: None,
            })
            .await
            .unwrap();
        store
            .fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id: relocation.id.clone(),
                runner_id: format!("runner-{target_host}"),
                lease_token: "relocation-lease".to_string(),
                failure_message: "synthetic post-register failure".to_string(),
                provisioned_finite_private_api_key_id: None,
                now: None,
            })
            .await
            .unwrap();
        let preserved = store
            .admin_runtime_overviews()
            .await
            .unwrap()
            .into_iter()
            .find(|overview| overview.agent_runtime_id == runtime_id)
            .unwrap();
        assert_eq!(preserved.source_host_id, source_host);
        assert_eq!(preserved.runtime_status, RuntimeSummaryStatus::Offline);

        // A failed attempt must not consume the Project's original
        // creation-row uniqueness or prevent an exact retry.
        let retry = store
            .admin_request_runtime_relocate_exact(AdminRuntimeRelocateExactInput {
                admin_verified_email: "relocate-admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-relocate-admin".to_string(),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: source_host.to_string(),
                expected_source_machine_id: machine.to_string(),
                target_source_host_id: target_host.to_string(),
                expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                durable_state_manifest_sha256: "b".repeat(64),
                operator_observed_compute_absent: false,
                now: None,
            })
            .await
            .unwrap();
        assert_ne!(retry.id, relocation.id);
        store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: format!("runner-{target_host}"),
                source_host_id: Some(target_host.to_string()),
                lease_token: "relocation-retry-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        store
            .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                request_id: retry.id.clone(),
                runner_id: format!("runner-{target_host}"),
                lease_token: "relocation-retry-lease".to_string(),
                source_host_id: target_host.to_string(),
                source_machine_id: machine.to_string(),
                runtime_artifact_id: Some("artifact-relocate-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://127.0.0.1:4202/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Relocation Canary".to_string()),
                hostname: None,
                runtime_host: Some(target_host.to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Unknown),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                now: None,
            })
            .await
            .unwrap();
        let completed_retry = store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: retry.id.clone(),
                runner_id: format!("runner-{target_host}"),
                lease_token: "relocation-retry-lease".to_string(),
                source_host_id: target_host.to_string(),
                source_machine_id: machine.to_string(),
                runtime_artifact_id: Some("artifact-relocate-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://127.0.0.1:4202/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Relocation Canary".to_string()),
                hostname: None,
                runtime_host: Some(target_host.to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                agent_npub: None,
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(
            completed_retry.request.status,
            AgentCreationRequestStatus::Running
        );

        let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        let runtime = raw
            .query_one(
                "SELECT source_host_id, source_machine_id, host_facts->>'runtime_status' AS status
                     FROM agent_runtimes WHERE id = $1",
                &[&runtime_id],
            )
            .await
            .unwrap();
        assert_eq!(runtime.get::<_, String>("source_host_id"), target_host);
        assert_eq!(runtime.get::<_, String>("source_machine_id"), machine);
        assert_eq!(runtime.get::<_, String>("status"), "online");
        let request = raw
            .query_one(
                "SELECT status, agent_runtime_id, relocation_spec->>'schema' AS schema
                     FROM agent_creation_requests WHERE id = $1",
                &[&relocation.id],
            )
            .await
            .unwrap();
        assert_eq!(request.get::<_, String>("status"), "failed");
        assert_eq!(
            request
                .get::<_, Option<String>>("agent_runtime_id")
                .as_deref(),
            Some(runtime_id.as_str())
        );
        assert_eq!(
            request.get::<_, Option<String>>("schema").as_deref(),
            Some(RUNTIME_RELOCATION_SCHEMA)
        );
        let retry_status: String = raw
            .query_one(
                "SELECT status FROM agent_creation_requests WHERE id = $1",
                &[&retry.id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(retry_status, "running");

        drop(raw);
        connection.abort();
    })
    .await;
}

/// Relocation is the one writer that can append a request row to a
/// project that already has one, so its single-flight contract matters
/// most here: two CONCURRENT identical operator attempts must resolve to
/// ONE row. The pre-check and the insert share the transaction, and the
/// insert's ON CONFLICT DO NOTHING defers to the partial unique index
/// `agent_creation_requests_one_active_relocation_per_runtime`, so the
/// loser re-reads the winner's committed row and reuses it instead of
/// surfacing the unique violation (or inserting a second active row).
#[tokio::test]
async fn postgres_concurrent_identical_cold_relocation_reuses_one_request() {
    with_isolated_postgres(|store| async move {
        let run = "relocate-race";
        let source_host = "relocate-race-source";
        let target_host = "relocate-race-target";
        let machine = "finite-kata-relocate-race";
        let email = format!("{run}@finite.vip");
        let workos = format!("workos-{run}");
        let launch_code = issue_test_launch_code(&store, "2026-09-14T12:00:00Z").await;

        store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: "artifact-relocate-race".to_string(),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/agent-runtime:relocate-race@sha256:{}",
                    "5".repeat(64)
                ),
                version_label: "relocate-race".to_string(),
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
            .request_agent_creation_configured(
                RequestAgentCreationInput {
                    verified_email: email.clone(),
                    workos_user_id: workos.clone(),
                    display_name: "Relocation Race Canary".to_string(),
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
        let creation = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: format!("runner-{source_host}"),
                source_host_id: Some(source_host.to_string()),
                lease_token: "create-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        let completed = store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: creation.request.id,
                runner_id: format!("runner-{source_host}"),
                lease_token: "create-lease".to_string(),
                source_host_id: source_host.to_string(),
                source_machine_id: machine.to_string(),
                runtime_artifact_id: Some("artifact-relocate-race".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://127.0.0.1:4301/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Relocation Race Canary".to_string()),
                hostname: None,
                runtime_host: Some(source_host.to_string()),
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

        let stop = store
            .request_runtime_stop(RequestRuntimeStopInput {
                verified_email: email,
                workos_user_id: workos,
                project_id: project_id.clone(),
                now: None,
            })
            .await
            .unwrap();
        let stop_lease = store
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: format!("runner-{source_host}"),
                lease_token: "stop-lease".to_string(),
                lease_seconds: Some(300),
                source_host_id: Some(source_host.to_string()),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..RunnerLeaseCapacity::default()
                }),
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stop_lease.request.id, stop.id);
        store
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: stop.id,
                runner_id: format!("runner-{source_host}"),
                lease_token: "stop-lease".to_string(),
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

        let relocate_input = || AdminRuntimeRelocateExactInput {
            admin_verified_email: "relocate-race-admin@finite.vip".to_string(),
            admin_workos_user_id: "workos-relocate-race-admin".to_string(),
            project_id: project_id.clone(),
            expected_agent_runtime_id: runtime_id.clone(),
            expected_source_host_id: source_host.to_string(),
            expected_source_machine_id: machine.to_string(),
            target_source_host_id: target_host.to_string(),
            expected_agent_npub: format!("npub1{}", "r".repeat(58)),
            durable_state_manifest_sha256: "c".repeat(64),
            operator_observed_compute_absent: false,
            now: None,
        };
        let competing_store = CoreStore::connect(&store.url).await.unwrap();
        let (first, second) = tokio::join!(
            store
                .store
                .admin_request_runtime_relocate_exact(relocate_input()),
            competing_store.admin_request_runtime_relocate_exact(relocate_input()),
        );
        let first = first.expect("first concurrent relocation must succeed");
        let second = second.expect("the loser must reuse the winner's row, not error");
        assert_eq!(first.id, second.id);
        assert_eq!(first.relocation, second.relocation);
        assert_eq!(first.target_source_host_id, second.target_source_host_id);

        let rows = store.all("agent_creation_requests").await;
        let relocations: Vec<_> = rows
            .iter()
            .filter(|row| !row["relocation_spec"].is_null())
            .collect();
        assert_eq!(
            relocations.len(),
            1,
            "the race must not insert two active relocation rows"
        );
        assert_eq!(rows.len(), 2, "original creation plus one relocation");
    })
    .await;
}
