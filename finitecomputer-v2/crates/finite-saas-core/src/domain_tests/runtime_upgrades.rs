use super::*;

#[tokio::test]
async fn explicit_kata_upgrade_binds_compatible_artifact_and_commits_actual_facts_atomically() {
    with_isolated_postgres(|db| async move {
        let launch_code = issue_test_launch_code(&db).await;
        promote_runtime_artifact(&db).await;
        let requested = db
            .request_agent_creation_configured(
                RequestAgentCreationInput {
                    verified_email: "upgrade@finite.vip".to_string(),
                    workos_user_id: "workos-upgrade".to_string(),
                    display_name: "Upgrade Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "upgrade-agent".to_string(),
                    now: Some(NOW.to_string()),
                },
                AgentCreationConfiguration {
                    placement: Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
                    requested_hosting_tier: None,
                    profile_picture_url: None,
                    owner_chat_account_id: None,
                },
            ).await
            .unwrap();
        db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "kata-runner".to_string(),
                source_host_id: None,
                lease_token: "launch-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some(LATER.to_string()),
            }).await
            .unwrap()
            .unwrap();
        let completed = db
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: requested.request.id,
                runner_id: "kata-runner".to_string(),
                lease_token: "launch-lease".to_string(),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "finite-kata-upgrade".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
                provider_runtime_handle: None,
                contact_endpoint: Some("http://127.0.0.1:41001/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: None,
                hostname: None,
                runtime_host: Some("http://127.0.0.1:41001".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: vec!["http://127.0.0.1:41001/contact".to_string()],
                agent_npub: None,
                now: Some("2026-05-25T13:02:00Z".to_string()),
            }).await
            .unwrap();
        let runtime_id = completed.request.agent_runtime_id.unwrap();
        db.exec(&format!(
            "INSERT INTO runtime_relay_credentials \
             (agent_runtime_id, token_hash, created_at, updated_at) \
             VALUES ('{runtime_id}', 'existing-relay-token-hash', \
             '2026-05-25T13:02:00Z', '2026-05-25T13:02:00Z')"
        ))
        .await;
        promote_runtime_artifact_version(
            &db,
            "artifact-mutable",
            "ghcr.io/finitecomputer/agent-runtime:latest",
            "mutable",
            "db-v1",
            "2026-05-25T13:02:10Z",
        ).await;
        let mutable = db
            .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-admin".to_string(),
                project_id: requested.project.id.clone(),
                target_runtime_artifact_id: "artifact-mutable".to_string(),
                now: Some("2026-05-25T13:02:20Z".to_string()),
            }).await
            .unwrap_err();
        assert!(matches!(mutable, CoreError::RuntimeUpgradeUnsupported));
        promote_runtime_artifact_version(
            &db,
            "artifact-incompatible",
            &format!(
                "ghcr.io/finitecomputer/agent-runtime:future@sha256:{}",
                "c".repeat(64)
            ),
            "future",
            "db-v2",
            "2026-05-25T13:02:30Z",
        ).await;
        let incompatible = db
            .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-admin".to_string(),
                project_id: requested.project.id.clone(),
                target_runtime_artifact_id: "artifact-incompatible".to_string(),
                now: Some("2026-05-25T13:02:40Z".to_string()),
            }).await
            .unwrap_err();
        assert!(matches!(
            incompatible,
            CoreError::RuntimeUpgradeStateSchemaIncompatible
        ));
        promote_runtime_artifact_version(
            &db,
            "artifact-v2",
            &format!(
                "ghcr.io/finitecomputer/agent-runtime:v2@sha256:{}",
                "b".repeat(64)
            ),
            "v2",
            "db-v1",
            "2026-05-25T13:03:00Z",
        ).await;
        db.exec("UPDATE runtime_artifacts SET recover_known_good_chat = true WHERE id = 'artifact-v2'")
            .await;

        let changed_binding = db
            .admin_request_runtime_upgrade_exact(AdminRuntimeUpgradeExactInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-admin".to_string(),
                project_id: requested.project.id.clone(),
                expected_agent_runtime_id: "runtime-replaced-after-plan".to_string(),
                expected_source_host_id: "oslo-host-1".to_string(),
                expected_source_machine_id: "finite-kata-upgrade".to_string(),
                target_runtime_artifact_id: "artifact-v2".to_string(),
                now: Some("2026-05-25T13:03:30Z".to_string()),
            }).await
            .unwrap_err();
        assert!(matches!(changed_binding, CoreError::RuntimeSpecMismatch));
        assert!(db.all_runtime_control_requests().await.is_empty());

        let upgrade = db
            .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-admin".to_string(),
                project_id: requested.project.id.clone(),
                target_runtime_artifact_id: "artifact-v2".to_string(),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            }).await
            .unwrap();
        assert_eq!(upgrade.kind, RuntimeControlKind::Upgrade);
        assert_eq!(
            upgrade.target_runtime_artifact_id.as_deref(),
            Some("artifact-v2")
        );
        let conflicting_stop = db
            .request_runtime_stop(RequestRuntimeStopInput {
                verified_email: "upgrade@finite.vip".to_string(),
                workos_user_id: "workos-upgrade".to_string(),
                project_id: requested.project.id.clone(),
                now: Some("2026-05-25T13:04:30Z".to_string()),
            }).await
            .unwrap_err();
        assert!(matches!(
            conflicting_stop,
            CoreError::RuntimeControlOperationConflict
        ));
        db.exec("UPDATE runtime_artifacts SET retired_at = '2026-05-25T13:04:40Z' WHERE id = 'artifact-v2'")
            .await;
        // A second, healthy Runtime on the same Project, copied from the
        // first so it shares its artifact and capabilities.
        let healthy_runtime_id = "runtime-healthy-behind-poison".to_string();
        db.exec(&format!(
            "INSERT INTO agent_runtimes \
             (id, project_id, source_host_id, source_machine_id, source_import_key, \
              runtime_artifact_id, state_schema_version, host_facts, \
              created_at, updated_at, placement_runner_class, runtime_resource_class, \
              runtime_capabilities) \
             SELECT '{healthy_runtime_id}', project_id, source_host_id, \
                    'healthy-behind-poison', \
                    source_host_id || '/healthy-behind-poison', \
                    runtime_artifact_id, state_schema_version, host_facts, \
                    created_at, updated_at, placement_runner_class, \
                    runtime_resource_class, runtime_capabilities \
             FROM agent_runtimes WHERE id = '{runtime_id}'"
        ))
        .await;
        let user_id = db
            .user_by_email("upgrade@finite.vip")
            .await
            .expect("owner exists")
            .id;
        let healthy_request_id = "runtime_ctl_healthy_behind_poison".to_string();
        let healthy_project_id = requested.project.id.clone();
        db.exec(&format!(
            "INSERT INTO runtime_control_requests \
             (id, project_id, agent_runtime_id, source_host_id, source_machine_id, \
              requested_by_user_id, kind, status, created_at, updated_at) \
             VALUES ('{healthy_request_id}', '{healthy_project_id}', \
             '{healthy_runtime_id}', 'oslo-host-1', 'healthy-behind-poison', \
             '{user_id}', 'restart', 'requested', \
             '2026-05-25T13:04:45Z', '2026-05-25T13:04:45Z')"
        ))
        .await;
        let healthy_lease = db
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "kata-runner".to_string(),
                lease_token: "must-not-stick".to_string(),
                lease_seconds: Some(300),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some("2026-05-25T13:04:50Z".to_string()),
            }).await
            .unwrap()
            .expect("poisoned upgrade must not starve the next healthy request");
        assert_eq!(healthy_lease.request.id, healthy_request_id);
        assert_eq!(
            db.runtime_control_request(&upgrade.id).await.unwrap().status,
            RuntimeControlRequestStatus::Failed
        );
        assert!(
            db.runtime_control_request(&upgrade.id).await.unwrap()
                .failure_message
                .as_deref()
                .unwrap_or_default()
                .contains("retired")
        );
        db.exec("UPDATE runtime_artifacts SET retired_at = NULL WHERE id = 'artifact-v2'")
            .await;
        // An N-1 request that predates persisted runtime specs.
        db.exec(&format!(
            "UPDATE agent_creation_requests SET runtime_spec = NULL \
             WHERE agent_runtime_id = '{runtime_id}'"
        ))
        .await;
        let upgrade = db
            .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-admin".to_string(),
                project_id: requested.project.id.clone(),
                target_runtime_artifact_id: "artifact-v2".to_string(),
                now: Some("2026-05-25T13:04:55Z".to_string()),
            }).await
            .unwrap();
        let refreshed_secret_references = vec!["FAL_KEY".to_string(), "XAI_API_KEY".to_string()];
        let lease = with_runtime_config(&db, &BTreeMap::new(), &refreshed_secret_references).lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: "kata-runner".to_string(),
                    lease_token: "upgrade-lease".to_string(),
                    lease_seconds: Some(300),
                    source_host_id: Some("oslo-host-1".to_string()),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: Some("2026-05-25T13:05:00Z".to_string()),
                }).await
            .unwrap()
            .unwrap();
        assert_eq!(
            lease
                .target_runtime_artifact
                .as_ref()
                .map(|artifact| artifact.id.as_str()),
            Some("artifact-v2")
        );
        let synthesized_upgrade_spec = lease.runtime_spec.as_ref().unwrap();
        // The Runner half of this pin is finite-saas-runner's
        // `legacy_runtime_migrates_by_container_name_and_the_probe_agrees`
        // (same machine id, same `agent_runtime_id != source_machine_id`
        // shape): the machine-named directory is discovered by container
        // name and renamed to the runtime-id root, never planned from
        // the spec.
        assert_ne!(runtime_id, "finite-kata-upgrade");
        assert_eq!(
            runtime_spec_v1(synthesized_upgrade_spec).durable_state_id,
            runtime_id,
            "legacy synthesis names the durable root by the Agent Runtime id, never the source machine"
        );
        assert_eq!(
            runtime_spec_v1(synthesized_upgrade_spec).operation_id,
            upgrade.id
        );
        assert_eq!(
            runtime_spec_v1(synthesized_upgrade_spec).secret_references,
            vec!["FINITE_PRIVATE_API_KEY", "FAL_KEY", "XAI_API_KEY"]
        );

        let mismatch = db
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: upgrade.id.clone(),
                runner_id: "kata-runner".to_string(),
                lease_token: "upgrade-lease".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: Some("db-v1".to_string()),
                runtime_capabilities: None,
                runtime_host: Some("http://127.0.0.1:41002".to_string()),
                published_app_urls: Some(vec!["http://127.0.0.1:41002/contact".to_string()]),
                retirement_snapshot: None,
                now: Some("2026-05-25T13:06:00Z".to_string()),
            }).await
            .unwrap_err();
        assert!(matches!(
            mismatch,
            CoreError::RuntimeUpgradeCompletionMismatch
        ));
        assert_eq!(
            db.runtime_control_request(&upgrade.id).await.unwrap().status,
            RuntimeControlRequestStatus::Launching
        );
        assert_eq!(
            db.agent_runtime(&runtime_id).await.unwrap()
                .runtime_artifact_id
                .as_deref(),
            Some("artifact-v1")
        );

        db.exec("UPDATE runtime_artifacts SET retired_at = '2026-05-25T13:06:30Z' WHERE id = 'artifact-v2'")
            .await;
        with_runtime_config(&db, &BTreeMap::new(), &refreshed_secret_references).complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: upgrade.id.clone(),
                    runner_id: "kata-runner".to_string(),
                    lease_token: "upgrade-lease".to_string(),
                    runtime_artifact_id: Some("artifact-v2".to_string()),
                    state_schema_version: Some("db-v1".to_string()),
                    runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                        RuntimeCapabilitiesV1 {
                            recover_known_good_chat: true,
                            ..*kata_runtime_capabilities().v1()
                        },
                    )),
                    runtime_host: Some("http://127.0.0.1:41002".to_string()),
                    published_app_urls: Some(vec!["http://127.0.0.1:41002/contact".to_string()]),
                    retirement_snapshot: None,
                    now: Some("2026-05-25T13:06:40Z".to_string()),
                }).await
            .unwrap();
        let runtime = &db.agent_runtime(&runtime_id).await.unwrap();
        assert_eq!(runtime.runtime_artifact_id.as_deref(), Some("artifact-v2"));
        assert_eq!(runtime.source_machine_id, "finite-kata-upgrade");
        assert_eq!(
            runtime.contact_endpoint.as_deref(),
            Some("http://127.0.0.1:41002/contact")
        );
        assert_eq!(runtime.host_facts.runtime_host, "http://127.0.0.1:41002");
        assert!(
            runtime
                .runtime_capabilities
                .as_ref()
                .unwrap()
                .v1()
                .recover_known_good_chat
        );
        assert!(!db.query_json(
            "SELECT to_jsonb(t) FROM runtime_relay_credentials t \
             WHERE t.agent_runtime_id = $1",
            &[&runtime_id],
        )
        .await
        .is_empty());
        let requests = db.all_agent_creation_requests().await;
        let persisted_spec = requests
            .iter()
            .find(|request| request.agent_runtime_id.as_deref() == Some(runtime_id.as_str()))
            .and_then(|request| request.runtime_spec.as_ref())
            .unwrap();
        assert_eq!(
            runtime_spec_v1(persisted_spec).secret_references,
            vec!["FINITE_PRIVATE_API_KEY", "FAL_KEY", "XAI_API_KEY"]
        );
        assert!(
            db.all("project_runtime_links").await.iter()
                .any(|link| { link["agent_runtime_id"] == runtime_id.as_str() && link["active"] == true })
        );
        assert!(db.all_finite_private_api_keys().await.iter().all(|key| {
            key.agent_runtime_id.as_deref() != Some(runtime_id.as_str())
                || key.status == FinitePrivateApiKeyStatus::Active
        }));
        assert!(
            db.finite_private_admin_audit_events().await.unwrap().iter()
                .any(|event| {
                    event.action == "runtime.admin_upgrade"
                        && event.metadata["targetRuntimeArtifactId"] == "artifact-v2"
                })
        );

        let recovery = db
            .request_runtime_recover_known_good_chat(RequestRuntimeRecoverKnownGoodChatInput {
                verified_email: "upgrade@finite.vip".to_string(),
                workos_user_id: "workos-upgrade".to_string(),
                project_id: requested.project.id,
                now: Some("2026-05-25T13:07:00Z".to_string()),
            }).await
            .unwrap();
        let recovery_capabilities = RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
            recover_known_good_chat: true,
            ..*kata_runtime_capabilities().v1()
        });
        let recovery_lease = db
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "kata-runner".to_string(),
                lease_token: "recovery-lease".to_string(),
                lease_seconds: Some(300),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(recovery_capabilities),
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some("2026-05-25T13:07:01Z".to_string()),
            }).await
            .unwrap()
            .unwrap();
        assert_eq!(recovery_lease.request.id, recovery.id);
        let recovery_spec = runtime_spec_v1(recovery_lease.runtime_spec.as_ref().unwrap());
        assert_eq!(
            recovery_spec.boot_intent,
            RuntimeBootIntent::RecoverKnownGood
        );
        assert_eq!(recovery_spec.runtime_artifact_id, "artifact-v2");
    })
    .await;
}

#[tokio::test]
async fn runtime_upgrade_rejects_non_kata_runtime_before_leasing() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let runtime_id = complete_self_serve_agent(
            &db,
            "not-kata@finite.vip",
            "workos-not-kata",
            "not-kata",
            "not-kata-runtime",
            "artifact-v1",
            LATER,
        )
        .await;
        promote_runtime_artifact_version(
            &db,
            "artifact-mutable",
            "ghcr.io/finitecomputer/agent-runtime:latest",
            "mutable",
            "db-v1",
            "2026-05-25T13:03:00Z",
        )
        .await;
        let project_id = db
            .agent_runtime(&runtime_id)
            .await
            .unwrap()
            .project_id
            .clone();
        let error = db
            .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-admin".to_string(),
                project_id,
                target_runtime_artifact_id: "artifact-mutable".to_string(),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(error, CoreError::RuntimeUpgradeUnsupported));
        assert!(db.all_runtime_control_requests().await.is_empty());
    })
    .await;
}
