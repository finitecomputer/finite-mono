use super::*;

#[tokio::test]
async fn core_api_serves_finite_private_operator_grant_and_key_lifecycle() {
    with_isolated_postgres(|db| async move {
        let app = router(db.store.clone(), test_auth());
        let operator_authorization = format!(
            "Bearer {}",
            access_token_with_subject(
                "operator_finite_private",
                "admin@finite.vip",
                true,
                Some(OPERATOR_ORG_ID),
            )
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/finite-private/grants")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "verifiedEmail": "private@finite.vip",
                            "workosUserId": "user_workos_private",
                            "now": "2026-05-26T12:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/finite-private/grants")
                    .header("authorization", &operator_authorization)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "verifiedEmail": "private@finite.vip",
                            "workosUserId": "user_workos_private",
                            "now": "2026-05-26T12:00:00Z"
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
        let grant: FinitePrivateGrant = serde_json::from_slice(&body).unwrap();
        assert_eq!(grant.status, crate::FinitePrivateGrantStatus::Active);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/finite-private/grants/{}/api-keys",
                        grant.id
                    ))
                    .header("authorization", &operator_authorization)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        // Unscoped: this test covers the operator
                        // grant/key lifecycle, not project scoping, and
                        // never asserted on either id. Real ids would need
                        // a seeded Project and Agent Runtime, since both
                        // columns are foreign keys.
                        serde_json::json!({
                            "rawKey": "fpk_live_old",
                            "now": "2026-05-26T12:01:00Z"
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
        let old_key: FinitePrivateApiKey = serde_json::from_slice(&body).unwrap();
        assert_eq!(old_key.status, crate::FinitePrivateApiKeyStatus::Active);
        assert_eq!(old_key.grant_id, grant.id);
        assert_ne!(old_key.key_hash, "fpk_live_old");

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
                            "requestId": "req-private-admin-before-reset",
                            "presentedApiKey": "fpk_live_old",
                            "endpoint": "/v1/chat/completions",
                            "model": "kimi-k2-6",
                            "estimatedPromptTokens": 50,
                            "estimatedCompletionTokens": 100,
                            "estimatedUsageUnits": 350,
                            "usageFormulaVersion": "2026-05-26.v1",
                            "dashboardUrl": "https://finite.computer/dashboard",
                            "now": "2026-05-26T12:02:00Z"
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

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/finite-private/grants/{}/reset",
                        grant.id
                    ))
                    .header("authorization", &operator_authorization)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "now": "2026-05-26T12:03:00Z"
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
        let reset_grant: FinitePrivateGrant = serde_json::from_slice(&body).unwrap();
        assert_eq!(reset_grant.current_window_used_units, 0);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/finite-private/api-keys/{}/rotate",
                        old_key.id
                    ))
                    .header("authorization", &operator_authorization)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "rawKey": "fpk_live_new",
                            "now": "2026-05-26T12:04:00Z"
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
        let new_key: FinitePrivateApiKey = serde_json::from_slice(&body).unwrap();
        assert_ne!(new_key.id, old_key.id);
        assert_eq!(new_key.status, crate::FinitePrivateApiKeyStatus::Active);
        assert_eq!(new_key.grant_id, grant.id);

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
                            "requestId": "req-private-admin-old-key-denied",
                            "presentedApiKey": "fpk_live_old",
                            "endpoint": "/v1/chat/completions",
                            "model": "kimi-k2-6",
                            "estimatedPromptTokens": 50,
                            "estimatedCompletionTokens": 100,
                            "estimatedUsageUnits": 350,
                            "usageFormulaVersion": "2026-05-26.v1",
                            "dashboardUrl": "https://finite.computer/dashboard",
                            "now": "2026-05-26T12:05:00Z"
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
        assert_eq!(decision.decision, "deny");

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/finite-private/api-keys/{}/revoke",
                        new_key.id
                    ))
                    .header("authorization", &operator_authorization)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "now": "2026-05-26T12:06:00Z"
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
        let revoked_key: FinitePrivateApiKey = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            revoked_key.status,
            crate::FinitePrivateApiKeyStatus::Revoked
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/core/v1/finite-private/admin-audit-events")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token("admin@finite.vip", true, Some(OPERATOR_ORG_ID),)
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
        let audit_events: Vec<FinitePrivateAdminAuditEvent> =
            serde_json::from_slice(&body).unwrap();
        assert!(
            audit_events
                .iter()
                .any(|event| event.action == "finite_private.api_key.rotate")
        );
        let audit_json = serde_json::to_string(&audit_events).unwrap();
        assert!(!audit_json.contains("fpk_live_old"));
        assert!(!audit_json.contains("fpk_live_new"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/core/v1/finite-private/admin-state")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token("admin@finite.vip", true, Some(OPERATOR_ORG_ID),)
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
        let admin_state: crate::FinitePrivateAdminState = serde_json::from_slice(&body).unwrap();
        assert_eq!(admin_state.grants.len(), 1);
        assert_eq!(admin_state.api_keys.len(), 2);
        assert!(
            admin_state.api_keys.iter().any(|key| key.id == old_key.id
                && key.status == crate::FinitePrivateApiKeyStatus::Revoked)
        );
        let admin_state_json = serde_json::to_string(&admin_state).unwrap();
        assert!(!admin_state_json.contains("fpk_live_old"));
        assert!(!admin_state_json.contains("fpk_live_new"));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/finite-private/grants/{}/revoke",
                        grant.id
                    ))
                    .header("authorization", &operator_authorization)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "now": "2026-05-26T12:07:00Z"
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
        let revoked_grant: FinitePrivateGrant = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            revoked_grant.status,
            crate::FinitePrivateGrantStatus::Revoked
        );
    })
    .await;
}

#[tokio::test]
async fn core_api_admin_friend_key_lifecycle_returns_raw_key_exactly_once() {
    with_isolated_postgres(|db| async move {
        let app = admin_router(db.store.clone());
        let admin = operator_identity_headers("admin@finite.vip");

        let (status, issued) = send_json(
            &app,
            "POST",
            "/api/core/v1/admin/finite-private/friend-keys",
            &admin,
            Some(serde_json::json!({
                "email": "Friend@Finite.VIP",
                "now": "2026-05-25T12:00:00Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let raw_key = issued["raw_api_key"].as_str().unwrap().to_string();
        assert!(raw_key.starts_with("fpk_live_"));
        assert_eq!(issued["grant"]["status"], "active");
        assert_eq!(issued["api_key"]["status"], "active");
        assert_ne!(issued["api_key"]["key_hash"], raw_key.as_str());
        assert!(
            issued["raw_api_key_note"]
                .as_str()
                .unwrap()
                .contains("shown once")
        );
        let key_id = issued["api_key"]["id"].as_str().unwrap().to_string();
        let grant_id = issued["grant"]["id"].as_str().unwrap().to_string();

        let (status, assigned) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/finite-private/grants/{grant_id}/limit-profile"),
            &admin,
            Some(serde_json::json!({
                "limitProfileId": crate::FINITE_PRIVATE_5X_LIMIT_PROFILE,
                "now": "2026-05-25T12:30:00Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            assigned["limit_profile_id"],
            crate::FINITE_PRIVATE_5X_LIMIT_PROFILE
        );

        // Core never stores or re-serves the raw key: the whole admin state
        // must not contain it anywhere.
        let (status, admin_state) = send_json(
            &app,
            "GET",
            "/api/core/v1/finite-private/admin-state",
            &admin,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!admin_state.to_string().contains(&raw_key));
        assert_eq!(admin_state["accounts"][0]["email"], "friend@finite.vip");
        assert_eq!(
            admin_state["accounts"][0]["grant"]["limit_profile_id"],
            crate::FINITE_PRIVATE_5X_LIMIT_PROFILE
        );
        assert!(
            admin_state["profiles"]
                .as_array()
                .unwrap()
                .iter()
                .any(|profile| {
                    profile["id"] == crate::FINITE_PRIVATE_5X_LIMIT_PROFILE
                        && profile["burst_limit_units"] == 500_000_000
                })
        );

        // Rotate returns a brand-new one-time raw key and revokes the old key.
        let (status, rotated) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/finite-private/keys/{key_id}/rotate"),
            &admin,
            Some(serde_json::json!({ "now": "2026-05-25T13:00:00Z" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let rotated_raw = rotated["raw_api_key"].as_str().unwrap().to_string();
        assert!(rotated_raw.starts_with("fpk_live_"));
        assert_ne!(rotated_raw, raw_key);
        assert!(rotated.get("grant").is_none());
        let rotated_key_id = rotated["api_key"]["id"].as_str().unwrap().to_string();
        assert_ne!(rotated_key_id, key_id);

        let (_, admin_state) = send_json(
            &app,
            "GET",
            "/api/core/v1/finite-private/admin-state",
            &admin,
            None,
        )
        .await;
        let keys = admin_state["apiKeys"].as_array().unwrap().clone();
        let old_key = keys
            .iter()
            .find(|key| key["id"] == key_id.as_str())
            .unwrap();
        assert_eq!(old_key["status"], "revoked");
        let new_key = keys
            .iter()
            .find(|key| key["id"] == rotated_key_id.as_str())
            .unwrap();
        assert_eq!(new_key["status"], "active");
        assert!(!admin_state.to_string().contains(&rotated_raw));

        // Revoke the rotated key.
        let (status, revoked) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/finite-private/keys/{rotated_key_id}/revoke"),
            &admin,
            Some(serde_json::json!({ "now": "2026-05-25T14:00:00Z" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(revoked["status"], "revoked");

        // Burst window reset mirrors the CLI window-reset semantics.
        let (status, reset) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/finite-private/grants/{grant_id}/window-reset"),
            &admin,
            Some(serde_json::json!({ "now": "2026-05-25T15:00:00Z" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reset["current_window_used_units"], 0);
        // Compare instants, not strings: Postgres renders RFC3339 with
        // microseconds ("2026-05-25T15:00:00.000000Z") where the old
        // in-memory store echoed the request string back verbatim.
        // Both name the same instant, which is what this pins.
        assert_eq!(
            parse_time(reset["current_window_started_at"].as_str().unwrap()).unwrap(),
            parse_time("2026-05-25T15:00:00Z").unwrap(),
        );

        // Unknown ids surface as 404s, and every admin action was audited
        // with the admin as actor.
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/admin/finite-private/grants/missing/window-reset",
            &admin,
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (_, events) = send_json(
            &app,
            "GET",
            "/api/core/v1/finite-private/admin-audit-events",
            &admin,
            None,
        )
        .await;
        let admin_actions = events
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["actor"] == "admin@finite.vip")
            .map(|event| event["action"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        for expected in [
            "finite_private.friend_key.admin_issue",
            "finite_private.api_key.admin_rotate",
            "finite_private.api_key.admin_revoke",
            "finite_private.grant.admin_window_reset",
            "finite_private.grant.admin_assign_limit_profile",
        ] {
            assert!(
                admin_actions.contains(&expected.to_string()),
                "missing audit action {expected}"
            );
        }
    })
    .await;
}
