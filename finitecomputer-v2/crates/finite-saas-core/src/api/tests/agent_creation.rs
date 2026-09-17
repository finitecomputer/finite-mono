use super::*;

#[tokio::test]
async fn core_api_owner_chat_account_id_validation() {
    with_isolated_postgres(|db| async move {
        let store = db.store.clone();
        let app = router(store, test_auth());
        let submit = |owner_chat_account_id: serde_json::Value, key: &str| {
            let app = app.clone();
            let store = db.store.clone();
            let key = key.to_string();
            async move {
                let launch_code = issue_test_launch_code(&store).await;
                app.oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/core/v1/me/agent-creation-requests")
                        .header(
                            "authorization",
                            format!(
                                "Bearer {}",
                                access_token_with_subject(
                                    "user_workos_owner_npub",
                                    "owner-npub@finite.vip",
                                    true,
                                    None,
                                )
                            ),
                        )
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::json!({
                                "displayName": "Owner Npub Agent",
                                "launchCode": launch_code,
                                "idempotencyKey": key,
                                "ownerChatAccountId": owner_chat_account_id,
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap()
            }
        };

        // Malformed values are rejected with 400 before any durable state.
        let response = submit(serde_json::json!("npub1qqqqqq"), "owner-npub-bad").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Mixed-version contract the dashboard's fail-open retry keys on:
        // a field the receiving Core does not know (a new dashboard
        // posting `ownerChatAccountId` to a pre-owner-npub Core hits the
        // same path) is a 422 JSON data rejection, never a 400.
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
                                "user_workos_owner_npub",
                                "owner-npub@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "displayName": "Unknown Field",
                            "launchCode": "finite_test",
                            "idempotencyKey": "owner-npub-unknown-field",
                            "futureUnknownField": "x",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        // Missing the field keeps the legacy allow-all admission path.
        let response = submit(serde_json::Value::Null, "owner-npub-absent").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let result: RequestAgentCreationResult = serde_json::from_slice(&body).unwrap();
        assert_eq!(result.request.owner_chat_account_id, None);

        // A 64-hex account id is accepted, normalized, and persisted.
        let owner_account_id = "b".repeat(64);
        let response = submit(
            serde_json::json!(owner_account_id.to_uppercase()),
            "owner-npub-good",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let result: RequestAgentCreationResult = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            result.request.owner_chat_account_id.as_deref(),
            Some(owner_account_id.as_str())
        );
    })
    .await;
}

#[tokio::test]
async fn core_api_creates_self_serve_agent_request_with_launch_code() {
    with_isolated_postgres(|db| async move {
        let store = db.store.clone();
        let launch_code = issue_test_launch_code(&store).await;
        let app = router(store, test_auth());
        let create = serde_json::to_vec(&CreateAgentRequest {
            display_name: "Oslo Agent".to_string(),
            launch_code: launch_code.clone(),
            idempotency_key: "browser-submit-1".to_string(),
            hosting_tier: None,
            profile_picture_url: Some("https://chat.finite.computer/v1/blobs/profile".to_string()),
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
                    .body(Body::from(create.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let result: RequestAgentCreationResult = serde_json::from_slice(&body).unwrap();
        assert_eq!(result.project.display_name, "Oslo Agent");
        let agent_email = result
            .project
            .agent_email
            .as_deref()
            .expect("new hosted agents receive a canonical email");
        assert!(agent_email.starts_with("oslo-agent-"));
        assert!(agent_email.ends_with("@finite.vip"));

        assert!(result.project.import_candidate_id.is_none());
        assert!(result.request.agent_runtime_id.is_none());
        assert_eq!(result.request.runner_class, RunnerClass::Kata);
        assert_eq!(
            result.request.profile_picture_url.as_deref(),
            Some("https://chat.finite.computer/v1/blobs/profile")
        );
        assert!(!result.reused);

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
        assert_eq!(me.projects.len(), 1);
        assert_eq!(me.projects[0].project.id, result.project.id);
        assert_eq!(
            me.projects[0].project.agent_email.as_deref(),
            Some(agent_email)
        );
        assert!(me.projects[0].runtime.is_none());
        assert_eq!(me.agent_creation_requests.len(), 1);
        assert_eq!(me.agent_creation_requests[0].project_id, result.project.id);
        assert_eq!(
            me.agent_creation_requests[0].status,
            crate::AgentCreationRequestStatus::Requested
        );

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
        let retry: RequestAgentCreationResult = serde_json::from_slice(&body).unwrap();
        assert!(retry.reused);
        assert_eq!(retry.project.id, result.project.id);

        let second = serde_json::to_vec(&CreateAgentRequest {
            display_name: "Second Agent".to_string(),
            launch_code: launch_code.clone(),
            idempotency_key: "browser-submit-2".to_string(),
            hosting_tier: None,
            profile_picture_url: None,
            owner_chat_account_id: None,
        })
        .unwrap();
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
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(second))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    })
    .await;
}

#[tokio::test]
async fn core_api_targeted_code_preserves_host_through_creation_and_retry() {
    with_isolated_postgres(|db| async move {
        let store = db.store.clone();
        store
            .link_verified_user(LinkVerifiedUserInput {
                verified_email: "new@finite.vip".into(),
                workos_user_id: "user_workos_new".into(),
                now: None,
            })
            .await
            .unwrap();
        let issued = store
            .issue_launch_code_batch(crate::launch_codes::IssueLaunchCodeBatchInput {
                name: "API canary".into(),
                code_count: 1,
                expires_in_hours: Some(1),
                hosting_tier: Some(crate::HostingTier::Standard),
                created_by_workos_user_id: "user_workos_new".into(),
                now: None,
            })
            .await
            .unwrap();
        let code = &issued.codes[0];
        store
            .target_launch_code_exact(
                &code.id,
                &issued.batch.id,
                "target-host",
                "new@finite.vip",
                "user_workos_new",
            )
            .await
            .unwrap();
        let app = router(store.clone(), test_auth());
        let create = serde_json::to_vec(&CreateAgentRequest {
            display_name: "API Canary".into(),
            launch_code: code.code.clone(),
            idempotency_key: "targeted-browser-submit".into(),
            hosting_tier: None,
            profile_picture_url: None,
            owner_chat_account_id: Some("a".repeat(64)),
        })
        .unwrap();
        let mut request_id = None;
        for retry in [false, true] {
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
                                    None
                                )
                            ),
                        )
                        .header("content-type", "application/json")
                        .body(Body::from(create.clone()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let result: RequestAgentCreationResult = serde_json::from_slice(&body).unwrap();
            assert_eq!(result.reused, retry);
            assert_eq!(
                result.request.target_source_host_id.as_deref(),
                Some("target-host")
            );
            assert_eq!(
                result.request.status,
                crate::AgentCreationRequestStatus::Requested
            );
            assert!(result.request.agent_runtime_id.is_none());
            assert_eq!(
                result.request.owner_chat_account_id.as_deref(),
                Some("a".repeat(64).as_str())
            );
            if let Some(ref first) = request_id {
                assert_eq!(&result.request.id, first);
            }
            request_id = Some(result.request.id);
        }
        // The persisted request, not only the HTTP response, must exclude
        // another host and a caller with no declared host identity.
        for host in [Some("wrong-host"), None, Some("target-host")] {
            let leased = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: host.unwrap_or("missing-host").into(),
                    source_host_id: host.map(String::from),
                    lease_token: "api-canary-lease".into(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: None,
                })
                .await
                .unwrap();
            if host == Some("target-host") {
                assert_eq!(Some(leased.unwrap().request.id), request_id);
            } else {
                assert!(leased.is_none());
            }
        }
    })
    .await;
}

#[tokio::test]
async fn core_api_rejects_tier_mismatch_without_consuming_launch_code() {
    with_isolated_postgres(|db| async move {
        let store = db.store.clone();
        let launch_code = issue_test_launch_code(&store).await;
        let app = router(store, test_auth());
        let user = identity_headers("tier-check@finite.vip", "true");
        let mut request = serde_json::json!({
            "displayName": "Tier Check",
            "launchCode": launch_code,
            "idempotencyKey": "tier-check-submit",
            "hostingTier": "confidential"
        });

        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/me/agent-creation-requests",
            &user,
            Some(request.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        request["hostingTier"] = serde_json::json!("standard");
        let (status, body) = send_json(
            &app,
            "POST",
            "/api/core/v1/me/agent-creation-requests",
            &user,
            Some(request),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["request"]["runner_class"], "kata");
    })
    .await;
}

#[tokio::test]
async fn core_api_uses_server_time_for_launch_code_expiry() {
    with_isolated_postgres(|db| async move {
        let store = db.store.clone();
        let issued = store
            .issue_launch_code_batch(crate::launch_codes::IssueLaunchCodeBatchInput {
                name: "Expired browser code".to_string(),
                code_count: 1,
                expires_in_hours: Some(1),
                hosting_tier: None,
                created_by_workos_user_id: "workos-test-operator".to_string(),
                now: Some("2020-01-01T00:00:00Z".to_string()),
            })
            .await
            .expect("expired test batch should issue");
        let plaintext = issued.codes[0].code.clone();
        let app = router(store.clone(), test_auth());
        let user = identity_headers("expired-code@finite.vip", "true");
        let request = serde_json::json!({
            "displayName": "Expired Agent",
            "launchCode": plaintext,
            "idempotencyKey": "expired-browser-submit"
        });

        let mut forged = request.clone();
        forged["now"] = serde_json::json!("2020-01-01T00:30:00Z");
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/me/agent-creation-requests",
            &user,
            Some(forged),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/me/agent-creation-requests",
            &user,
            Some(request),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let batches = store.list_launch_code_batches().await.unwrap();
        assert_eq!(batches.len(), 1);
        assert!(batches[0].codes[0].redeemed_at.is_none());
        assert!(batches[0].codes[0].redeemed_customer_org_id.is_none());
    })
    .await;
}
