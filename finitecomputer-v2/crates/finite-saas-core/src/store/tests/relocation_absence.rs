use super::*;

#[tokio::test]
async fn postgres_cold_relocation_accepts_online_source_only_under_absence_attestation() {
    with_isolated_postgres(|store| async move {
        let run = "cold-relocate-online";
        let source_host = "relocate-online-source";
        let target_host = "relocate-online-target";
        let machine = "finite-kata-relocate-online";
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
                    display_name: "Online Relocation Canary".to_string(),
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
                runner_id: format!("runner-{source_host}"),
                source_host_id: Some(source_host.to_string()),
                lease_token: "create-lease".to_string(),
                lease_seconds: Some(300),
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
        let completed = store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: created.request.id,
                runner_id: format!("runner-{source_host}"),
                lease_token: "create-lease".to_string(),
                source_host_id: source_host.to_string(),
                source_machine_id: machine.to_string(),
                runtime_artifact_id: Some("artifact-relocate-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://127.0.0.1:4203/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Online Relocation Canary".to_string()),
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

        // Without the operator's compute-absent attestation an online
        // source is NOT frozen: it may still be running, so the exact
        // relocation must refuse.
        let unattested = store
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
            .await;
        assert!(matches!(
            unattested,
            Err(CoreError::RuntimeControlUnsupported)
        ));

        // Under the attestation the pre-death `online` report is exactly
        // as frozen as `stale`: the dead host's runner can neither lease
        // a control nor file a fresh report, so nothing could have moved
        // the runtime since the host died.
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
                operator_observed_compute_absent: true,
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(relocation.status, AgentCreationRequestStatus::Requested);
    })
    .await;
}

#[tokio::test]
async fn postgres_same_host_relocation_recreates_compute_only_under_absence_attestation() {
    with_isolated_postgres(|store| async move {
            let run = "cold-relocate-same-host";
            let host = "relocate-same-host";
            let machine = "finite-kata-relocate-same-host";
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
                        display_name: "Same-Host Relocation Canary".to_string(),
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
            // A cleanly stopped runtime: the strongest precondition a
            // cross-host relocation accepts. Same-host still needs more.
            let completed = store
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: created.request.id,
                    runner_id: format!("runner-{host}"),
                    lease_token: "create-lease".to_string(),
                    source_host_id: host.to_string(),
                    source_machine_id: machine.to_string(),
                    runtime_artifact_id: Some("artifact-relocate-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: Some("http://127.0.0.1:4204/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("Same-Host Relocation Canary".to_string()),
                    hostname: None,
                    runtime_host: Some(host.to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Offline),
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
            // Existing-state fixture: an activated credential from the original
            // launch. The public relocation flow below must revoke it even
            // though host, machine, runtime ID, and Project link stay equal.
            let mut credential_bytes = [0u8; 32];
            getrandom::getrandom(&mut credential_bytes).unwrap();
            let old_secret: String = credential_bytes.iter().map(|b| format!("{b:02x}")).collect();
            store.connection().await.unwrap().execute(
                "INSERT INTO runtime_core_credentials (agent_runtime_id,creation_request_id,source_host_id,source_machine_id,token_sha256,lease_sha256,activated,bootstrap_secret,owner_user_id)
                 SELECT $1,$2,$3,$4,$5,$6,TRUE,$7,p.owner_user_id FROM projects p JOIN agent_runtimes r ON r.project_id=p.id WHERE r.id=$1",
                &[&runtime_id, &completed.request.id, &host, &machine, &runtime_credentials::digest(&old_secret), &runtime_credentials::digest("create-lease"), &old_secret],
            ).await.unwrap();
            assert!(store.authenticate_runtime_credential(&old_secret).await.unwrap().is_some());
            let relocate_input = |absent: bool| AdminRuntimeRelocateExactInput {
                admin_verified_email: "relocate-admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-relocate-admin".to_string(),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: host.to_string(),
                expected_source_machine_id: machine.to_string(),
                target_source_host_id: host.to_string(),
                expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                durable_state_manifest_sha256: "d".repeat(64),
                operator_observed_compute_absent: absent,
                now: None,
            };

            // Source and target host coincide: without the compute-absent
            // attestation that is a restart under another name, refused
            // exactly as before, even for a cleanly stopped source.
            let unattested = store
                .admin_request_runtime_relocate_exact(relocate_input(false))
                .await;
            assert!(matches!(unattested, Err(CoreError::RuntimeSpecMismatch)));

            // Under the attestation the container is gone and a restart has
            // nothing to restart; the same-host relocation is the lane that
            // recreates compute against the durable tree where it lives.
            let relocation = store
                .admin_request_runtime_relocate_exact(relocate_input(true))
                .await
                .unwrap();
            assert_eq!(relocation.status, AgentCreationRequestStatus::Requested);
            assert_eq!(relocation.target_source_host_id.as_deref(), Some(host));
            let envelope = relocation.relocation.as_ref().unwrap().v1();
            assert_eq!(envelope.source_host_id, host);
            assert_eq!(envelope.target_source_host_id, host);
            assert_eq!(envelope.source_machine_id, machine);
            assert!(envelope.source_compute_absent);
            assert_eq!(
                runtime_spec_v1(relocation.runtime_spec.as_ref().unwrap()).durable_state_id,
                runtime_id,
                "the recreated compute mounts the runtime-id durable root"
            );

            // The same host's Runner leases it, registers (non-mutating) and
            // completes under the same machine name: the runtime keeps its
            // binding and comes back online with no second runtime row.
            let lease = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: format!("runner-{host}"),
                    source_host_id: Some(host.to_string()),
                    lease_token: "relocate-lease".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(capacity),
                    now: None,
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(lease.request.id, relocation.id);
            store
                .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                    request_id: relocation.id.clone(),
                    runner_id: format!("runner-{host}"),
                    lease_token: "relocate-lease".to_string(),
                    source_host_id: host.to_string(),
                    source_machine_id: machine.to_string(),
                    runtime_artifact_id: Some("artifact-relocate-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: Some("http://127.0.0.1:4204/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: None,
                    hostname: None,
                    runtime_host: Some(host.to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Unknown),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: None,
                })
                .await
                .unwrap();
            let registered = store.agent_runtime(&runtime_id).await.unwrap();
            assert_eq!(
                registered.host_facts.runtime_status,
                RuntimeSummaryStatus::Offline
            );
            let completed = store
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: relocation.id.clone(),
                    runner_id: format!("runner-{host}"),
                    lease_token: "relocate-lease".to_string(),
                    source_host_id: host.to_string(),
                    source_machine_id: machine.to_string(),
                    runtime_artifact_id: Some("artifact-relocate-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: Some("http://127.0.0.1:4204/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: None,
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
            assert!(store.authenticate_runtime_credential(&old_secret).await.unwrap().is_none());
            let credential = store.query_json("SELECT to_jsonb(c) FROM runtime_core_credentials c WHERE agent_runtime_id=$1", &[&runtime_id]).await;
            assert_eq!(credential[0]["revoked"], true);
            // The relocated incarnation latches `online` with no standing
            // report of its own (the old host's last report must not project
            // as its status) and its attribution pin is the relocation's
            // expected principal, so the first report must present it.
            let relocated = store
                .admin_runtime_overviews()
                .await
                .unwrap()
                .into_iter()
                .find(|overview| overview.agent_runtime_id == runtime_id)
                .unwrap();
            assert_eq!(relocated.lifecycle_status, RuntimeSummaryStatus::Online);
            assert_eq!(relocated.runtime_status, RuntimeSummaryStatus::Unknown);
            assert_eq!(
                relocated.runtime_health,
                crate::RuntimeHealthProjection {
                    agent_npub: Some(format!("npub1{}", "q".repeat(58))),
                    ..crate::RuntimeHealthProjection::unreported()
                },
                "no report survives the relocation; the pin is the expected principal"
            );
            let relocated_target = store
                .runtime_health_targets_for_host(host)
                .await
                .unwrap()
                .targets
                .into_iter()
                .find(|target| target.agent_runtime_id == runtime_id)
                .unwrap();
            assert_eq!(
                relocated_target.agent_npub.as_deref(),
                Some(format!("npub1{}", "q".repeat(58)).as_str())
            );
            assert_eq!(
                completed.request.status,
                AgentCreationRequestStatus::Running
            );
            assert_eq!(
                completed.request.agent_runtime_id.as_deref(),
                Some(runtime_id.as_str())
            );
            let relocated = store.agent_runtime(&runtime_id).await.unwrap();
            assert_eq!(relocated.source_host_id, host);
            assert_eq!(relocated.source_machine_id, machine);
            assert_eq!(
                relocated.host_facts.runtime_status,
                RuntimeSummaryStatus::Online
            );
            let runtime_rows = store
                .query_json(
                    "SELECT to_jsonb(id) FROM agent_runtimes WHERE project_id = $1",
                    &[&project_id],
                )
                .await;
            assert_eq!(runtime_rows, vec![serde_json::json!(runtime_id)]);
        })
        .await;
}
