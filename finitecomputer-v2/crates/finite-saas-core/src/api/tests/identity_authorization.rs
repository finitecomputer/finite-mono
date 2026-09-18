use super::*;

#[tokio::test]
async fn core_api_rejects_spoofed_legacy_identity_headers() {
    with_isolated_postgres(|db| async move {
        let app = router(db.store.clone(), test_auth());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_test")
                    .header(WORKOS_EMAIL_HEADER, "test@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
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
async fn core_api_rejects_mismatched_identity_headers_even_with_valid_jwt() {
    with_isolated_postgres(|db| async move {
        let app = router(db.store.clone(), test_auth());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject("user_real", "real@finite.vip", true, None,)
                        ),
                    )
                    .header(WORKOS_USER_ID_HEADER, "user_spoofed")
                    .header(WORKOS_EMAIL_HEADER, "spoofed@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
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
async fn route_scoped_credentials_cannot_cross_user_admin_or_runner_boundaries() {
    with_isolated_postgres(|db| async move {
        let app = router(db.store.clone(), scoped_test_auth());
        let runner = vec![("authorization".to_string(), boundary_runner_authorization())];
        let service = vec![("authorization".to_string(), "Bearer core-token".to_string())];
        let usage = vec![("authorization".to_string(), usage_authorization())];

        for uri in ["/api/core/v1/me", "/api/core/v1/admin/runtimes"] {
            for (credential, headers) in [
                ("service", &service),
                ("Runner", &runner),
                ("usage", &usage),
            ] {
                let (status, _) = send_json(&app, "GET", uri, headers, None).await;
                assert_eq!(
                    status,
                    StatusCode::UNAUTHORIZED,
                    "{credential} credential entered {uri}"
                );
            }
        }

        let runner_routes = [
            (
                "/api/core/v1/agent-creation-requests/lease",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary",
                    "leaseSeconds": 60,
                    "runnerCapacity": runner_capacity_json(RunnerClass::Kata)
                }),
            ),
            (
                "/api/core/v1/runtime-control-requests/lease",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary",
                    "leaseSeconds": 60,
                    "sourceHostId": "source-auth-boundary",
                    "runnerCapacity": runner_capacity_json(RunnerClass::Kata)
                }),
            ),
            (
                "/api/core/v1/runtime-control-requests/missing/complete",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary"
                }),
            ),
            (
                "/api/core/v1/runtime-control-requests/missing/fail",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary",
                    "failureMessage": "boundary test"
                }),
            ),
            (
                "/api/core/v1/agent-creation-requests/missing/complete",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary",
                    "sourceHostId": "source-auth-boundary",
                    "sourceMachineId": "machine-auth-boundary",
                    "publishedAppUrls": []
                }),
            ),
            (
                "/api/core/v1/agent-creation-requests/missing/runtime",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary",
                    "sourceHostId": "source-auth-boundary",
                    "sourceMachineId": "machine-auth-boundary",
                    "runtimeRelayTokenHash": "hash-auth-boundary",
                    "publishedAppUrls": []
                }),
            ),
            (
                "/api/core/v1/agent-creation-requests/missing/finite-private-key",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary"
                }),
            ),
            (
                "/api/core/v1/agent-creation-requests/missing/fail",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary",
                    "failureMessage": "boundary test"
                }),
            ),
        ];
        for (uri, body) in &runner_routes {
            for (credential, headers) in [("service", &service), ("usage", &usage)] {
                let (status, _) = send_json(&app, "POST", uri, headers, Some(body.clone())).await;
                assert_eq!(
                    status,
                    StatusCode::UNAUTHORIZED,
                    "{credential} credential entered {uri}"
                );
            }
        }

        // A service-token-only route: auth is checked before the missing
        // creation request is rejected, so the wrong credential sees 401
        // and the right one sees the post-auth lookup error instead.
        for headers in [&runner, &usage] {
            let (status, _) = send_json(
                &app,
                "POST",
                "/api/core/v1/agent-creation-requests/missing/cancel",
                headers,
                Some(serde_json::json!({})),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/agent-creation-requests/missing/cancel",
            &service,
            Some(serde_json::json!({})),
        )
        .await;
        assert_ne!(status, StatusCode::UNAUTHORIZED);

        let (status, _) = send_json(
            &app,
            "PUT",
            "/api/core/v1/runtime-artifacts/artifact-auth-boundary",
            &service,
            Some(serde_json::json!({
                "kind": "oci_image",
                "reference": "ghcr.io/finitecomputer/agent-runtime@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "versionLabel": "auth-boundary",
                "stateSchemaVersion": "state-v1",
                "baseImage": "python:3.13-trixie",
                "promoted": true
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        for headers in [&service, &usage] {
            let (status, _) = send_json(
                &app,
                "GET",
                "/api/core/v1/runtime-artifacts/artifact-auth-boundary",
                headers,
                None,
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let (status, artifact) = send_json(
            &app,
            "GET",
            "/api/core/v1/runtime-artifacts/artifact-auth-boundary",
            &runner,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(artifact["id"], "artifact-auth-boundary");

        for headers in [&runner, &usage] {
            let (status, _) = send_json(
                &app,
                "PUT",
                "/api/core/v1/runtime-artifacts/forbidden-artifact",
                headers,
                Some(serde_json::json!({
                    "kind": "oci_image",
                    "reference": "ghcr.io/finitecomputer/agent-runtime@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "versionLabel": "forbidden",
                    "stateSchemaVersion": "state-v1",
                    "baseImage": "python:3.13-trixie",
                    "promoted": true
                })),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }

        for headers in [&service, &usage] {
            let (status, _) = send_json(
                &app,
                "GET",
                "/api/core/v1/runtime-artifacts/missing",
                headers,
                None,
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let (status, _) = send_json(
            &app,
            "GET",
            "/api/core/v1/runtime-artifacts/missing",
            &runner,
            None,
        )
        .await;
        assert_ne!(status, StatusCode::UNAUTHORIZED);

        for headers in [&service, &runner] {
            let (status, _) = send_json(
                &app,
                "GET",
                "/internal/finite-private/v1/health",
                headers,
                None,
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let (status, _) = send_json(
            &app,
            "GET",
            "/internal/finite-private/v1/health",
            &usage,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send_json(
            &app,
            "POST",
            "/api/core/v1/agent-creation-requests/lease",
            &runner,
            Some(runner_routes[0].1.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.is_null(), "empty Runner queue should return null");
    })
    .await;
}
