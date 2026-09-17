use super::*;

#[tokio::test]
async fn postgres_provider_operation_ledger_replays_and_crosses_runtime_boundaries() {
    with_isolated_postgres(|store| async move {
        let launch_code = issue_test_launch_code(&store, "unused").await;
        let created = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "provider-ledger@finite.vip".to_string(),
                workos_user_id: "workos_provider_ledger".to_string(),
                display_name: "Provider Ledger".to_string(),
                launch_code,
                idempotency_key: "provider-ledger-create".to_string(),
                now: None,
            })
            .await
            .unwrap();
        let request_id = created.request.id;
        let first = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "ledger-runner-a".to_string(),
                lease_token: "ledger-token-a".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                source_host_id: None,
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        let placement = RuntimePlacement::for_hosting_tier(HostingTier::Standard);
        let input = |runner: &str,
                     token: &str,
                     correlation: &str,
                     transition: ProviderOperationTransition| {
            RecordProviderOperationTransitionInput {
                request_id: request_id.clone(),
                runner_id: runner.to_string(),
                lease_token: token.to_string(),
                correlation_id: correlation.to_string(),
                placement,
                transition,
            }
        };
        let reserved = store
            .record_provider_operation_transition(input(
                "ledger-runner-a",
                "ledger-token-a",
                "opaque-ledger-correlation",
                ProviderOperationTransition::CorrelationReserved,
            ))
            .await
            .unwrap();
        let replay = store
            .record_provider_operation_transition(input(
                "ledger-runner-a",
                "ledger-token-a",
                "opaque-ledger-correlation",
                ProviderOperationTransition::CorrelationReserved,
            ))
            .await
            .unwrap();
        assert_eq!(replay, reserved);
        let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        raw.execute(
            "UPDATE agent_creation_requests
                 SET lease_expires_at = CURRENT_TIMESTAMP - interval '1 second'
                 WHERE id = $1",
            &[&request_id],
        )
        .await
        .unwrap();
        let expired_failure = store
            .fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id: request_id.clone(),
                runner_id: "ledger-runner-a".to_string(),
                lease_token: "ledger-token-a".to_string(),
                failure_message: "stale worker failure".to_string(),
                provisioned_finite_private_api_key_id: None,
                now: None,
            })
            .await
            .unwrap_err();
        assert!(
            matches!(
                expired_failure,
                CoreError::AgentCreationRequestLeaseConflict
            ),
            "unexpected expired failure result: {expired_failure:?}"
        );
        let intact = raw
            .query_one(
                "SELECT request.status,
                            (SELECT count(*)
                             FROM agent_creation_provider_operation_transitions transition
                             WHERE transition.agent_creation_request_id = request.id)
                     FROM agent_creation_requests request WHERE request.id = $1",
                &[&request_id],
            )
            .await
            .unwrap();
        assert_eq!(intact.get::<_, String>(0), "launching");
        assert_eq!(intact.get::<_, i64>(1), 1);
        assert!(matches!(
            store
                .record_provider_operation_transition(input(
                    "ledger-runner-a",
                    "wrong-token",
                    "opaque-ledger-correlation",
                    ProviderOperationTransition::Provisioned {
                        provider_facts: json!({}),
                    },
                ))
                .await,
            Err(CoreError::AgentCreationRequestLeaseConflict)
        ));
        let second = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "ledger-runner-b".to_string(),
                lease_token: "ledger-token-b".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                source_host_id: None,
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.request.id, first.request.id);
        assert_eq!(second.provider_operation.unwrap().v1().transitions.len(), 1);
        store
            .record_provider_operation_transition(input(
                "ledger-runner-b",
                "ledger-token-b",
                "opaque-ledger-correlation",
                ProviderOperationTransition::ProvisionStarted,
            ))
            .await
            .unwrap();
        assert!(matches!(
            store
                .fail_agent_creation_request(FailAgentCreationRequestInput {
                    request_id: request_id.clone(),
                    runner_id: "ledger-runner-b".to_string(),
                    lease_token: "ledger-token-b".to_string(),
                    failure_message: "crashed after provider mutation started".to_string(),
                    provisioned_finite_private_api_key_id: None,
                    now: None,
                })
                .await,
            Err(CoreError::ProviderOperationBoundaryNotReached)
        ));
        assert!(matches!(
            store
                .cancel_agent_creation_request(CancelAgentCreationRequestInput {
                    request_id: request_id.clone(),
                    now: None,
                })
                .await,
            Err(CoreError::ProviderOperationBoundaryNotReached)
        ));
        let started = raw
            .query_one(
                "SELECT status,
                            (SELECT count(*)
                             FROM agent_creation_provider_operation_transitions transition
                             WHERE transition.agent_creation_request_id = request.id)
                     FROM agent_creation_requests request WHERE request.id = $1",
                &[&request_id],
            )
            .await
            .unwrap();
        assert_eq!(started.get::<_, String>(0), "launching");
        assert_eq!(started.get::<_, i64>(1), 2);
        store
            .record_provider_operation_transition(input(
                "ledger-runner-b",
                "ledger-token-b",
                "opaque-ledger-correlation",
                ProviderOperationTransition::Provisioned {
                    provider_facts: json!({"provider_id": "opaque-ledger-runtime"}),
                },
            ))
            .await
            .unwrap();
        let provisioned_key = store
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id: request_id.clone(),
                runner_id: "ledger-runner-b".to_string(),
                lease_token: "ledger-token-b".to_string(),
                source_host_id: Some("ledger-host".to_string()),
                source_machine_id: Some("ledger-machine".to_string()),
                now: None,
            })
            .await
            .unwrap();
        assert!(matches!(
                store
                    .fail_agent_creation_request(FailAgentCreationRequestInput {
                        request_id: request_id.clone(),
                        runner_id: "ledger-runner-b".to_string(),
                        lease_token: "ledger-token-b".to_string(),
                        failure_message: "must remain resumable".to_string(),
                        provisioned_finite_private_api_key_id: Some(
                            provisioned_key.api_key.id.clone(),
                        ),
                        now: None,
                    })
                    .await,
                Err(CoreError::ProviderOperationBoundaryNotReached)
            ));
        assert_eq!(
            store
                .finite_private_admin_state()
                .await
                .unwrap()
                .api_keys
                .into_iter()
                .find(|key| key.id == provisioned_key.api_key.id)
                .unwrap()
                .status,
            FinitePrivateApiKeyStatus::Active
        );
        assert!(matches!(
            store
                .cancel_agent_creation_request(CancelAgentCreationRequestInput {
                    request_id: request_id.clone(),
                    now: None,
                })
                .await,
            Err(CoreError::ProviderOperationBoundaryNotReached)
        ));
        assert_eq!(
            store
                .finite_private_admin_state()
                .await
                .unwrap()
                .api_keys
                .into_iter()
                .find(|key| key.id == provisioned_key.api_key.id)
                .unwrap()
                .status,
            FinitePrivateApiKeyStatus::Active
        );
        store
            .record_provider_operation_transition(input(
                "ledger-runner-b",
                "ledger-token-b",
                "opaque-ledger-correlation",
                ProviderOperationTransition::CommitStarted,
            ))
            .await
            .unwrap();

        let handle = crate::ProviderRuntimeHandleEnvelope::V1(crate::ProviderRuntimeHandleV1 {
            runner_class: crate::RunnerClass::Kata,
            opaque: json!({"sandbox_id": "opaque-ledger-runtime"}),
        });
        let completed = store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: request_id.clone(),
                runner_id: "ledger-runner-b".to_string(),
                lease_token: "ledger-token-b".to_string(),
                source_host_id: "ledger-host".to_string(),
                source_machine_id: "ledger-machine".to_string(),
                runtime_artifact_id: Some("artifact-postgres-fixture".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: Some(handle),
                contact_endpoint: None,
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: None,
                hostname: None,
                runtime_host: None,
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: None,
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                agent_npub: None,
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(
            completed.provider_operation.unwrap().v1().transitions.len(),
            6
        );
        let sequences = raw
            .query(
                "SELECT sequence FROM agent_creation_provider_operation_transitions
                     WHERE agent_creation_request_id = $1 ORDER BY sequence",
                &[&request_id],
            )
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.get::<_, i32>(0))
            .collect::<Vec<_>>();
        assert_eq!(sequences, vec![0, 1, 2, 3, 4, 5]);

        let current_code = issue_test_launch_code(&store, "unused").await;
        let current = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "provider-ledger-current@finite.vip".to_string(),
                workos_user_id: "workos_provider_ledger_current".to_string(),
                display_name: "Current Failure".to_string(),
                launch_code: current_code,
                idempotency_key: "provider-ledger-current".to_string(),
                now: None,
            })
            .await
            .unwrap();
        store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "ledger-current".to_string(),
                lease_token: "ledger-current-token".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                source_host_id: None,
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        store
            .record_provider_operation_transition(RecordProviderOperationTransitionInput {
                request_id: current.request.id.clone(),
                runner_id: "ledger-current".to_string(),
                lease_token: "ledger-current-token".to_string(),
                correlation_id: "current-failure-correlation".to_string(),
                placement,
                transition: ProviderOperationTransition::CorrelationReserved,
            })
            .await
            .unwrap();
        let abandoned_key = store
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id: current.request.id.clone(),
                runner_id: "ledger-current".to_string(),
                lease_token: "ledger-current-token".to_string(),
                source_host_id: None,
                source_machine_id: None,
                now: None,
            })
            .await
            .unwrap();
        let failed = store
            .fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id: current.request.id.clone(),
                runner_id: "ledger-current".to_string(),
                lease_token: "ledger-current-token".to_string(),
                failure_message: "failed before provider mutation".to_string(),
                provisioned_finite_private_api_key_id: None,
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(failed.status, AgentCreationRequestStatus::Failed);
        let cancelled = store
            .cancel_agent_creation_request(CancelAgentCreationRequestInput {
                request_id: current.request.id,
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(cancelled.status, AgentCreationRequestStatus::Cancelled);
        assert_eq!(
            store
                .finite_private_admin_state()
                .await
                .unwrap()
                .api_keys
                .into_iter()
                .find(|key| key.id == abandoned_key.api_key.id)
                .unwrap()
                .status,
            FinitePrivateApiKeyStatus::Revoked
        );
        drop(raw);
        connection.abort();
    })
    .await;
}
