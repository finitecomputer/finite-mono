use super::*;

#[tokio::test]
async fn phala_capacity_reservation_is_atomic_and_releases_only_the_existing_in_flight_request() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let mut request_ids = Vec::new();
        for index in 0..2 {
            let launch_code = issue_launch_code(&db, Some(HostingTier::Confidential)).await;
            let requested = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: format!("confidential-{index}@finite.vip"),
                    workos_user_id: format!("user_workos_confidential_{index}"),
                    display_name: format!("Confidential Agent {index}"),
                    launch_code,
                    idempotency_key: format!("confidential-submit-{index}"),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            request_ids.push(requested.request.id);
        }

        let first = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "phala-runner-a".to_string(),
                source_host_id: Some("phala-host".to_string()),
                lease_token: "phala-lease-a".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(phala_runner_capacity(0)),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        assert!(request_ids.contains(&first.request.id));
        let waiting_request_id = request_ids
            .iter()
            .find(|request_id| request_id.as_str() != first.request.id)
            .unwrap();
        let reservation = first.in_flight_capacity_reservation.as_ref().unwrap().v1();
        assert_eq!(reservation.request_id, first.request.id);
        assert_eq!(
            reservation.placement,
            RuntimePlacement::for_hosting_tier(HostingTier::Confidential)
        );
        assert_eq!(reservation.provider_inventory_count, 0);
        assert_eq!(reservation.core_in_flight_count, 1);
        assert_eq!(reservation.max_sandbox_count, 1);

        let second = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "phala-runner-b".to_string(),
                source_host_id: Some("phala-host".to_string()),
                lease_token: "phala-lease-b".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(phala_runner_capacity(0)),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap();
        assert!(second.is_none());

        let resumed = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "phala-runner-c".to_string(),
                source_host_id: Some("phala-host".to_string()),
                lease_token: "phala-lease-c".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(phala_runner_capacity(1)),
                now: Some("2026-05-25T14:00:00Z".to_string()),
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
        assert_eq!(
            db.agent_creation_request(waiting_request_id)
                .await
                .unwrap()
                .status,
            AgentCreationRequestStatus::Requested
        );
    })
    .await;
}

#[tokio::test]
async fn project_selected_runner_class_routes_to_a_matching_worker() {
    with_isolated_postgres(|db| async move {
        let launch_code = issue_test_launch_code(&db).await;
        promote_runtime_artifact(&db).await;
        let requested = db
            .request_agent_creation_configured(
                RequestAgentCreationInput {
                    verified_email: "kata@finite.vip".to_string(),
                    workos_user_id: "user_workos_kata".to_string(),
                    display_name: "Kata Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "kata-submit".to_string(),
                    now: Some(NOW.to_string()),
                },
                AgentCreationConfiguration {
                    placement: Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
                    requested_hosting_tier: None,
                    profile_picture_url: Some(
                        "https://chat.finite.computer/v1/blobs/profile".to_string(),
                    ),
                    owner_chat_account_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(requested.request.runner_class, RunnerClass::Kata);

        let draining_kata = RunnerLeaseCapacity {
            draining: true,
            runner_classes: vec![RunnerClass::Kata],
            runtime_capabilities: Some(kata_runtime_capabilities()),
            ..RunnerLeaseCapacity::default()
        };
        assert!(!draining_kata.accepts_agent_creation());
        assert!(draining_kata.accepts_runtime_control());

        let phala = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "phala-worker".to_string(),
                source_host_id: None,
                lease_token: "phala-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(phala_runner_capacity(0)),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap();
        assert!(phala.is_none());

        let unspecified = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "unspecified-worker".to_string(),
                source_host_id: None,
                lease_token: "unspecified-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity::default()),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap();
        assert!(unspecified.is_none());

        let kata = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "kata-worker".to_string(),
                source_host_id: None,
                lease_token: "kata-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap()
            .expect("Kata worker should claim Kata placement");
        assert_eq!(kata.request.id, requested.request.id);
    })
    .await;
}

#[tokio::test]
async fn kata_is_the_only_runtime_recovery_capability_boundary() {
    with_isolated_postgres(|db| async move {
        let recover = RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
            recover_known_good_chat: true,
            ..RuntimeCapabilitiesV1::default()
        });
        assert!(
            RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Kata],
                runtime_capabilities: Some(recover.clone()),
                ..RunnerLeaseCapacity::default()
            }
            .validate_runtime_capability_policy()
            .is_ok()
        );
        for runner_classes in [
            Vec::new(),
            vec![RunnerClass::Phala],
            vec![RunnerClass::Kata, RunnerClass::Phala],
        ] {
            assert!(matches!(
                (RunnerLeaseCapacity {
                    runner_classes,
                    runtime_capabilities: Some(recover.clone()),
                    ..RunnerLeaseCapacity::default()
                })
                .validate_runtime_capability_policy(),
                Err(CoreError::RuntimeCapabilitiesNotAuthorized)
            ));
        }
        assert!(
            validate_runtime_capabilities_policy(
                Some(&recover),
                Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard))
            )
            .is_ok()
        );
        assert!(matches!(
            validate_runtime_capabilities_policy(
                Some(&recover),
                Some(RuntimePlacement::for_hosting_tier(
                    HostingTier::Confidential
                ))
            ),
            Err(CoreError::RuntimeCapabilitiesNotAuthorized)
        ));
        promote_runtime_artifact(&db).await;
        let legacy_artifact = db
            .runtime_artifact_row("artifact-v1")
            .await
            .unwrap()
            .clone();
        assert!(matches!(
            validate_runtime_capabilities_artifact_policy(
                Some(&recover),
                Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
                &legacy_artifact,
            ),
            Err(CoreError::RuntimeCapabilitiesNotAuthorized)
        ));
        let capable_artifact = RuntimeArtifact {
            canary_runtime_id: None,
            recover_known_good_chat: true,
            ..legacy_artifact.clone()
        };
        assert!(
            validate_runtime_capabilities_artifact_policy(
                Some(&recover),
                Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
                &capable_artifact,
            )
            .is_ok()
        );
        assert!(
            !runtime_artifact_material_matches(&legacy_artifact, &capable_artifact),
            "artifact recovery support is immutable release material"
        );
        for key in ["FINITE_AGENT_BOOT_INTENT_JSON", "FINITE_AGENT_STATE_ROOT"] {
            assert!(matches!(
                validate_runtime_spec_environment(&BTreeMap::from([(
                    key.to_string(),
                    "caller-owned".to_string()
                )])),
                Err(CoreError::RuntimeSpecMismatch)
            ));
        }
    })
    .await;
}

#[tokio::test]
async fn runner_leases_and_completes_self_serve_agent_request() {
    with_isolated_postgres(|db| async move {
        let launch_code = issue_test_launch_code(&db).await;
        promote_runtime_artifact(&db).await;
        db.exec(
            "UPDATE runtime_artifacts SET recover_known_good_chat = true WHERE id = 'artifact-v1'",
        )
        .await;
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
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap()
            .expect("pending request should be leased");
        assert_eq!(lease.project.id, requested.project.id);
        assert_eq!(lease.request.status, AgentCreationRequestStatus::Launching);
        assert_eq!(lease.request.runner_id.as_deref(), Some("runner-oslo-1"));
        assert!(lease.request.lease_expires_at.is_some());

        let none = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-2".to_string(),
                source_host_id: None,
                lease_token: "lease-token-2".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some("2026-05-25T13:01:00Z".to_string()),
            })
            .await
            .unwrap();
        assert!(none.is_none());

        let completed = db
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
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
                runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                    RuntimeCapabilitiesV1 {
                        recover_known_good_chat: true,
                        ..*kata_runtime_capabilities().v1()
                    },
                )),
                display_name: None,
                hostname: Some("oslo-agent-001.finite.computer".to_string()),
                runtime_host: Some("oslo-host-1".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                agent_npub: None,
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(
            completed.request.status,
            AgentCreationRequestStatus::Running
        );
        assert!(completed.request.lease_token.is_none());
        let runtime_id = completed.request.agent_runtime_id.unwrap();
        let runtime = db.agent_runtime(&runtime_id).await.unwrap();
        assert!(
            runtime
                .runtime_capabilities
                .as_ref()
                .unwrap()
                .v1()
                .recover_known_good_chat
        );
        assert_eq!(runtime.project_id, requested.project.id);
        assert_eq!(runtime.runtime_artifact_id.as_deref(), Some("artifact-v1"));
        assert_eq!(runtime.state_schema_version.as_deref(), Some("db-v1"));
        assert_eq!(runtime.source_host_id, "oslo-host-1");
        assert_eq!(runtime.source_machine_id, "oslo-agent-001");
        assert_eq!(
            runtime.host_facts.runtime_status,
            RuntimeSummaryStatus::Online
        );
        assert_eq!(
            db.all("project_runtime_links")
                .await
                .iter()
                .filter(|link| link["project_id"] == requested.project.id.as_str()
                    && link["active"] == true)
                .count(),
            1
        );
    })
    .await;
}

#[tokio::test]
async fn runner_lease_can_expire_and_reassign_but_completion_requires_current_token() {
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
        let first_lease = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-a".to_string(),
                source_host_id: None,
                lease_token: "lease-a".to_string(),
                lease_seconds: Some(60),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first_lease.request.project_id, requested.project.id);
        let second_lease = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-b".to_string(),
                source_host_id: None,
                lease_token: "lease-b".to_string(),
                lease_seconds: Some(60),
                runner_capacity: None,
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second_lease.request.runner_id.as_deref(), Some("runner-b"));

        let stale_complete = db
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: requested.request.id,
                runner_id: "runner-a".to_string(),
                lease_token: "lease-a".to_string(),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "oslo-agent-001".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
                provider_runtime_handle: None,
                contact_endpoint: None,
                runtime_capabilities: None,
                display_name: None,
                hostname: None,
                runtime_host: None,
                runtime_status: None,
                active_inference_profile: None,
                hermes_available: None,
                published_app_urls: Vec::new(),
                agent_npub: None,
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            stale_complete,
            CoreError::AgentCreationRequestLeaseConflict
        ));
    })
    .await;
}
