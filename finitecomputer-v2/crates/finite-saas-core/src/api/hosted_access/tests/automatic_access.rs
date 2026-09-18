use super::*;
use crate::auth::test_support::{core_auth_with_runner_credentials, runner_credential_config};
use crate::store::runtime_credentials::RuntimeBootstrapCredential;
use crate::store::runtime_credentials::tests::upgrade;

#[tokio::test]
async fn automatic_hosted_access_uses_trusted_host_configuration_and_application_ack() {
    for (configured, enrollment) in [(false, false), (true, false), (false, true), (true, true)] {
        with_isolated_postgres(move |db| async move {
            let creation = requested(&db).await;
            let auth = core_auth_with_runner_credentials(
                "service",
                vec![runner_credential_config(
                    "runner",
                    "runner-secret",
                    "auth-runner",
                    &[crate::RunnerClass::Kata],
                    "auth-host",
                    false,
                )],
                "usage",
            );
            let origins = if configured {
                HostedHermesOrigins::from_json(r#"{"auth-host":"https://agent.example.test"}"#)
                    .unwrap()
            } else {
                HostedHermesOrigins::default()
            };
            let app =
                router_with_hosted_hermes_origins(db.store.clone(), auth, None, origins.clone());
            let existing = if enrollment {
                let runtime = register(&db, &creation).await;
                complete(&db, &creation).await.unwrap();
                Some((runtime, upgrade(&db, &creation).await))
            } else {
                None
            };
            let (path, lease) = match &existing {
                Some((_, upgrade)) => (
                    format!(
                        "/api/core/v1/runtime-control-requests/{}/runtime-credential",
                        upgrade.request.id
                    ),
                    "upgrade-lease",
                ),
                None => (
                    format!("/api/core/v1/agent-creation-requests/{creation}/runtime-credential"),
                    "test-launch-lease",
                ),
            };
            let owner = access_token_with_subject(
                "runtime-auth-user",
                "runtime-auth@finite.test",
                true,
                None,
            );
            let make_request = |token: &str, body: Value| {
                Request::builder()
                    .method("POST")
                    .uri(&path)
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap()
            };
            let body = json!({"runnerId":"auth-runner","leaseToken":lease});
            let denied = app
                .clone()
                .oneshot(make_request(&owner, body.clone()))
                .await
                .unwrap();
            assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
            let forged = app
                .clone()
                .oneshot(make_request(
                    "runner-secret",
                    json!({
                        "runnerId":"auth-runner","leaseToken":lease,"prepareHostedAccess":true,
                    }),
                ))
                .await
                .unwrap();
            assert!(!forged.status().is_success());
            let response = app
                .clone()
                .oneshot(make_request("runner-secret", body))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let issued: RuntimeBootstrapCredential =
                serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                    .unwrap();
            let runtime = match existing {
                Some((runtime, _)) => runtime,
                None => register(&db, &creation).await,
            };
            let access = db
                .hosted_access(&runtime, "runtime-auth-user")
                .await
                .unwrap();
            assert_eq!(access.enabled, configured);
            assert!(
                db.hosted_login(&runtime, "runtime-auth-user", &origins)
                    .await
                    .is_err()
            );
            assert!(
                db.hosted_route_targets_for_host("auth-host")
                    .await
                    .unwrap()
                    .is_empty()
            );
            let agent = runtime_router(db.store.clone(), origins.clone());
            let report = agent
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/core/v1/runtime/hosted-hermes/report")
                        .header("authorization", format!("Bearer {}", issued.secret))
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"generation":1,"status":"applied"}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(report.status(), StatusCode::NO_CONTENT);
            if !enrollment {
                assert!(
                    db.hosted_route_targets_for_host("auth-host")
                        .await
                        .unwrap()
                        .is_empty(),
                    "launch must complete before publication"
                );
                complete(&db, &creation).await.unwrap();
            }
            assert_eq!(
                db.hosted_login(&runtime, "runtime-auth-user", &origins)
                    .await
                    .is_ok(),
                configured
            );
            assert!(
                db.hosted_login(&runtime, "other-user", &origins)
                    .await
                    .is_err()
            );
            assert_eq!(
                db.hosted_route_targets_for_host("auth-host")
                    .await
                    .unwrap()
                    .len(),
                usize::from(configured)
            );
        })
        .await;
    }
}
