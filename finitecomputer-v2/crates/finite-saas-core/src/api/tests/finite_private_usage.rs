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
        assert_eq!(status.burst_limit_units, 200_000_000);
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

#[tokio::test]
async fn finite_private_request_diagnostics_are_idempotent_and_separate_from_accounting() {
    with_isolated_postgres(|db| async move {
        let store = db.store.clone();
        let grant = store
            .approve_finite_private_grant(crate::ApproveFinitePrivateGrantInput {
                verified_email: "metrics@finite.vip".to_string(),
                workos_user_id: Some("user_workos_metrics".to_string()),
                limit_profile_id: None,
                now: None,
            })
            .await
            .unwrap();
        db.query_json("WITH org AS (INSERT INTO customer_orgs (id,owner_user_id,name,billing_class,created_at,updated_at) VALUES ('org-metrics',$1,'Synthetic','standard',NOW(),NOW()) ON CONFLICT (owner_user_id) DO UPDATE SET name=customer_orgs.name RETURNING id), project AS (INSERT INTO projects (id,customer_org_id,owner_user_id,display_name,created_at,updated_at) SELECT 'project-metrics',id,$1,'Synthetic',NOW(),NOW() FROM org RETURNING id) SELECT to_jsonb(project) FROM project", &[&grant.user_id]).await;
        store
            .issue_finite_private_api_key(crate::IssueFinitePrivateApiKeyInput {
                grant_id: grant.id.clone(),
                raw_key: "fpk_live_metrics".to_string(),
                project_id: Some("project-metrics".into()),
                agent_runtime_id: None,
                now: None,
            })
            .await
            .unwrap();
        let decision = store
            .reserve_finite_private_usage(crate::ReserveFinitePrivateUsageInput {
                request_id: "req-metrics-1".to_string(),
                presented_api_key: "fpk_live_metrics".to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "glm-5-3-flash".to_string(),
                estimated_prompt_tokens: 10,
                estimated_completion_tokens: 20,
                estimated_usage_units: 70,
                usage_formula_version: "2026-05-26.v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: None,
            })
            .await
            .unwrap();
        let reservation_id = decision.reservation_id.unwrap();
        let input = crate::RecordFinitePrivateRequestDiagnosticInput {
            reservation_id: reservation_id.clone(),
            request_id: "req-metrics-1".to_string(),
            prompt_tokens: Some(11),
            completion_tokens: Some(19),
            first_output_ms: Some(240),
            first_answer_ms: Some(300),
            duration_ms: Some(900),
            termination_reason: "complete".to_string(),
            measurement_quality: "observed_usage".to_string(),
        };
        // Existing key issuance can update the association before completion.
        store.issue_finite_private_api_key(crate::IssueFinitePrivateApiKeyInput {
            grant_id: grant.id.clone(), raw_key: "fpk_live_metrics".into(),
            project_id: None, agent_runtime_id: None, now: None,
        }).await.unwrap();
        let response = router(store.clone(), scoped_test_auth())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/finite-private/v1/request-diagnostics")
                    .header("authorization", usage_authorization())
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&input).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(
            axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .is_empty()
        );
        let first = db
            .query_json(
                "SELECT to_jsonb(d) FROM finite_private_request_diagnostics d WHERE reservation_id=$1",
                &[&reservation_id],
            )
            .await
            .pop()
            .unwrap();
        assert_eq!(first["prompt_tokens"], 11);
        assert_eq!(first["completion_tokens"], 19);
        assert_eq!(first["project_id"], "project-metrics");
        assert_eq!(first["agent_runtime_id"], serde_json::Value::Null);
        // Accounting may advance after diagnostics have been recorded/exported.
        // Replaying diagnostics must not change accounting or the diagnostic payload.
        store.settle_finite_private_reservation(crate::SettleFinitePrivateReservationInput {
            reservation_id: reservation_id.clone(), request_id: "req-metrics-1".into(),
            settlement: crate::FinitePrivateSettlementKind::Actual,
            prompt_tokens: Some(11), completion_tokens: Some(19), usage_units: Some(68),
            usage_formula_version: "2026-05-26.v1".into(), upstream_status: Some(200),
            upstream_error_class: None, now: None,
        }).await.unwrap();
        let mut replacement = input.clone();
        replacement.completion_tokens = Some(20);
        store
            .record_finite_private_request_diagnostic(replacement)
            .await
            .unwrap();
        let retry = db
            .query_json(
                "SELECT to_jsonb(d) FROM finite_private_request_diagnostics d WHERE reservation_id=$1",
                &[&reservation_id],
            )
            .await
            .pop()
            .unwrap();
        assert_eq!(retry, first);

        // An old event remains old when replayed; a retry cannot extend its TTL.
        db.query_json(
            "UPDATE finite_private_request_diagnostics SET observed_at=CURRENT_TIMESTAMP - INTERVAL '8 days' WHERE reservation_id=$1 RETURNING to_jsonb(finite_private_request_diagnostics)",
            &[&reservation_id],
        )
        .await;
        let aged = db
            .query_json(
                "SELECT to_jsonb(d) FROM finite_private_request_diagnostics d WHERE reservation_id=$1",
                &[&reservation_id],
            )
            .await
            .pop()
            .unwrap();
        store
            .record_finite_private_request_diagnostic(input.clone())
            .await
            .unwrap();
        let aged_retry = db
            .query_json(
                "SELECT to_jsonb(d) FROM finite_private_request_diagnostics d WHERE reservation_id=$1",
                &[&reservation_id],
            )
            .await
            .pop()
            .unwrap();
        assert_eq!(aged_retry, aged);

        let mut wrong_request = input.clone();
        wrong_request.request_id = "req-metrics-wrong".into();
        assert!(matches!(
            store.record_finite_private_request_diagnostic(wrong_request).await,
            Err(crate::CoreError::FinitePrivateReservationNotFound)
        ));
        let mut negative_tokens = input;
        negative_tokens.prompt_tokens = Some(-1);
        assert!(matches!(
            store.record_finite_private_request_diagnostic(negative_tokens).await,
            Err(crate::CoreError::InvalidFinitePrivateUsageEstimate)
        ));
        let ledger = db.row("finite_private_reservations", &reservation_id).await.unwrap();
        assert_eq!(ledger["settled_usage_units"], 68);
        assert_eq!(ledger["status"], "settled");

    })
    .await;
}
