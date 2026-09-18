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
            observed_at: Some("2026-09-17T12:01:01Z".to_string()),
        };
        // Existing key issuance can update the association before completion.
        store.issue_finite_private_api_key(crate::IssueFinitePrivateApiKeyInput {
            grant_id: grant.id.clone(), raw_key: "fpk_live_metrics".into(),
            project_id: None, agent_runtime_id: None, now: None,
        }).await.unwrap();
        store
            .record_finite_private_request_diagnostic(input.clone())
            .await
            .unwrap();
        // Accounting may advance after diagnostics have been recorded/exported.
        // Replaying diagnostics must not change either its payload or its TTL.
        let first = store.finite_private_request_diagnostics(10).await.unwrap().remove(0);
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
        let details = store.finite_private_request_diagnostics(10).await.unwrap();
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].reservation_id, reservation_id);
        // The first accepted diagnostic is immutable; a retry cannot rewrite
        // an event that may already have been exported to Loki.
        assert_eq!(details[0].completion_tokens, Some(19));
        assert_eq!(details[0].model, "glm-5-3-flash");
        assert_eq!(details[0], first);
        assert_eq!(details[0].project_id.as_deref(), Some("project-metrics"));
        let second = store.reserve_finite_private_usage(crate::ReserveFinitePrivateUsageInput {
            request_id: "req-metrics-2".into(), presented_api_key: "fpk_live_metrics".into(),
            endpoint: "/v1/chat/completions".into(), model: "glm-5-3-flash".into(),
            estimated_prompt_tokens: 10, estimated_completion_tokens: 20, estimated_usage_units: 70,
            usage_formula_version: "2026-05-26.v1".into(), dashboard_url: "https://dashboard.invalid".into(), now: None,
        }).await.unwrap();
        let mut second_input = input.clone();
        second_input.reservation_id = second.reservation_id.unwrap();
        second_input.request_id = "req-metrics-2".into();
        store.record_finite_private_request_diagnostic(second_input).await.unwrap();
        let page = store.finite_private_request_diagnostics_page(crate::FinitePrivateRequestDiagnosticQuery { limit: Some(1), model: Some("glm-5-3-flash".into()), ..Default::default() }).await.unwrap();
        assert_eq!(page.items.len(), 1);
        assert!(page.truncated);
        let next = store.finite_private_request_diagnostics_page(crate::FinitePrivateRequestDiagnosticQuery {
            limit: Some(1), before_observed_at: page.next_before_observed_at,
            before_reservation_id: page.next_before_reservation_id, model: Some("glm-5-3-flash".into()), ..Default::default() }).await.unwrap();
        assert_eq!(next.items.len(), 1);
        assert!(!next.truncated);
        assert_ne!(next.items[0].reservation_id, page.items[0].reservation_id);
        assert!(store.finite_private_request_diagnostics_page(crate::FinitePrivateRequestDiagnosticQuery {
            api_key_id: Some("unrelated-key".into()), ..Default::default() }).await.unwrap().items.is_empty());
        assert!(store.finite_private_request_diagnostics_page(crate::FinitePrivateRequestDiagnosticQuery {
            project_id: Some("unrelated-project".into()), ..Default::default() }).await.unwrap().items.is_empty());
        // Age only the synthetic diagnostic, not its durable accounting record.
        db.query_json("WITH aged AS (UPDATE finite_private_request_diagnostics SET observed_at = NOW() - INTERVAL '8 days' WHERE reservation_id=$1 RETURNING reservation_id) SELECT to_jsonb(aged) FROM aged", &[&reservation_id]).await;
        assert_eq!(store.finite_private_request_diagnostics(10).await.unwrap().len(), 1);
        let ledger = db.row("finite_private_reservations", &reservation_id).await.unwrap();
        assert_eq!(ledger["settled_usage_units"], 68);
        assert_eq!(ledger["status"], "settled");

    })
    .await;
}

#[tokio::test]
async fn finite_private_request_details_require_operator_identity() {
    with_isolated_postgres(|db| async move {
        let app = admin_router(db.store.clone());
        for headers in [vec![], identity_headers("member@finite.vip", "true")] {
            let (status, _) = send_json(
                &app,
                "GET",
                "/api/core/v1/finite-private/admin-request-details",
                &headers,
                None,
            )
            .await;
            assert!(status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN);
        }
        let headers = vec![(
            "authorization".into(),
            format!(
                "Bearer {}",
                access_token_with_subject(
                    "operator-request-reporting",
                    "admin@finite.vip",
                    true,
                    Some(OPERATOR_ORG_ID)
                )
            ),
        )];
        let (status, body) = send_json(
            &app,
            "GET",
            "/api/core/v1/finite-private/admin-request-details?limit=1",
            &headers,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["coverage"], "core-reserved-only");
        assert_eq!(body["retentionDays"], 7);
        assert_eq!(body["items"], serde_json::json!([]));
    })
    .await;
}
