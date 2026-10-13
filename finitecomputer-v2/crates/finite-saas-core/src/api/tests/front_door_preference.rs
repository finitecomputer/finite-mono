use super::*;

const PREFERENCE_URI: &str = "/api/core/v1/me/front-door-preference";

/// Sign an account in through `/me`, which links it like the dashboard does.
async fn linked_account(app: &Router, email: &str) -> (Vec<(String, String)>, serde_json::Value) {
    let headers = identity_headers(email, "true");
    let (status, me) = send_json(app, "GET", "/api/core/v1/me", &headers, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    (headers, me)
}

async fn set(
    app: &Router,
    headers: &[(String, String)],
    use_new_finite: bool,
) -> (StatusCode, serde_json::Value) {
    send_json(
        app,
        "PUT",
        PREFERENCE_URI,
        headers,
        Some(serde_json::json!({ "useNewFinite": use_new_finite })),
    )
    .await
}

async fn marked_emails(db: &crate::test_support::TestDb) -> Vec<serde_json::Value> {
    db.query_json(
        "SELECT to_jsonb(normalized_email) FROM users
         WHERE migrated_at IS NOT NULL ORDER BY normalized_email",
        &[],
    )
    .await
}

#[tokio::test]
async fn front_door_preference_sets_and_clears_only_the_callers_row() {
    with_isolated_postgres(|db| async move {
        let app = router(db.store.clone(), scoped_test_auth());
        let (owner, me) = linked_account(&app, "moving-owner@finite.vip").await;
        assert_eq!(me["migrated_at"], serde_json::Value::Null);
        let (bystander, _) = linked_account(&app, "staying-owner@finite.vip").await;

        let (status, first) = set(&app, &owner, true).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["use_new_finite"], true);
        let migrated_at = first["migrated_at"].as_str().unwrap().to_string();
        parse_time(&migrated_at).expect("migrated_at is RFC3339");
        // Idempotent: turning it on again keeps the first timestamp.
        let (status, again) = set(&app, &owner, true).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(again, first);

        let (_, me) = send_json(&app, "GET", "/api/core/v1/me", &owner, None).await;
        assert_eq!(me["migrated_at"], migrated_at.as_str());
        let (status, summary) = send_json(
            &app,
            "GET",
            "/api/core/v1/me/dashboard-summary",
            &owner,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{summary}");
        assert_eq!(summary["me"]["migrated_at"], migrated_at.as_str());
        let (_, other) = send_json(&app, "GET", "/api/core/v1/me", &bystander, None).await;
        assert_eq!(other["migrated_at"], serde_json::Value::Null);
        assert_eq!(
            marked_emails(&db).await,
            vec![serde_json::json!("moving-owner@finite.vip")]
        );

        // Clearing is the caller's own, idempotent choice too.
        for _ in 0..2 {
            let (status, cleared) = set(&app, &owner, false).await;
            assert_eq!(status, StatusCode::OK, "{cleared}");
            assert_eq!(
                cleared,
                serde_json::json!({ "use_new_finite": false, "migrated_at": null })
            );
        }
        let (_, me) = send_json(&app, "GET", "/api/core/v1/me", &owner, None).await;
        assert_eq!(me["migrated_at"], serde_json::Value::Null);
        assert!(marked_emails(&db).await.is_empty());

        // The bystander turning theirs on leaves the owner's row alone.
        let (status, _) = set(&app, &bystander, true).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            marked_emails(&db).await,
            vec![serde_json::json!("staying-owner@finite.vip")]
        );
    })
    .await;
}

#[tokio::test]
async fn front_door_preference_requires_the_callers_own_session() {
    with_isolated_postgres(|db| async move {
        let app = router(db.store.clone(), scoped_test_auth());
        let bearer = |value: String| vec![("authorization".to_string(), value)];
        let spoofed = vec![
            (
                WORKOS_USER_ID_HEADER.to_string(),
                "user_spoofed".to_string(),
            ),
            (
                WORKOS_EMAIL_HEADER.to_string(),
                "spoofed@finite.vip".to_string(),
            ),
            (WORKOS_EMAIL_VERIFIED_HEADER.to_string(), "true".to_string()),
        ];
        for (name, headers, expected) in [
            ("anonymous", Vec::new(), StatusCode::UNAUTHORIZED),
            (
                "service",
                bearer(format!("Bearer {TOKEN}")),
                StatusCode::UNAUTHORIZED,
            ),
            (
                "Runner",
                bearer(runner_authorization()),
                StatusCode::UNAUTHORIZED,
            ),
            (
                "usage",
                bearer(usage_authorization()),
                StatusCode::UNAUTHORIZED,
            ),
            (
                "spoofed identity headers",
                spoofed,
                StatusCode::UNAUTHORIZED,
            ),
            (
                "unverified email",
                identity_headers("unverified@finite.vip", "false"),
                StatusCode::FORBIDDEN,
            ),
        ] {
            for use_new_finite in [true, false] {
                let (status, response) = set(&app, &headers, use_new_finite).await;
                assert_eq!(status, expected, "{name} caller: {response}");
            }
        }
        assert!(marked_emails(&db).await.is_empty());
        assert!(
            db.all("users").await.is_empty(),
            "no account is linked or created"
        );
    })
    .await;
}

#[tokio::test]
async fn front_door_preference_rejects_malformed_requests_after_authentication() {
    with_isolated_postgres(|db| async move {
        let app = router(db.store.clone(), scoped_test_auth());
        let (owner, _) = linked_account(&app, "malformed-owner@finite.vip").await;
        for malformed in [
            serde_json::json!({}),
            serde_json::json!([]),
            serde_json::json!({ "useNewFinite": "true" }),
            serde_json::json!({ "useNewFinite": null }),
            // The caller can never choose the recorded time or another account.
            serde_json::json!({ "useNewFinite": true, "migratedAt": "2020-01-01T00:00:00Z" }),
            serde_json::json!({ "useNewFinite": true, "workosUserId": "user_someone_else" }),
        ] {
            let (status, response) =
                send_json(&app, "PUT", PREFERENCE_URI, &owner, Some(malformed.clone())).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{malformed}: {response}");
        }
        assert!(marked_emails(&db).await.is_empty());
    })
    .await;
}
