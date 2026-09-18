use super::*;
mod automatic_access;
use crate::auth::test_support::{access_token_with_subject, core_auth};
use crate::store::runtime_credentials::tests::{complete, provision, register, requested};
use crate::test_support::with_isolated_postgres;
use axum::body::{Body, to_bytes};
use axum::http::Request;
use tower::ServiceExt;

#[tokio::test]
async fn hosted_route_targets_are_runner_host_scoped_and_credential_free() {
    use crate::auth::test_support::{core_auth_with_runner_credentials, runner_credential_config};
    use crate::store::hosted_hermes::ApplyStatus;
    use crate::store::runtime_credentials::new_secret;
    with_isolated_postgres(|db| async move {
        let request = requested(&db).await;
        let secret = db.provision_runtime_credential(provision(&request)).await.unwrap().secret;
        let runtime = register(&db, &request).await;
        let project = complete(&db, &request).await.unwrap().project.id;
        db.set_hosted_access(&runtime, "runtime-auth-user", SetHostedAccess { enabled: true, expected_generation: 1 }).await.unwrap();
        db.report_hosted(&secret, HostedReport { generation: 2, status: ApplyStatus::Applied }).await.unwrap();
        let runner_token = new_secret().unwrap();
        let other_host_token = new_secret().unwrap();
        let revoked_token = new_secret().unwrap();
        let service_token = new_secret().unwrap();
        let auth = core_auth_with_runner_credentials(&service_token, vec![
            runner_credential_config("runner", &runner_token, "auth-runner", &[crate::RunnerClass::Kata], "auth-host", false),
            runner_credential_config("other", &other_host_token, "other-runner", &[crate::RunnerClass::Kata], "other-host", false),
            runner_credential_config("revoked", &revoked_token, "old-runner", &[crate::RunnerClass::Kata], "auth-host", true),
        ], new_secret().unwrap());
        let origins = HostedHermesOrigins::from_json(r#"{"auth-host":"https://agent.example.test"}"#).unwrap();
        let app = router_with_hosted_hermes_origins(db.store.clone(), auth.clone(), None, origins.clone());
        let owner = access_token_with_subject("runtime-auth-user", "runtime-auth@finite.test", true, None);
        let path = "/api/core/v1/hosted-hermes-route-targets";
        for token in [&owner, &secret, &service_token, &revoked_token, ""] {
            let response = app.clone().oneshot(Request::builder().uri(path)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        for (token, expected) in [
            (&runner_token, json!([{
                "runtimeId":runtime, "projectId":project, "sourceMachineId":"auth-machine", "generation":2,
            }])),
            (&other_host_token, json!([])),
        ] {
            let response = app.clone().oneshot(Request::builder()
                // Caller-supplied placement cannot expand the credential's scope.
                .uri(format!("{path}?sourceHostId=auth-host"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["cache-control"], "no-store");
            let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 32768).await.unwrap()).unwrap();
            // Exact equality also excludes native and bootstrap credentials.
            assert_eq!(body, expected);
        }
        let unconfigured = router_with_hosted_hermes_origins(db.store.clone(), auth, None, HostedHermesOrigins::default());
        let response = unconfigured.oneshot(Request::builder().uri(path)
            .header("authorization", format!("Bearer {runner_token}"))
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 32768).await.unwrap()).unwrap();
        assert_eq!(body, json!([]));
        let response = runtime_router(db.store.clone(), origins).oneshot(Request::builder().uri(path)
            .header("authorization", format!("Bearer {runner_token}"))
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }).await;
}

#[tokio::test]
async fn hosted_api_allows_nonadmin_owner_and_separates_runtime_listener() {
    with_isolated_postgres(|db| async move {
        let request = requested(&db).await;
        let secret = db
            .provision_runtime_credential(provision(&request))
            .await
            .unwrap()
            .secret;
        let runtime = register(&db, &request).await;
        complete(&db, &request).await.unwrap();
        let origins =
            HostedHermesOrigins::from_json(r#"{"auth-host":"https://agent.example.test"}"#)
                .unwrap();
        let app = router_with_hosted_hermes_origins(
            db.store.clone(),
            core_auth("service", "runner", "usage"),
            None,
            origins.clone(),
        );
        let agents = runtime_router(db.store.clone(), origins);
        let owner =
            access_token_with_subject("runtime-auth-user", "runtime-auth@finite.test", true, None);
        let other = access_token_with_subject("other-user", "other@finite.test", true, None);
        let path = format!("/api/core/v1/me/runtimes/{runtime}/hosted-access");
        for (token, status) in [
            (&owner, StatusCode::OK),
            (&other, StatusCode::NOT_FOUND),
            (&secret, StatusCode::UNAUTHORIZED),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(&path)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status);
        }
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(&path)
                    .header("authorization", format!("Bearer {owner}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"enabled":true,"expectedGeneration":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 32768).await.unwrap()).unwrap();
        assert_eq!(body["enabled"], true);
        assert!(body.get("password").is_none());
        let pull = "/api/core/v1/runtime/hosted-hermes";
        for (router, path, token, status) in [
            (&agents, pull, &secret, StatusCode::OK),
            (&agents, pull, &owner, StatusCode::UNAUTHORIZED),
            (&app, pull, &secret, StatusCode::NOT_FOUND),
            (&agents, path.as_str(), &owner, StatusCode::NOT_FOUND),
            (
                &agents,
                "/api/core/v1/agent-creation-requests",
                &secret,
                StatusCode::NOT_FOUND,
            ),
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status);
        }
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/me/runtimes/{runtime}/hosted-hermes-session"
                    ))
                    .header("authorization", format!("Bearer {owner}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT); // application has not reported
    })
    .await;
}
