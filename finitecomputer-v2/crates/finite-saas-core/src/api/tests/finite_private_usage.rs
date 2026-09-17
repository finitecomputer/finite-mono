use super::*;

#[tokio::test]
async fn finite_private_user_controls_share_one_daily_claim_across_key_and_dashboard() {
    with_isolated_postgres(|db| async move {
        let store = db.store.clone();
        let grant = store
            .approve_finite_private_grant(crate::ApproveFinitePrivateGrantInput {
                verified_email: "controls@finite.vip".to_string(),
                workos_user_id: Some("user_workos_controls".to_string()),
                limit_profile_id: None,
                now: None,
            })
            .await
            .unwrap();
        store
            .issue_finite_private_api_key(crate::IssueFinitePrivateApiKeyInput {
                grant_id: grant.id,
                raw_key: "fpk_live_controls".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: None,
            })
            .await
            .unwrap();
        let app = router(store, test_auth());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/finite-private/usage")
                    .header("authorization", "Bearer fpk_live_controls")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let status: crate::FinitePrivateUsageStatus = serde_json::from_slice(&body).unwrap();
        assert_eq!(status.burst_limit_units, 100_000_000);
        assert!(status.free_daily_reset_available);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/finite-private/usage/reset")
                    .header("authorization", "Bearer fpk_live_controls")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let reset: crate::FinitePrivateDailyResetResult = serde_json::from_slice(&body).unwrap();
        assert!(reset.performed);

        let dashboard_authorization = format!(
            "Bearer {}",
            access_token_with_subject("user_workos_controls", "controls@finite.vip", true, None,)
        );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/finite-private/usage/reset")
                    .header("authorization", &dashboard_authorization)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let dashboard_reset: Option<crate::FinitePrivateDailyResetResult> =
            serde_json::from_slice(&body).unwrap();
        assert!(!dashboard_reset.unwrap().performed);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/finite-private/usage")
                    .header("authorization", "Bearer invalid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    })
    .await;
}

#[tokio::test]
async fn core_api_serves_finite_private_usage_reserve_and_settle() {
    with_isolated_postgres(|db| async move {
        let store = db.store.clone();
        let grant = store
            .approve_finite_private_grant(crate::ApproveFinitePrivateGrantInput {
                verified_email: "private@finite.vip".to_string(),
                workos_user_id: Some("user_workos_private".to_string()),
                limit_profile_id: None,
                now: Some("2026-05-25T12:00:00Z".to_string()),
            })
            .await
            .unwrap();
        store
            .issue_finite_private_api_key(crate::IssueFinitePrivateApiKeyInput {
                grant_id: grant.id,
                raw_key: "fpk_live_secret".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: Some("2026-05-25T12:00:00Z".to_string()),
            })
            .await
            .unwrap();
        let app = router(store, test_auth());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/internal/finite-private/v1/health")
                    .header("authorization", "Bearer core-token")
                    .body(Body::empty())
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
                    .uri("/internal/finite-private/v1/reservations")
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "requestId": "req-private-api-1",
                            "presentedApiKey": "fpk_live_secret",
                            "endpoint": "/v1/chat/completions",
                            "model": "kimi-k2-6",
                            "estimatedPromptTokens": 120000,
                            "estimatedCompletionTokens": 4096,
                            "estimatedUsageUnits": 250000,
                            "usageFormulaVersion": "2026-05-26.v1",
                            "dashboardUrl": "https://finite.computer/dashboard",
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
        let decision: FinitePrivateUsageDecision = serde_json::from_slice(&body).unwrap();
        assert_eq!(decision.decision, "allow");
        let reservation_id = decision.reservation_id.unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/internal/finite-private/v1/reservations/{reservation_id}/settle"
                    ))
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "requestId": "req-private-api-1",
                            "settlement": "actual",
                            "promptTokens": 120000,
                            "completionTokens": 1200,
                            "usageUnits": 160000,
                            "usageFormulaVersion": "2026-05-26.v1",
                            "upstreamStatus": 200,
                            "upstreamErrorClass": null,
                            "now": "2026-05-25T13:05:00Z"
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
        let settled: SettleFinitePrivateReservationResult = serde_json::from_slice(&body).unwrap();
        assert!(settled.settled);
        assert_eq!(settled.reservation_id, reservation_id);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/finite-private/v1/reservations")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "requestId": "req-private-api-unauth",
                            "presentedApiKey": "fpk_live_secret",
                            "endpoint": "/v1/chat/completions",
                            "model": "kimi-k2-6",
                            "estimatedPromptTokens": 100,
                            "estimatedCompletionTokens": 100,
                            "estimatedUsageUnits": 200,
                            "usageFormulaVersion": "2026-05-26.v1",
                            "dashboardUrl": "https://finite.computer/dashboard"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    })
    .await;
}
