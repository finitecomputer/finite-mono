use super::*;

#[tokio::test]
async fn launch_code_creates_one_self_serve_agent_request_and_visible_project() {
    with_isolated_postgres(|db| async move {
        let launch_code = issue_test_launch_code(&db).await;

        let first = db
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
        let second = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent duplicate submit".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: "first-submit".to_string(),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap();

        assert!(!first.reused);
        assert!(second.reused);
        assert_eq!(first.request.id, second.request.id);
        assert_eq!(first.project.id, second.project.id);
        assert_eq!(db.table_len("projects").await, 1);
        assert_eq!(db.table_len("agent_runtimes").await, 0);
        assert_eq!(db.table_len("agent_creation_requests").await, 1);
        assert_eq!(first.project.hosting_tier, Some(HostingTier::Standard));
        assert_eq!(
            first.project.placement,
            Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard))
        );
        assert_eq!(first.request.runner_class, RunnerClass::Kata);
        assert_eq!(first.request.hosting_tier, Some(HostingTier::Standard));
        let user = db.all_users().await.into_iter().next().unwrap();
        let org = db.all_customer_orgs().await.into_iter().next().unwrap();
        assert_eq!(org.billing_class, BillingClass::Sponsored);
        assert_eq!(
            db.visible_projects_for_user(&user.id)
                .await
                .into_iter()
                .map(|visible| visible.project)
                .collect::<Vec<_>>(),
            vec![first.project]
        );
    })
    .await;
}

#[tokio::test]
async fn confidential_launch_code_resolves_phala_placement_inside_core() {
    with_isolated_postgres(|db| async move {
        let launch_code = issue_launch_code(&db, Some(HostingTier::Confidential)).await;
        promote_runtime_artifact(&db).await;

        let requested = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "confidential@finite.vip".to_string(),
                workos_user_id: "user_workos_confidential".to_string(),
                display_name: "Confidential Agent".to_string(),
                launch_code,
                idempotency_key: "confidential-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();

        assert_eq!(
            requested.project.hosting_tier,
            Some(HostingTier::Confidential)
        );
        assert_eq!(
            requested.project.placement,
            Some(RuntimePlacement::for_hosting_tier(
                HostingTier::Confidential
            ))
        );
        assert_eq!(requested.request.runner_class, RunnerClass::Phala);

        let lease = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "phala-runner".to_string(),
                source_host_id: Some("phala-host".to_string()),
                lease_token: "phala-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                        RuntimeCapabilitiesV1 {
                            restart: true,
                            stop: true,
                            ..RuntimeCapabilitiesV1::default()
                        },
                    )),
                    ..phala_runner_capacity(0)
                }),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        let error = db
            .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                request_id: lease.request.id,
                runner_id: "phala-runner".to_string(),
                lease_token: "phala-lease".to_string(),
                source_host_id: "phala-host".to_string(),
                source_machine_id: "phala-cvm".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: Some("db-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: None,
                runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                    RuntimeCapabilitiesV1 {
                        restart: true,
                        runtime_upgrade: true,
                        stop: true,
                        ..RuntimeCapabilitiesV1::default()
                    },
                )),
                display_name: None,
                hostname: None,
                runtime_host: None,
                runtime_status: Some(RuntimeSummaryStatus::Unknown),
                active_inference_profile: None,
                hermes_available: None,
                published_app_urls: Vec::new(),
                now: Some("2026-05-25T13:01:00Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(error, CoreError::RuntimeCapabilitiesNotAuthorized));
    })
    .await;
}

#[tokio::test]
async fn selected_hosting_tier_must_match_launch_code_before_code_redemption() {
    with_isolated_postgres(|db| async move {
        let launch_code = issue_test_launch_code(&db).await;
        let input = RequestAgentCreationInput {
            verified_email: "tier-check@finite.vip".to_string(),
            workos_user_id: "user_workos_tier_check".to_string(),
            display_name: "Tier Check".to_string(),
            launch_code: launch_code.clone(),
            idempotency_key: "tier-check-submit".to_string(),
            now: Some(NOW.to_string()),
        };

        let denied = db
            .request_agent_creation_configured(
                input.clone(),
                AgentCreationConfiguration {
                    requested_hosting_tier: Some(HostingTier::Confidential),
                    ..AgentCreationConfiguration::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(denied, CoreError::HostingTierNotAuthorized));
        assert!(db.all_users().await.is_empty());
        assert!(db.all_projects().await.is_empty());
        assert!(db.all_agent_creation_requests().await.is_empty());

        let created = db
            .request_agent_creation_configured(
                input,
                AgentCreationConfiguration {
                    requested_hosting_tier: Some(HostingTier::Standard),
                    ..AgentCreationConfiguration::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(created.request.runner_class, RunnerClass::Kata);
    })
    .await;
}

#[tokio::test]
async fn cancelled_request_does_not_make_a_redeemed_launch_code_reusable() {
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
        db.fail_agent_creation_request(FailAgentCreationRequestInput {
            request_id: requested.request.id.clone(),
            runner_id: "runner-oslo-1".to_string(),
            lease_token: "lease-token-1".to_string(),
            failure_message: "runner capacity unavailable".to_string(),
            provisioned_finite_private_api_key_id: None,
            now: Some("2026-05-25T13:02:00Z".to_string()),
        })
        .await
        .unwrap();

        let cancelled = db
            .cancel_agent_creation_request(CancelAgentCreationRequestInput {
                request_id: requested.request.id,
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(cancelled.status, AgentCreationRequestStatus::Cancelled);
        assert!(cancelled.agent_runtime_id.is_none());
        assert!(
            db.visible_projects_for_user(&requested.project.owner_user_id)
                .await
                .is_empty()
        );

        let retry = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Retry Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: "second-submit".to_string(),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(retry, CoreError::InvalidLaunchCode));
    })
    .await;
}

#[tokio::test]
async fn fresh_launch_code_adds_one_creation_to_an_exhausted_entitlement() {
    with_isolated_postgres(|db| async move {
        let launch_code = issue_test_launch_code(&db).await;

        let bad = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: "wrong".to_string(),
                idempotency_key: "bad-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(bad, CoreError::InvalidLaunchCode));
        assert!(db.all_users().await.is_empty());
        assert!(db.all_customer_orgs().await.is_empty());
        assert!(db.all("agent_creation_entitlements").await.is_empty());

        db.request_agent_creation(RequestAgentCreationInput {
            verified_email: "new@finite.vip".to_string(),
            workos_user_id: "user_workos_new".to_string(),
            display_name: "Oslo Agent".to_string(),
            launch_code: launch_code.clone(),
            idempotency_key: "first-submit".to_string(),
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap();
        let unused_launch_code = issue_test_launch_code(&db).await;
        let unused_launch_code_id = issued_launch_code_id(&db, &unused_launch_code).await;
        let second = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Second Agent".to_string(),
                launch_code: unused_launch_code.clone(),
                idempotency_key: "second-submit".to_string(),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap();
        assert!(!second.reused);
        assert!(
            !db.row("launch_codes", &unused_launch_code_id)
                .await
                .unwrap()["redeemed_at"]
                .is_null(),
            "the top-up code must be consumed"
        );
        let entitlement = db
            .all("agent_creation_entitlements")
            .await
            .iter()
            .find(|entitlement| {
                entitlement["customer_org_id"] == second.project.customer_org_id.as_str()
            })
            .unwrap()
            .clone();
        assert_eq!(entitlement["allowed_new_agent_runtimes"], 2);
        let entitlement_org_id = entitlement["customer_org_id"].as_str().unwrap().to_string();

        let retry = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Second Agent".to_string(),
                launch_code: unused_launch_code,
                idempotency_key: "second-submit".to_string(),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap();
        assert!(retry.reused);
        assert_eq!(
            db.agent_creation_entitlement(&entitlement_org_id)
                .await
                .unwrap()
                .allowed_new_agent_runtimes,
            2,
            "an identical retry must not apply the top-up twice"
        );
    })
    .await;
}
