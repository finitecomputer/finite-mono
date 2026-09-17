use super::*;

#[tokio::test]
async fn provider_operation_ledger_is_fenced_monotonic_and_survives_re_lease() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let launch_code = issue_test_launch_code(&db).await;
        let requested = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "ledger@finite.vip".to_string(),
                workos_user_id: "workos-ledger".to_string(),
                display_name: "Ledger Agent".to_string(),
                launch_code,
                idempotency_key: "ledger-submit".to_string(),
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
        let fail_input =
            |runner: &str, token: &str, key_id: Option<String>| FailAgentCreationRequestInput {
                request_id: request_id.clone(),
                runner_id: runner.to_string(),
                lease_token: token.to_string(),
                failure_message: "provider launch failed".to_string(),
                provisioned_finite_private_api_key_id: key_id,
                now: Some("2098-01-01T00:00:30Z".to_string()),
            };

        let reserved = db
            .record_provider_operation_transition(input(
                "runner-a",
                "token-a",
                "provider-correlation-1",
                ProviderOperationTransition::CorrelationReserved,
            ))
            .await
            .unwrap();
        let replay = db
            .record_provider_operation_transition(input(
                "runner-a",
                "token-a",
                "provider-correlation-1",
                ProviderOperationTransition::CorrelationReserved,
            ))
            .await
            .unwrap();
        assert_eq!(replay, reserved, "replay returns the exact persisted ack");
        db.exec(&format!(
            "UPDATE agent_creation_requests SET lease_expires_at = '2020-01-01T00:00:00Z' \
             WHERE id = '{request_id}'"
        ))
        .await;
        assert!(matches!(
            db.fail_agent_creation_request(fail_input("runner-a", "token-a", None))
                .await,
            Err(CoreError::AgentCreationRequestLeaseConflict)
        ));
        assert_eq!(
            db.provider_operation(&request_id).await.as_ref(),
            Some(&reserved)
        );
        assert_eq!(
            db.agent_creation_request(&request_id).await.unwrap().status,
            AgentCreationRequestStatus::Launching
        );
        db.exec(&format!(
            "UPDATE agent_creation_requests SET lease_expires_at = '2099-01-01T00:00:00Z' \
             WHERE id = '{request_id}'"
        ))
        .await;
        assert!(matches!(
            db.record_provider_operation_transition(input(
                "runner-a",
                "wrong-token",
                "provider-correlation-1",
                ProviderOperationTransition::CorrelationReserved,
            ))
            .await,
            Err(CoreError::AgentCreationRequestLeaseConflict)
        ));
        assert!(matches!(
            db.record_provider_operation_transition(input(
                "runner-a",
                "token-a",
                "provider-correlation-1",
                ProviderOperationTransition::Provisioned {
                    provider_facts: json!({"api_token": "must-not-persist"}),
                },
            ))
            .await,
            Err(CoreError::InvalidProviderOperationFacts)
        ));
        assert!(matches!(
            db.record_provider_operation_transition(input(
                "runner-a",
                "token-a",
                "provider-correlation-1",
                ProviderOperationTransition::Provisioned {
                    provider_facts: json!({"provider_id": "must-not-skip-start"}),
                },
            ))
            .await,
            Err(CoreError::ProviderOperationTransitionConflict)
        ));

        let provision_started = db
            .record_provider_operation_transition(input(
                "runner-a",
                "token-a",
                "provider-correlation-1",
                ProviderOperationTransition::ProvisionStarted,
            ))
            .await
            .unwrap();
        assert!(matches!(
            db.fail_agent_creation_request(fail_input("runner-a", "token-a", None))
                .await,
            Err(CoreError::ProviderOperationBoundaryNotReached)
        ));
        assert!(matches!(
            db.cancel_agent_creation_request(CancelAgentCreationRequestInput {
                request_id: request_id.clone(),
                now: Some("2098-01-01T00:00:32Z".to_string()),
            })
            .await,
            Err(CoreError::ProviderOperationBoundaryNotReached)
        ));
        assert_eq!(
            db.provider_operation(&request_id).await.as_ref(),
            Some(&provision_started),
            "a crash after the pre-mutation fence remains resumable"
        );

        let provision_unknown = db
            .record_provider_operation_transition(input(
                "runner-a",
                "token-a",
                "provider-correlation-1",
                ProviderOperationTransition::ProvisionUnknown {
                    provider_facts: json!({"attempt": "timed_out"}),
                },
            ))
            .await
            .unwrap();
        assert!(matches!(
            db.fail_agent_creation_request(fail_input("runner-a", "token-a", None))
                .await,
            Err(CoreError::ProviderOperationBoundaryNotReached)
        ));
        assert_eq!(
            db.provider_operation(&request_id).await.as_ref(),
            Some(&provision_unknown)
        );
        assert!(matches!(
            db.record_provider_operation_transition(input(
                "runner-a",
                "token-a",
                "provider-correlation-1",
                ProviderOperationTransition::CommitStarted,
            ))
            .await,
            Err(CoreError::ProviderOperationTransitionConflict)
        ));
        let provisioned = db
            .record_provider_operation_transition(input(
                "runner-a",
                "token-a",
                "provider-correlation-1",
                ProviderOperationTransition::Provisioned {
                    provider_facts: json!({"provider_id": "opaque-123", "region": "test"}),
                },
            ))
            .await
            .unwrap();
        let provisioned_key = db
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id: request_id.clone(),
                runner_id: "runner-a".to_string(),
                lease_token: "token-a".to_string(),
                source_host_id: Some("ledger-host".to_string()),
                source_machine_id: Some("ledger-machine".to_string()),
                now: Some("2098-01-01T00:00:40Z".to_string()),
            })
            .await
            .unwrap();
        assert!(matches!(
            db.fail_agent_creation_request(fail_input(
                "runner-a",
                "token-a",
                Some(provisioned_key.api_key.id.clone()),
            ))
            .await,
            Err(CoreError::ProviderOperationBoundaryNotReached)
        ));
        assert_eq!(
            db.provider_operation(&request_id).await.as_ref(),
            Some(&provisioned)
        );
        assert_eq!(
            db.finite_private_api_key(&provisioned_key.api_key.id)
                .await
                .unwrap()
                .status,
            FinitePrivateApiKeyStatus::Active
        );
        assert!(matches!(
            db.cancel_agent_creation_request(CancelAgentCreationRequestInput {
                request_id: request_id.clone(),
                now: Some("2098-01-01T00:00:41Z".to_string()),
            })
            .await,
            Err(CoreError::ProviderOperationBoundaryNotReached)
        ));
        let committed = db
            .record_provider_operation_transition(input(
                "runner-a",
                "token-a",
                "provider-correlation-1",
                ProviderOperationTransition::CommitStarted,
            ))
            .await
            .unwrap();
        assert!(matches!(
            db.fail_agent_creation_request(fail_input(
                "runner-a",
                "token-a",
                Some(provisioned_key.api_key.id.clone()),
            ))
            .await,
            Err(CoreError::ProviderOperationBoundaryNotReached)
        ));
        assert_eq!(
            db.provider_operation(&request_id).await.as_ref(),
            Some(&committed)
        );

        db.exec(&format!(
            "UPDATE agent_creation_requests SET lease_expires_at = '2097-01-01T00:00:00Z' \
             WHERE id = '{request_id}'"
        ))
        .await;
        let second = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-b".to_string(),
                lease_token: "token-b".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                source_host_id: None,
                now: Some("2098-01-01T00:00:00Z".to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.provider_operation.as_ref(), Some(&committed));
        assert!(matches!(
            db.record_provider_operation_transition(input(
                "runner-a",
                "token-a",
                "provider-correlation-1",
                ProviderOperationTransition::CommitStarted,
            ))
            .await,
            Err(CoreError::AgentCreationRequestLeaseConflict)
        ));
        assert!(matches!(
            db.record_provider_operation_transition(input(
                "runner-b",
                "token-b",
                "different-correlation",
                ProviderOperationTransition::CommitStarted,
            ))
            .await,
            Err(CoreError::ProviderOperationIdentityMismatch)
        ));
        let replay_after_crash = db
            .record_provider_operation_transition(input(
                "runner-b",
                "token-b",
                "provider-correlation-1",
                ProviderOperationTransition::CommitStarted,
            ))
            .await
            .unwrap();
        assert_eq!(replay_after_crash, committed);

        let handle = ProviderRuntimeHandleEnvelope::V1(ProviderRuntimeHandleV1 {
            runner_class: RunnerClass::Kata,
            opaque: json!({"sandbox_id": "opaque-123"}),
        });
        let registered = db
            .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                request_id: request_id.clone(),
                runner_id: "runner-b".to_string(),
                lease_token: "token-b".to_string(),
                source_host_id: "ledger-host".to_string(),
                source_machine_id: "ledger-machine".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
                provider_runtime_handle: Some(handle.clone()),
                contact_endpoint: None,
                runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                    RuntimeCapabilitiesV1 {
                        recover_known_good_chat: true,
                        ..*kata_runtime_capabilities().v1()
                    },
                )),
                display_name: None,
                hostname: None,
                runtime_host: None,
                runtime_status: None,
                active_inference_profile: None,
                hermes_available: None,
                published_app_urls: Vec::new(),
                now: Some("2098-01-01T00:01:00Z".to_string()),
            })
            .await
            .unwrap();
        assert!(
            !db.agent_runtime(registered.request.agent_runtime_id.as_ref().unwrap())
                .await
                .unwrap()
                .runtime_capabilities
                .as_ref()
                .unwrap()
                .v1()
                .recover_known_good_chat,
            "an old artifact bounds the worker's process-wide recovery maximum"
        );
        assert!(matches!(
            registered
                .provider_operation
                .as_ref()
                .unwrap()
                .v1()
                .transitions
                .last()
                .unwrap()
                .transition,
            ProviderOperationTransition::ProviderHandleRecorded { .. }
        ));
        let runtime_id = registered.request.agent_runtime_id.clone().unwrap();
        let handle_recorded = registered.provider_operation.clone().unwrap();
        assert!(matches!(
            db.fail_agent_creation_request(fail_input(
                "runner-b",
                "token-b",
                Some(provisioned_key.api_key.id.clone()),
            ))
            .await,
            Err(CoreError::ProviderOperationBoundaryNotReached)
        ));
        assert_eq!(
            db.provider_operation(&request_id).await.as_ref(),
            Some(&handle_recorded)
        );
        assert!(db.agent_runtime(&runtime_id).await.is_some());
        assert_eq!(
            db.finite_private_api_key(&provisioned_key.api_key.id)
                .await
                .unwrap()
                .status,
            FinitePrivateApiKeyStatus::Active
        );
        assert!(matches!(
            db.cancel_agent_creation_request(CancelAgentCreationRequestInput {
                request_id: request_id.clone(),
                now: Some("2098-01-01T00:01:01Z".to_string()),
            })
            .await,
            Err(CoreError::ProviderOperationBoundaryNotReached)
        ));
        assert!(db.agent_runtime(&runtime_id).await.is_some());
        let completed = db
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id,
                runner_id: "runner-b".to_string(),
                lease_token: "token-b".to_string(),
                source_host_id: "ledger-host".to_string(),
                source_machine_id: "ledger-machine".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
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
                now: Some("2098-01-01T00:02:00Z".to_string()),
            })
            .await
            .unwrap();
        assert!(matches!(
            completed
                .provider_operation
                .unwrap()
                .v1()
                .transitions
                .last()
                .unwrap()
                .transition,
            ProviderOperationTransition::Ready
        ));
    })
    .await;
}
