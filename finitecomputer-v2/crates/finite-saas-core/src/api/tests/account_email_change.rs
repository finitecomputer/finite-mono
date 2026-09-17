use super::*;

#[tokio::test]
async fn account_email_change_api_requires_operator_and_fresh_verified_same_subject() {
    with_isolated_postgres(|db| async move {
        let subject = "user_email_route_fixture";
        let user = db
            .link_verified_user(LinkVerifiedUserInput {
                verified_email: "route-before@example.test".into(),
                workos_user_id: subject.into(),
                now: None,
            })
            .await
            .unwrap();
        let request = json!({
            "operationId": "route-email-fixture", "userId": user.id,
            "workosUserId": subject, "expectedEmail": "route-before@example.test",
            "newEmail": "route-after@example.test", "evidenceReference": "private-route-fixture",
        });
        let app = admin_router(db.store.clone());
        let operator = operator_identity_headers("operator-email@example.test");
        for action in ["preview", "prepare", "complete", "cancel"] {
            let path = format!("/api/core/v1/admin/account-email-changes/{action}");
            for headers in [
                vec![],
                vec![("authorization".into(), "Bearer core-token".into())],
                identity_headers("ordinary-email@example.test", "true"),
            ] {
                let (status, _) =
                    send_json(&app, "POST", &path, &headers, Some(request.clone())).await;
                assert!(matches!(
                    status,
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
                ));
            }
        }
        for headers in [
            vec![],
            identity_headers("ordinary-email@example.test", "true"),
        ] {
            let (status, _) = send_json(
                &app,
                "POST",
                "/api/core/v1/admin/account-email-target",
                &headers,
                Some(json!({"email": "route-before@example.test"})),
            )
            .await;
            assert!(matches!(
                status,
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            ));
            let (status, _) = send_json(
                &app,
                "GET",
                "/api/core/v1/admin/account-email-changes/route-email-fixture",
                &headers,
                None,
            )
            .await;
            assert!(matches!(
                status,
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            ));
        }
        let (status, target) = send_json(
            &app,
            "POST",
            "/api/core/v1/admin/account-email-target",
            &operator,
            Some(json!({"email": "route-before@example.test"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(target["userId"], user.id);
        assert_eq!(target["workosUserId"], subject);
        let _ = access_token_with_subject(subject, "route-before@example.test", true, None);
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/admin/account-email-changes/prepare",
            &operator,
            Some(request.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/admin/account-email-changes/complete",
            &operator,
            Some(request.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        // Merely asserting the destination in the request cannot substitute
        // for the provider's verification of that same subject.
        let _ = access_token_with_subject(subject, "route-after@example.test", false, None);
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/admin/account-email-changes/complete",
            &operator,
            Some(request.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let _ = access_token_with_subject(subject, "route-after@example.test", true, None);
        let (status, result) = send_json(
            &app,
            "POST",
            "/api/core/v1/admin/account-email-changes/complete",
            &operator,
            Some(request),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["status"], "completed");
        let (status, receipt) = send_json(
            &app,
            "GET",
            "/api/core/v1/admin/account-email-changes/route-email-fixture",
            &operator,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(receipt["status"], "completed");
        assert_eq!(receipt["request"]["userId"], user.id);
        assert_eq!(receipt["request"]["newEmail"], "route-after@example.test");

        assert_eq!(
            db.row("users", &user.id).await.unwrap()["normalized_email"],
            "route-after@example.test"
        );
    })
    .await;
}
