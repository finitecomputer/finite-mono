use super::*;

#[tokio::test]
async fn launch_code_admin_api_derives_operator_and_returns_plaintext_once() {
    with_isolated_postgres(|db| async move {
        let app = admin_router(db.store.clone());
        let operator_subject = "workos_operator_subject";
        let operator = vec![(
            "authorization".to_string(),
            format!(
                "Bearer {}",
                access_token_with_subject(
                    operator_subject,
                    "admin@finite.vip",
                    true,
                    Some(OPERATOR_ORG_ID),
                )
            ),
        )];
        let (status, issued) = send_json(
            &app,
            "POST",
            "/api/core/v1/admin/launch-code-batches",
            &operator,
            Some(serde_json::json!({
                "name": "Twelve-person training",
                "codeCount": 12,
                "expiresInHours": 24
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(issued["batch"]["code_count"], 12);
        assert_eq!(
            issued["batch"]["created_by_workos_user_id"],
            operator_subject
        );
        let codes = issued["codes"].as_array().unwrap();
        assert_eq!(codes.len(), 12);
        let plaintext = codes[0]["code"].as_str().unwrap().to_string();
        let batch_id = issued["batch"]["id"].as_str().unwrap().to_string();

        let (status, listed) = send_json(
            &app,
            "GET",
            "/api/core/v1/admin/launch-code-batches",
            &operator,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert!(!listed.to_string().contains(&plaintext));
        assert!(listed[0]["codes"][0].get("code").is_none());

        let (status, revoked) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/launch-code-batches/{batch_id}/revoke"),
            &operator,
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            revoked["batch"]["revoked_by_workos_user_id"],
            operator_subject
        );
        assert!(!revoked.to_string().contains(&plaintext));

        let ordinary_user = identity_headers("member@finite.vip", "true");
        for (method, uri, body) in [
            (
                "GET",
                "/api/core/v1/admin/launch-code-batches".to_string(),
                None,
            ),
            (
                "POST",
                "/api/core/v1/admin/launch-code-batches".to_string(),
                Some(serde_json::json!({
                    "name": "Forbidden",
                    "codeCount": 1,
                    "expiresInHours": 24
                })),
            ),
        ] {
            let (status, _) = send_json(&app, method, &uri, &ordinary_user, body).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
        }
    })
    .await;
}

#[tokio::test]
async fn launch_code_plaintext_issuance_response_is_not_cacheable() {
    with_isolated_postgres(|db| async move {
        let app = admin_router(db.store.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/admin/launch-code-batches")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "workos_operator_no_store",
                                "admin@finite.vip",
                                true,
                                Some(OPERATOR_ORG_ID),
                            )
                        ),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "name": "One-time response",
                            "codeCount": 1,
                            "expiresInHours": 24
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("cache-control")
                .and_then(|value| value.to_str().ok()),
            Some("no-store, private")
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let issued: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(issued["codes"][0]["code"].as_str().is_some());
    })
    .await;
}

#[tokio::test]
async fn core_api_admin_endpoints_require_configured_operator_organization() {
    with_isolated_postgres(|db| async move {
        let app = admin_router(db.store.clone());

        // Missing WorkOS access token entirely.
        let (status, _) = send_json(&app, "GET", "/api/core/v1/admin/runtimes", &[], None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // A service credential cannot enter the operator boundary.
        let service = [("authorization".to_string(), "Bearer core-token".to_string())];
        let (status, _) =
            send_json(&app, "GET", "/api/core/v1/admin/runtimes", &service, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // A valid user without an organization is not an operator.
        let (status, body) = send_json(
            &app,
            "GET",
            "/api/core/v1/admin/runtimes",
            &identity_headers("stranger@finite.vip", "true"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body["error"].as_str().unwrap().contains("admin access"));

        // Operator organization membership cannot bypass email verification.
        let (status, _) = send_json(
            &app,
            "GET",
            "/api/core/v1/admin/runtimes",
            &workos_headers("admin@finite.vip", false, Some(OPERATOR_ORG_ID)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // The configured operator organization is accepted.
        let (status, body) = send_json(
            &app,
            "GET",
            "/api/core/v1/admin/runtimes",
            &operator_identity_headers("admin@finite.vip"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.as_array().unwrap().is_empty());

        // Account Auth's operator organization is never reused as a Core
        // Customer Organization, even when an operator uses a user route.
        let operator = operator_identity_headers("admin@finite.vip");
        let (status, _) = send_json(&app, "GET", "/api/core/v1/me", &operator, None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, billing) =
            send_json(&app, "GET", "/api/core/v1/me/billing", &operator, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_ne!(billing["customer_org"]["id"], OPERATOR_ORG_ID);

        // Every mutating admin endpoint rejects a valid user without the
        // configured operator organization.
        for (method, uri, body) in [
            (
                "GET",
                "/api/core/v1/finite-private/admin-audit-events".to_string(),
                serde_json::json!({}),
            ),
            (
                "GET",
                "/api/core/v1/finite-private/admin-state".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/finite-private/grants".to_string(),
                serde_json::json!({ "verifiedEmail": "friend@finite.vip" }),
            ),
            (
                "POST",
                "/api/core/v1/finite-private/grants/grant_x/api-keys".to_string(),
                serde_json::json!({ "rawKey": "test-key-never-stored" }),
            ),
            (
                "POST",
                "/api/core/v1/finite-private/grants/grant_x/revoke".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/finite-private/grants/grant_x/reset".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/finite-private/api-keys/key_x/revoke".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/finite-private/api-keys/key_x/rotate".to_string(),
                serde_json::json!({ "rawKey": "replacement-test-key-never-stored" }),
            ),
            (
                "GET",
                "/api/core/v1/admin/launch-code-batches".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/admin/launch-code-batches".to_string(),
                serde_json::json!({
                    "name": "Forbidden",
                    "codeCount": 1,
                    "expiresInHours": 24
                }),
            ),
            (
                "POST",
                "/api/core/v1/admin/launch-code-batches/batch_x/revoke".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/admin/projects/project_x/runtime/restart".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/admin/projects/project_x/runtime/recover-known-good-chat".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/admin/projects/project_x/runtime/upgrade".to_string(),
                serde_json::json!({ "targetRuntimeArtifactId": "artifact-v2" }),
            ),
            (
                "POST",
                "/api/core/v1/admin/finite-private/friend-keys".to_string(),
                serde_json::json!({ "email": "friend@finite.vip" }),
            ),
            (
                "POST",
                "/api/core/v1/admin/finite-private/keys/key_x/rotate".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/admin/finite-private/keys/key_x/revoke".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/admin/finite-private/grants/grant_x/window-reset".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/admin/finite-private/grants/grant_x/limit-profile".to_string(),
                serde_json::json!({
                    "limitProfileId": crate::FINITE_PRIVATE_5X_LIMIT_PROFILE
                }),
            ),
        ] {
            for headers in [
                identity_headers("stranger@finite.vip", "true"),
                workos_headers("member@finite.vip", true, Some("workos_org_not_operator")),
            ] {
                let (status, _) = send_json(&app, method, &uri, &headers, Some(body.clone())).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{uri} must be admin-gated");
            }
        }

        // A different WorkOS organization fails closed as well.
        let (status, _) = send_json(
            &app,
            "GET",
            "/api/core/v1/admin/runtimes",
            &workos_headers("admin@finite.vip", true, Some("workos_org_customer")),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}
