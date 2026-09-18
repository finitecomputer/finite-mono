use super::*;

#[tokio::test]
async fn core_api_lets_runner_lease_and_complete_agent_creation_request() {
    with_isolated_postgres(|db| async move {
        let store = db.store.clone();
        let launch_code = issue_test_launch_code(&store).await;
        let app = router(store, scoped_test_auth());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/core/v1/runtime-artifacts/artifact-v1")
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "kind": "oci_image",
                            "reference": format!(
                                "ghcr.io/finitecomputer/agent-runtime:v1@sha256:{}",
                                "a".repeat(64)
                            ),
                            "versionLabel": "v1",
                            "stateSchemaVersion": "state-v1",
                            "baseImage": "python:3.11-trixie",
                            "promoted": true,
                            "now": "2026-05-25T12:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let create = serde_json::to_vec(&CreateAgentRequest {
            display_name: "Oslo Agent".to_string(),
            launch_code: launch_code.clone(),
            idempotency_key: "browser-submit-1".to_string(),
            hosting_tier: None,
            profile_picture_url: None,
            owner_chat_account_id: None,
        })
        .unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/agent-creation-requests")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(create))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/agent-creation-requests/lease")
                    .header("authorization", runner_authorization())
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "runnerId": "runner-oslo-1",
                            "leaseToken": "lease-token-1",
                            "leaseSeconds": 300,
                            "runnerCapacity": runner_capacity_json(RunnerClass::Kata)
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let lease: Option<AgentCreationLease> = serde_json::from_slice(&body).unwrap();
        let lease = lease.unwrap();
        assert_eq!(
            lease.request.status,
            crate::AgentCreationRequestStatus::Launching
        );
        assert_eq!(lease.request.runner_id.as_deref(), Some("runner-oslo-1"));

        let operation_uri = format!(
            "/api/core/v1/agent-creation-requests/{}/provider-operation/transitions",
            lease.request.id
        );
        let runner_headers = vec![("authorization".to_string(), runner_authorization())];
        let transition =
            |lease_token: &str, correlation_id: &str, transition: serde_json::Value| {
                serde_json::json!({
                    "runnerId": "runner-oslo-1",
                    "leaseToken": lease_token,
                    "correlationId": correlation_id,
                    "placement": {
                        "runnerClass": "kata",
                        "runtimeResourceClass": "vcpu4_memory8_gib"
                    },
                    "transition": transition
                })
            };
        let reserve = transition(
            "lease-token-1",
            "api-correlation-1",
            serde_json::json!({"kind": "correlation_reserved"}),
        );
        let (status, first_ack) = send_json(
            &app,
            "POST",
            &operation_uri,
            &runner_headers,
            Some(reserve.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, replay_ack) =
            send_json(&app, "POST", &operation_uri, &runner_headers, Some(reserve)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(replay_ack, first_ack);
        let (status, _) = send_json(
            &app,
            "POST",
            &operation_uri,
            &runner_headers,
            Some(transition(
                "wrong-token",
                "api-correlation-1",
                serde_json::json!({"kind": "provisioned", "provider_facts": {}}),
            )),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        for provider_transition in [
            serde_json::json!({"kind": "provision_started"}),
            serde_json::json!({
                "kind": "provisioned",
                "provider_facts": {"provider_id": "opaque-api-1"}
            }),
            serde_json::json!({"kind": "commit_started"}),
        ] {
            let (status, _) = send_json(
                &app,
                "POST",
                &operation_uri,
                &runner_headers,
                Some(transition(
                    "lease-token-1",
                    "api-correlation-1",
                    provider_transition,
                )),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let me: MeResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(me.agent_creation_requests.len(), 1);
        assert_eq!(
            me.agent_creation_requests[0].status,
            crate::AgentCreationRequestStatus::Launching
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/agent-creation-requests/{}/complete",
                        lease.request.id
                    ))
                    .header("authorization", runner_authorization())
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "runnerId": "runner-oslo-1",
                            "leaseToken": "lease-token-1",
                            "sourceHostId": "oslo-host-1",
                            "sourceMachineId": "oslo-agent-001",
                            "runtimeArtifactId": "artifact-v1",
                            "providerRuntimeHandle": {
                                "schema": "provider_runtime_handle.v1",
                                "handle": {
                                    "runnerClass": "kata",
                                    "opaque": {"sandbox_id": "opaque-api-1"}
                                }
                            },
                            "contactEndpoint": "https://oslo-agent.example.test/contact/",
                            "hostname": "oslo-agent-001.finite.computer",
                            "runtimeHost": "oslo-host-1",
                            "runtimeStatus": "online",
                            "activeInferenceProfile": "finite-private",
                            "hermesAvailable": true,
                            "publishedAppUrls": [],
                            "runtimeCapabilities": runtime_capabilities_json(true),
                            "now": "2026-05-25T13:01:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let completed: AgentCreationLease = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            completed.request.status,
            crate::AgentCreationRequestStatus::Running
        );
        assert!(completed.request.agent_runtime_id.is_some());
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

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me/projects")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let projects_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_json_omits_keys(
            &projects_json,
            &[
                "runner_class",
                "placement",
                "source_host_id",
                "source_machine_id",
                "source_import_key",
                "provider_runtime_handle",
                "provider_runtime_handle_history",
                "published_app_urls",
                "host_facts",
            ],
        );
        let projects: Vec<PublicVisibleProject> = serde_json::from_slice(&body).unwrap();
        assert_eq!(projects.len(), 1);
        // Launched but never reported on: the derived status is unknown
        // while the lifecycle latch stays visible under its own name.
        assert_eq!(
            projects[0].runtime.as_ref().unwrap().runtime_status,
            RuntimeSummaryStatus::Unknown
        );
        assert_eq!(
            projects[0].runtime.as_ref().unwrap().lifecycle_status,
            RuntimeSummaryStatus::Online
        );
        assert_eq!(
            projects[0]
                .runtime
                .as_ref()
                .unwrap()
                .runtime_health
                .as_ref()
                .map(|health| health.status),
            Some(RuntimeHealthStatus::Unknown)
        );
        assert_eq!(
            projects[0]
                .runtime
                .as_ref()
                .unwrap()
                .contact_endpoint
                .as_deref(),
            Some("https://oslo-agent.example.test/contact")
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me/runtime-routes/oslo-agent-001")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resolution: RuntimeRouteResolution = serde_json::from_slice(&body).unwrap();
        assert_eq!(resolution.project_id, projects[0].project.id);
        assert_eq!(
            resolution.runtime_id,
            projects[0].runtime.as_ref().unwrap().id
        );
    })
    .await;
}

#[tokio::test]
async fn core_api_skips_full_or_draining_runner_without_blocking_other_runner() {
    with_isolated_postgres(|db| async move {
        let store = db.store.clone();
        let launch_code = issue_test_launch_code(&store).await;
        let app = router(store, test_auth());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/core/v1/runtime-artifacts/artifact-v1")
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "kind": "oci_image",
                            "reference": format!(
                                "ghcr.io/finitecomputer/agent-runtime:v1@sha256:{}",
                                "a".repeat(64)
                            ),
                            "versionLabel": "v1",
                            "stateSchemaVersion": "state-v1",
                            "baseImage": "python:3.11-trixie",
                            "promoted": true,
                            "now": "2026-05-25T12:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/agent-creation-requests")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "displayName": "Oslo Agent",
                            "launchCode": launch_code.clone(),
                            "idempotencyKey": "browser-submit-1"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        for runner_capacity in [
            serde_json::json!({
                "draining": true,
                "maxSandboxCount": 4,
                "activeSandboxCount": 1,
                "availableMemoryBytes": 8589934592_u64,
                "runnerClasses": ["kata"],
                "runtimeCapabilities": runtime_capabilities_json(true)
            }),
            serde_json::json!({
                "draining": false,
                "maxSandboxCount": 1,
                "activeSandboxCount": 1,
                "availableMemoryBytes": 1073741824_u64,
                "runnerClasses": ["kata"],
                "runtimeCapabilities": runtime_capabilities_json(true)
            }),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/core/v1/agent-creation-requests/lease")
                        .header("authorization", format!("Bearer {FULL_RUNNER_TOKEN}"))
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::json!({
                                "runnerId": "runner-oslo-full",
                                "leaseToken": "lease-token-full",
                                "leaseSeconds": 300,
                                "runnerCapacity": runner_capacity,
                                "now": "2026-05-25T13:00:00Z"
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let lease: Option<AgentCreationLease> = serde_json::from_slice(&body).unwrap();
            assert!(lease.is_none());
        }

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/agent-creation-requests/lease")
                    .header("authorization", format!("Bearer {SECOND_RUNNER_TOKEN}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "runnerId": "runner-oslo-2",
                            "leaseToken": "lease-token-2",
                            "leaseSeconds": 300,
                            "runnerCapacity": {
                                "draining": false,
                                "maxSandboxCount": 4,
                                "activeSandboxCount": 1,
                                "availableMemoryBytes": 8589934592_u64,
                                "runnerClasses": ["kata"],
                                "runtimeCapabilities": runtime_capabilities_json(true)
                            },
                            "now": "2026-05-25T13:00:01Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let lease: Option<AgentCreationLease> = serde_json::from_slice(&body).unwrap();
        let lease = lease.expect("available runner should lease queued work");
        assert_eq!(lease.request.runner_id.as_deref(), Some("runner-oslo-2"));
    })
    .await;
}
