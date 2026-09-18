use super::*;

#[tokio::test]
async fn core_api_lets_operator_cancel_failed_agent_creation_request() {
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
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: RequestAgentCreationResult = serde_json::from_slice(&body).unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/agent-creation-requests/lease")
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "runnerId": "runner-oslo-1",
                            "leaseToken": "lease-token-1",
                            "leaseSeconds": 300,
                            "now": "2026-05-25T13:00:00Z"
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
                    .uri(format!(
                        "/api/core/v1/agent-creation-requests/{}/fail",
                        created.request.id
                    ))
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "runnerId": "runner-oslo-1",
                            "leaseToken": "lease-token-1",
                            "failureMessage": "runtime did not publish a relay heartbeat",
                            "now": "2026-05-25T13:01:00Z"
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
                    .uri(format!(
                        "/api/core/v1/agent-creation-requests/{}/cancel",
                        created.request.id
                    ))
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let cancelled: AgentCreationRequest = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            cancelled.status,
            crate::AgentCreationRequestStatus::Cancelled
        );

        let response = app
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
        let projects: Vec<PublicVisibleProject> = serde_json::from_slice(&body).unwrap();
        assert!(projects.is_empty());
    })
    .await;
}

#[tokio::test]
async fn trusted_server_configuration_can_place_local_agent_creation() {
    with_isolated_postgres(|db| async move {
        let store = db.store.clone();
        let launch_code = issue_test_launch_code(&store).await;
        let placement = RuntimePlacement {
            runner_class: RunnerClass::AppleContainer,
            runtime_resource_class: crate::RuntimeResourceClass::Vcpu4Memory8Gib,
        };
        let app = router_with_agent_creation_placement(store, test_auth(), Some(placement));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/agent-creation-requests")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_local",
                                "local@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "displayName": "Local Agent",
                            "launchCode": launch_code,
                            "idempotencyKey": "local-browser-submit"
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
        let result: RequestAgentCreationResult = serde_json::from_slice(&body).unwrap();
        assert_eq!(result.project.hosting_tier, Some(HostingTier::Standard));
        assert_eq!(result.project.placement, Some(placement));
        assert_eq!(result.request.runner_class, RunnerClass::AppleContainer);
        assert_eq!(result.request.placement, Some(placement));
    })
    .await;
}
