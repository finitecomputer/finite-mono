use super::*;

#[tokio::test]
async fn creation_retry_reuses_the_persisted_complete_runtime_spec() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let launch_code = issue_test_launch_code(&db).await;
        let requested = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "retry@finite.vip".to_string(),
                workos_user_id: "user_workos_retry".to_string(),
                display_name: "Retry Agent".to_string(),
                launch_code,
                idempotency_key: "retry-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        let original_environment = BTreeMap::from([(
            "FINITE_SITES_API".to_string(),
            "https://api.finite.chat".to_string(),
        )]);
        let original_secret_references = vec![
            "FAL_KEY".to_string(),
            "FIRECRAWL_API_KEY".to_string(),
            "XAI_API_KEY".to_string(),
        ];
        let first = with_runtime_config(&db, &original_environment, &original_secret_references)
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "kata-worker-1".to_string(),
                source_host_id: None,
                lease_token: "lease-one".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        let first_spec = first.request.runtime_spec.clone().unwrap();
        let first_runtime_id = first.request.agent_runtime_id.clone().unwrap();
        let first_spec_v1 = runtime_spec_v1(&first_spec);
        assert_eq!(first_spec_v1.operation_id, requested.request.id);
        assert_eq!(first_spec_v1.agent_runtime_id, first_runtime_id);
        assert_eq!(first_spec_v1.durable_state_id, first_runtime_id);
        assert_eq!(first_spec_v1.environment, original_environment);
        assert_eq!(
            first_spec_v1.secret_references,
            vec![
                FINITE_PRIVATE_SECRET_REFERENCE.to_string(),
                "FAL_KEY".to_string(),
                "FIRECRAWL_API_KEY".to_string(),
                "XAI_API_KEY".to_string(),
            ]
        );

        promote_runtime_artifact_version(
            &db,
            "artifact-v2",
            &format!(
                "ghcr.io/finitecomputer/agent-runtime:v2@sha256:{}",
                "b".repeat(64)
            ),
            "v2",
            "db-v1",
            "2026-05-25T13:05:00Z",
        )
        .await;
        let second = with_runtime_config(
            &db,
            &BTreeMap::from([(
                "FINITE_SITES_API".to_string(),
                "https://changed.example.test".to_string(),
            )]),
            &["PERPLEXITY_API_KEY".to_string()],
        )
        .lease_agent_creation_request(LeaseAgentCreationRequestInput {
            runner_id: "kata-worker-2".to_string(),
            source_host_id: None,
            lease_token: "lease-two".to_string(),
            lease_seconds: Some(300),
            runner_capacity: Some(RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Kata],
                ..RunnerLeaseCapacity::default()
            }),
            now: Some("2026-05-25T13:06:00Z".to_string()),
        })
        .await
        .unwrap()
        .unwrap();

        assert_eq!(second.request.runtime_spec.as_ref(), Some(&first_spec));
        assert_eq!(
            second.request.desired_runtime_artifact_id.as_deref(),
            Some("artifact-v1")
        );
        assert_eq!(
            second.request.agent_runtime_id.as_deref(),
            Some(first_runtime_id.as_str())
        );
    })
    .await;
}

#[test]
fn configured_runtime_secret_references_are_bounded_unique_and_cannot_override_inference() {
    assert!(
        runtime_spec_secret_references(&["FAL_KEY".to_string(), "X_API_BEARER_TOKEN".to_string(),])
            .is_ok()
    );
    for invalid in [
        vec!["OPENAI_API_KEY".to_string()],
        vec!["FAL_KEY".to_string(), "FAL_KEY".to_string()],
        vec!["FINITE_SITES_API".to_string()],
        vec![FINITE_PRIVATE_SECRET_REFERENCE.to_string()],
    ] {
        assert!(runtime_spec_secret_references(&invalid).is_err());
    }
}

/// A launch key provisioned before a provider failure stays usable until
/// the request is finally cancelled.
///
/// Split from the ledger fencing test: failing and cancelling terminates the
/// request, so it cannot share a database with the assertions that continue
/// to drive the same request.
#[tokio::test]
async fn abandoned_launch_key_survives_failure_and_is_revoked_by_cancellation() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let launch_code = issue_test_launch_code(&db).await;
        let requested = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "abandoned@finite.vip".to_string(),
                workos_user_id: "workos-abandoned".to_string(),
                display_name: "Abandoned Agent".to_string(),
                launch_code,
                idempotency_key: "abandoned-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        let request_id = requested.request.id;
        db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
            runner_id: "runner-a".to_string(),
            lease_token: "token-a".to_string(),
            lease_seconds: Some(300),
            runner_capacity: Some(RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Kata],
                ..RunnerLeaseCapacity::default()
            }),
            source_host_id: None,
            now: Some(LATER.to_string()),
        })
        .await
        .unwrap()
        .unwrap();
        db.exec(&format!(
            "UPDATE agent_creation_requests SET lease_expires_at = '2099-01-01T00:00:00Z' \
             WHERE id = '{request_id}'"
        ))
        .await;
        let reserved = db
            .record_provider_operation_transition(RecordProviderOperationTransitionInput {
                request_id: request_id.clone(),
                runner_id: "runner-a".to_string(),
                lease_token: "token-a".to_string(),
                correlation_id: "provider-correlation-1".to_string(),
                placement: RuntimePlacement::for_hosting_tier(HostingTier::Standard),
                transition: ProviderOperationTransition::CorrelationReserved,
            })
            .await
            .unwrap();

        let abandoned_key = db
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id: request_id.clone(),
                runner_id: "runner-a".to_string(),
                lease_token: "token-a".to_string(),
                source_host_id: None,
                source_machine_id: None,
                now: Some("2098-01-01T00:00:20Z".to_string()),
            })
            .await
            .unwrap();
        let failed = db
            .fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id: request_id.clone(),
                runner_id: "runner-a".to_string(),
                lease_token: "token-a".to_string(),
                failure_message: "provider launch failed".to_string(),
                provisioned_finite_private_api_key_id: None,
                now: Some("2098-01-01T00:00:30Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(failed.status, AgentCreationRequestStatus::Failed);
        assert_eq!(
            db.finite_private_api_key(&abandoned_key.api_key.id)
                .await
                .unwrap()
                .status,
            FinitePrivateApiKeyStatus::Active,
            "failure cannot revoke a launch key the runner failed to identify"
        );
        assert_eq!(
            db.provider_operation(&request_id).await.as_ref(),
            Some(&reserved),
            "the accepted pre-provider failure keeps its audit journal"
        );

        let cancelled = db
            .cancel_agent_creation_request(CancelAgentCreationRequestInput {
                request_id: request_id.clone(),
                now: Some("2098-01-01T00:00:31Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(cancelled.status, AgentCreationRequestStatus::Cancelled);
        assert_eq!(
            db.finite_private_api_key(&abandoned_key.api_key.id)
                .await
                .unwrap()
                .status,
            FinitePrivateApiKeyStatus::Revoked,
            "final cancellation revokes an otherwise abandoned project launch key"
        );
    })
    .await;
}

#[tokio::test]
async fn self_serve_registration_launches_then_completion_marks_running() {
    with_isolated_postgres(|db| async move {
        let launch_code = issue_test_launch_code(&db).await;
        promote_runtime_artifact(&db).await;
        let requested = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        let lease = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                source_host_id: None,
                lease_token: "lease-token-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        let register_input = RegisterAgentCreationRuntimeInput {
            request_id: lease.request.id.clone(),
            runner_id: "runner-oslo-1".to_string(),
            lease_token: "lease-token-1".to_string(),
            source_host_id: "oslo-host-1".to_string(),
            source_machine_id: "oslo-agent-001".to_string(),
            runtime_artifact_id: Some("artifact-v1".to_string()),
            state_schema_version: None,
            provider_runtime_handle: Some(ProviderRuntimeHandleEnvelope::V1(
                ProviderRuntimeHandleV1 {
                    runner_class: RunnerClass::Kata,
                    opaque: json!({"container": "finite-kata-oslo-001"}),
                },
            )),
            contact_endpoint: Some("https://oslo-agent.example.com/contact/".to_string()),
            runtime_capabilities: Some(kata_runtime_capabilities()),
            display_name: None,
            hostname: None,
            runtime_host: Some("oslo-host-1".to_string()),
            runtime_status: Some(RuntimeSummaryStatus::Unknown),
            active_inference_profile: Some("finite-private".to_string()),
            hermes_available: None,
            published_app_urls: Vec::new(),
            now: Some("2026-05-25T13:01:30Z".to_string()),
        };
        let registered = db
            .register_agent_creation_runtime(register_input)
            .await
            .unwrap();

        assert_eq!(
            registered.request.status,
            AgentCreationRequestStatus::Launching
        );
        assert!(registered.request.agent_runtime_id.is_some());
        let runtime = &db
            .agent_runtime(registered.request.agent_runtime_id.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(
            runtime.contact_endpoint.as_deref(),
            Some("https://oslo-agent.example.com/contact")
        );
        assert_eq!(runtime.provider_runtime_handle_history.len(), 1);

        let completion_input = CompleteAgentCreationRequestInput {
            request_id: lease.request.id,
            runner_id: "runner-oslo-1".to_string(),
            lease_token: "lease-token-1".to_string(),
            source_host_id: "oslo-host-1".to_string(),
            source_machine_id: "oslo-agent-001".to_string(),
            runtime_artifact_id: Some("artifact-v1".to_string()),
            state_schema_version: None,
            provider_runtime_handle: Some(ProviderRuntimeHandleEnvelope::V1(
                ProviderRuntimeHandleV1 {
                    runner_class: RunnerClass::Kata,
                    opaque: json!({"container": "finite-kata-oslo-001"}),
                },
            )),
            contact_endpoint: Some("https://oslo-agent.example.com/contact".to_string()),
            runtime_capabilities: Some(kata_runtime_capabilities()),
            display_name: None,
            hostname: None,
            runtime_host: Some("oslo-host-1".to_string()),
            runtime_status: Some(RuntimeSummaryStatus::Online),
            active_inference_profile: Some("finite-private".to_string()),
            hermes_available: Some(true),
            published_app_urls: Vec::new(),
            agent_npub: None,
            now: Some("2026-05-25T13:02:00Z".to_string()),
        };
        let mut mismatched_completion = completion_input.clone();
        mismatched_completion.runtime_capabilities =
            Some(RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                runtime_upgrade: false,
                ..*kata_runtime_capabilities().v1()
            }));
        assert!(matches!(
            db.complete_agent_creation_request(mismatched_completion)
                .await,
            Err(CoreError::RuntimeCapabilitiesMismatch)
        ));
        let completed = db
            .complete_agent_creation_request(completion_input)
            .await
            .unwrap();

        assert_eq!(
            completed.request.status,
            AgentCreationRequestStatus::Running
        );
        assert_eq!(completed.project.id, requested.project.id);
    })
    .await;
}

#[tokio::test]
async fn runner_can_mark_agent_creation_request_failed_without_runtime() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let launch_code = issue_test_launch_code(&db).await;
        let requested = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
            runner_id: "runner-oslo-1".to_string(),
            source_host_id: None,
            lease_token: "lease-token-1".to_string(),
            lease_seconds: Some(300),
            runner_capacity: None,
            now: Some(LATER.to_string()),
        })
        .await
        .unwrap();

        let failed = db
            .fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id: requested.request.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                failure_message: "runner capacity unavailable".to_string(),
                provisioned_finite_private_api_key_id: None,
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(failed.status, AgentCreationRequestStatus::Failed);
        assert_eq!(
            failed.failure_message.as_deref(),
            Some("runner capacity unavailable")
        );
        assert!(failed.agent_runtime_id.is_none());
        assert!(db.all_agent_runtimes().await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn failed_self_serve_launch_removes_provisional_runtime() {
    with_isolated_postgres(|db| async move {
        let launch_code = issue_test_launch_code(&db).await;
        promote_runtime_artifact(&db).await;
        let requested = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        let lease = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                source_host_id: None,
                lease_token: "lease-token-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        db.register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
            request_id: lease.request.id.clone(),
            runner_id: "runner-oslo-1".to_string(),
            lease_token: "lease-token-1".to_string(),
            source_host_id: "oslo-host-1".to_string(),
            source_machine_id: "oslo-agent-001".to_string(),
            runtime_artifact_id: Some("artifact-v1".to_string()),
            state_schema_version: None,
            provider_runtime_handle: None,
            contact_endpoint: None,
            runtime_capabilities: Some(kata_runtime_capabilities()),
            display_name: None,
            hostname: None,
            runtime_host: Some("oslo-host-1".to_string()),
            runtime_status: Some(RuntimeSummaryStatus::Unknown),
            active_inference_profile: None,
            hermes_available: None,
            published_app_urls: Vec::new(),
            now: Some("2026-05-25T13:01:30Z".to_string()),
        })
        .await
        .unwrap();

        assert_eq!(db.table_len("agent_runtimes").await, 1);
        assert_eq!(db.table_len("project_runtime_links").await, 1);

        let failed = db
            .fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id: requested.request.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                failure_message: "runtime did not publish a relay heartbeat".to_string(),
                provisioned_finite_private_api_key_id: None,
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(failed.status, AgentCreationRequestStatus::Failed);
        assert!(failed.agent_runtime_id.is_none());
        assert!(db.all_agent_runtimes().await.is_empty());
        assert!(db.all("project_runtime_links").await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn finite_private_runtime_key_provisioning_is_bound_to_launching_request() {
    with_isolated_postgres(|db| async move {
        let launch_code = issue_test_launch_code(&db).await;
        promote_runtime_artifact(&db).await;
        let requested = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        let lease = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                source_host_id: None,
                lease_token: "lease-token-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap()
            .expect("request should be leased");

        let provisioned = db
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                source_host_id: Some("oslo-host-1".to_string()),
                source_machine_id: Some("finite-agent_123".to_string()),
                now: Some("2026-05-25T13:01:00Z".to_string()),
            })
            .await
            .unwrap();

        assert!(provisioned.raw_api_key.starts_with("fpk_live_"));
        assert_eq!(provisioned.grant.status, FinitePrivateGrantStatus::Active);
        assert_eq!(
            provisioned.api_key.project_id.as_deref(),
            Some(requested.project.id.as_str())
        );
        assert!(provisioned.api_key.agent_runtime_id.is_none());
        assert!(
            !serde_json::to_string(&db.all("finite_private_api_keys").await)
                .unwrap()
                .contains(&provisioned.raw_api_key)
        );

        let wrong_lease = db
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "wrong-token".to_string(),
                source_host_id: Some("oslo-host-1".to_string()),
                source_machine_id: Some("finite-agent_123".to_string()),
                now: Some("2026-05-25T13:01:00Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            wrong_lease,
            CoreError::AgentCreationRequestLeaseConflict
        ));

        let unrelated_key = db
            .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: provisioned.grant.id.clone(),
                raw_key: "fpk_live_unrelated_project_key".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: Some("2026-05-25T13:01:30Z".to_string()),
            })
            .await
            .unwrap();
        let mismatched = db
            .fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                failure_message: "runtime failed".to_string(),
                provisioned_finite_private_api_key_id: Some(unrelated_key.id.clone()),
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(mismatched, CoreError::InvalidFinitePrivateApiKey));
        assert_eq!(
            db.agent_creation_request(&lease.request.id)
                .await
                .unwrap()
                .status,
            AgentCreationRequestStatus::Launching
        );
        assert_eq!(
            db.finite_private_api_key(&unrelated_key.id)
                .await
                .unwrap()
                .status,
            FinitePrivateApiKeyStatus::Active
        );

        let failed = db
            .fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id: lease.request.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                failure_message: "runtime failed".to_string(),
                provisioned_finite_private_api_key_id: Some(provisioned.api_key.id.clone()),
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(failed.status, AgentCreationRequestStatus::Failed);
        assert_eq!(
            db.finite_private_api_key(&provisioned.api_key.id)
                .await
                .unwrap()
                .status,
            FinitePrivateApiKeyStatus::Revoked
        );
        assert_eq!(
            db.finite_private_api_key(&unrelated_key.id)
                .await
                .unwrap()
                .status,
            FinitePrivateApiKeyStatus::Active
        );
    })
    .await;
}
