use super::*;

#[tokio::test]
async fn postgres_migration_replay_preserves_and_repairs_pending_explicit_placement() {
    with_isolated_postgres(|store| async move {
        let launch_code = issue_test_launch_code(&store, "2026-07-31T12:00:00Z").await;
        let created = store
            .request_agent_creation_configured(
                RequestAgentCreationInput {
                    verified_email: "migration-replay@finite.vip".to_string(),
                    workos_user_id: "workos_migration_replay".to_string(),
                    display_name: "Migration Replay".to_string(),
                    launch_code,
                    idempotency_key: "migration-replay-submit".to_string(),
                    now: Some("2026-07-31T12:01:00Z".to_string()),
                },
                AgentCreationConfiguration {
                    placement: Some(RuntimePlacement {
                        runner_class: RunnerClass::AppleContainer,
                        runtime_resource_class: crate::RuntimeResourceClass::Vcpu4Memory8Gib,
                    }),
                    requested_hosting_tier: Some(HostingTier::Standard),
                    profile_picture_url: None,
                    owner_chat_account_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(created.request.runner_class, RunnerClass::AppleContainer);
        assert_eq!(
            created.request.placement.unwrap().runner_class,
            RunnerClass::AppleContainer
        );

        // Core replays the full concatenated schema on every startup. A
        // modern pending request must retain its exact placement.
        store.migrate().await.unwrap();
        let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        let runner_class: String = raw
            .query_one(
                "SELECT runner_class FROM agent_creation_requests WHERE id = $1",
                &[&created.request.id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(runner_class, "apple_container");

        // Reproduce the durable shape left by the bad replay, then prove
        // the guarded repair before exercising the real lease query.
        raw.execute(
            "UPDATE agent_creation_requests SET runner_class = 'kata' WHERE id = $1",
            &[&created.request.id],
        )
        .await
        .unwrap();
        store.migrate().await.unwrap();
        let repaired_runner_class: String = raw
            .query_one(
                "SELECT runner_class FROM agent_creation_requests WHERE id = $1",
                &[&created.request.id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(repaired_runner_class, "apple_container");

        let lease = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "devfinity-apple-runner".to_string(),
                lease_token: "migration-replay-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::AppleContainer],
                    max_sandbox_count: Some(1),
                    active_sandbox_count: Some(0),
                    ..RunnerLeaseCapacity::default()
                }),
                source_host_id: Some("devfinity-apple".to_string()),
                now: Some("2026-07-31T12:02:00Z".to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lease.request.id, created.request.id);
        assert_eq!(lease.request.runner_class, RunnerClass::AppleContainer);
        assert_eq!(
            lease.request.placement.unwrap().runner_class,
            RunnerClass::AppleContainer
        );

        drop(raw);
        connection.abort();
    })
    .await;
}

#[tokio::test]
async fn postgres_owner_chat_account_id_persists_and_lease_injects_spec_environment() {
    with_isolated_postgres(|store| async move {
        store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: "artifact-owner-npub-v1".to_string(),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/agent-runtime:owner-npub-v1@sha256:{}",
                    "5".repeat(64)
                ),
                version_label: "owner-npub-v1".to_string(),
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

        // Malformed owner chat identities are rejected before any durable
        // state is minted; an npub is not accepted because the Hermes
        // adapter allowlist and this column both speak 64-hex account ids.
        let malformed = store
            .request_agent_creation_configured(
                RequestAgentCreationInput {
                    verified_email: "owner-npub-bad@finite.vip".to_string(),
                    workos_user_id: "workos_owner_npub_bad".to_string(),
                    display_name: "Owner Npub Bad".to_string(),
                    launch_code: issue_test_launch_code(&store, "2026-08-27T12:00:00Z").await,
                    idempotency_key: "owner-npub-bad-submit".to_string(),
                    now: None,
                },
                AgentCreationConfiguration {
                    owner_chat_account_id: Some(format!("npub1{}", "q".repeat(58))),
                    ..AgentCreationConfiguration::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(malformed, CoreError::InvalidOwnerChatAccountId));

        let owner_account_id = "a".repeat(64);
        let created = store
            .request_agent_creation_configured(
                RequestAgentCreationInput {
                    verified_email: "owner-npub@finite.vip".to_string(),
                    workos_user_id: "workos_owner_npub".to_string(),
                    display_name: "Owner Npub Agent".to_string(),
                    launch_code: issue_test_launch_code(&store, "2026-08-27T12:00:01Z").await,
                    idempotency_key: "owner-npub-submit".to_string(),
                    now: None,
                },
                AgentCreationConfiguration {
                    owner_chat_account_id: Some(owner_account_id.clone()),
                    ..AgentCreationConfiguration::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            created.request.owner_chat_account_id.as_deref(),
            Some(owner_account_id.as_str())
        );
        let persisted = store
            .agent_creation_request(&created.request.id)
            .await
            .unwrap();
        assert_eq!(
            persisted.owner_chat_account_id.as_deref(),
            Some(owner_account_id.as_str())
        );

        let lease = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "owner-npub-runner".to_string(),
                lease_token: "owner-npub-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                source_host_id: None,
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        let RuntimeSpecEnvelope::V1(spec) = lease.request.runtime_spec.as_ref().unwrap();
        assert_eq!(
            spec.environment.get("FINITECHAT_OWNER_NPUBS"),
            Some(&owner_account_id)
        );

        // A request without the owner identity leases with the exact
        // legacy environment: no FINITECHAT_OWNER_NPUBS key.
        let legacy = store
            .request_agent_creation_configured(
                RequestAgentCreationInput {
                    verified_email: "owner-npub-absent@finite.vip".to_string(),
                    workos_user_id: "workos_owner_npub_absent".to_string(),
                    display_name: "Owner Npub Absent".to_string(),
                    launch_code: issue_test_launch_code(&store, "2026-08-27T12:00:02Z").await,
                    idempotency_key: "owner-npub-absent-submit".to_string(),
                    now: None,
                },
                AgentCreationConfiguration::default(),
            )
            .await
            .unwrap();
        assert_eq!(legacy.request.owner_chat_account_id, None);
        let legacy_lease = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "owner-npub-runner".to_string(),
                lease_token: "owner-npub-legacy-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                source_host_id: None,
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        let RuntimeSpecEnvelope::V1(legacy_spec) =
            legacy_lease.request.runtime_spec.as_ref().unwrap();
        assert!(
            !legacy_spec
                .environment
                .contains_key("FINITECHAT_OWNER_NPUBS")
        );
    })
    .await;
}

#[tokio::test]
async fn postgres_agent_creation_lease_partition_by_source_host() {
    with_isolated_postgres(|store| async move {
        let launch_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
        let second_launch_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
        let req_a = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "part-a@finite.vip".to_string(),
                workos_user_id: "workos_part_a".to_string(),
                display_name: "Partition Agent A".to_string(),
                launch_code: second_launch_code,
                idempotency_key: "part-a".to_string(),
                now: None,
            })
            .await
            .unwrap()
            .request
            .id;
        let req_b = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "part-b@finite.vip".to_string(),
                workos_user_id: "workos_part_b".to_string(),
                display_name: "Partition Agent B".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: "part-b".to_string(),
                now: None,
            })
            .await
            .unwrap()
            .request
            .id;

        // Route each request to a specific host (no product path sets this yet,
        // so tag directly — the lease's partition filter is what's under test).
        let (raw, conn) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        let conn = tokio::spawn(async move {
            let _ = conn.await;
        });
        raw.execute(
            "UPDATE agent_creation_requests SET target_source_host_id = 'parthosta' WHERE id = $1",
            &[&req_a],
        )
        .await
        .unwrap();
        raw.execute(
            "UPDATE agent_creation_requests SET target_source_host_id = 'parthostb' WHERE id = $1",
            &[&req_b],
        )
        .await
        .unwrap();

        // Host A's runner claims only A's request.
        let leased_a = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-a".to_string(),
                lease_token: "lease-a".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                source_host_id: Some("parthosta".to_string()),
                now: None,
            })
            .await
            .unwrap()
            .expect("host A runner should lease A's request");
        assert_eq!(leased_a.request.id, req_a);

        // A's runner has nothing else routable to it (B is host B).
        let leased_a_again = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-a".to_string(),
                lease_token: "lease-a2".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                source_host_id: Some("parthosta".to_string()),
                now: None,
            })
            .await
            .unwrap();
        assert!(leased_a_again.is_none(), "must not claim host B's request");

        // Host B's runner claims B's request.
        let leased_b = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-b".to_string(),
                lease_token: "lease-b".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                source_host_id: Some("parthostb".to_string()),
                now: None,
            })
            .await
            .unwrap()
            .expect("host B runner should lease B's request");
        assert_eq!(leased_b.request.id, req_b);

        raw.execute("SELECT 1", &[]).await.unwrap();
        drop(raw);
        conn.abort();
    })
    .await;
}
