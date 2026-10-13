use super::*;

const MARK_URI: &str = "/api/core/v1/account-migrations";

fn migration_token() -> String {
    scoped_token("account-migration")
}

fn migration_auth() -> CoreAuth {
    scoped_test_auth()
        .with_account_migration_api_token(migration_token())
        .expect("a distinct account migration credential is accepted")
}

fn migration_headers() -> Vec<(String, String)> {
    vec![(
        "authorization".to_string(),
        format!("Bearer {}", migration_token()),
    )]
}

/// Sign an account in through `/me`, which links it like the dashboard does.
async fn linked_account(app: &Router, email: &str) -> (Vec<(String, String)>, serde_json::Value) {
    let headers = identity_headers(email, "true");
    let (status, me) = send_json(app, "GET", "/api/core/v1/me", &headers, None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    (headers, me)
}

async fn marked_workos_user_ids(db: &crate::test_support::TestDb) -> Vec<serde_json::Value> {
    db.query_json(
        "SELECT to_jsonb(workos_user_id) FROM users
         WHERE migrated_at IS NOT NULL ORDER BY workos_user_id",
        &[],
    )
    .await
}

#[tokio::test]
async fn account_migration_mark_is_idempotent_and_read_only_in_me() {
    with_isolated_postgres(|db| async move {
        let app = router(db.store.clone(), migration_auth());
        let (owner, me) = linked_account(&app, "moving-owner@finite.vip").await;
        assert_eq!(me["migrated_at"], serde_json::Value::Null);
        let workos_user_id = me["workos_user_id"].as_str().unwrap().to_string();
        let body = serde_json::json!({ "workosUserId": workos_user_id });

        let (status, first) = send_json(
            &app,
            "POST",
            MARK_URI,
            &migration_headers(),
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["workos_user_id"], workos_user_id.as_str());
        assert_eq!(first["newly_marked"], true);
        let migrated_at = first["migrated_at"].as_str().unwrap().to_string();
        parse_time(&migrated_at).expect("migrated_at is RFC3339");

        let (status, replay) =
            send_json(&app, "POST", MARK_URI, &migration_headers(), Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{replay}");
        assert_eq!(replay["newly_marked"], false);
        assert_eq!(replay["migrated_at"], migrated_at.as_str());

        let (status, me) = send_json(&app, "GET", "/api/core/v1/me", &owner, None).await;
        assert_eq!(status, StatusCode::OK, "{me}");
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

        // `/me` is read-only for the flag: the account cannot set it itself,
        // and another account stays unmarked.
        let (_, staying) = linked_account(&app, "staying-owner@finite.vip").await;
        assert_eq!(staying["migrated_at"], serde_json::Value::Null);
        assert_eq!(
            marked_workos_user_ids(&db).await,
            vec![serde_json::json!(workos_user_id)]
        );
    })
    .await;
}

#[tokio::test]
async fn account_migration_route_accepts_only_its_dedicated_credential() {
    with_isolated_postgres(|db| async move {
        let app = router(db.store.clone(), migration_auth());
        let (owner, me) = linked_account(&app, "boundary-owner@finite.vip").await;
        let body = serde_json::json!({ "workosUserId": me["workos_user_id"] });
        let bearer = |value: String| vec![("authorization".to_string(), value)];
        let callers = [
            ("no", Vec::new()),
            ("service", bearer(format!("Bearer {TOKEN}"))),
            ("Runner", bearer(runner_authorization())),
            ("usage", bearer(usage_authorization())),
            ("bare migration", bearer(migration_token())),
            ("WorkOS user", owner),
            (
                "WorkOS operator",
                operator_identity_headers("boundary-operator@finite.vip"),
            ),
        ];
        for (name, headers) in callers {
            let (status, response) =
                send_json(&app, "POST", MARK_URI, &headers, Some(body.clone())).await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{name} credential reached the account migration route: {response}"
            );
        }
        assert!(marked_workos_user_ids(&db).await.is_empty());

        // A deployment without the dedicated credential refuses every caller.
        let unconfigured = router(db.store.clone(), scoped_test_auth());
        for headers in [migration_headers(), bearer(format!("Bearer {TOKEN}"))] {
            let (status, _) = send_json(
                &unconfigured,
                "POST",
                MARK_URI,
                &headers,
                Some(body.clone()),
            )
            .await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        }
        assert!(marked_workos_user_ids(&db).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn account_migration_rejects_unknown_accounts_and_malformed_requests() {
    with_isolated_postgres(|db| async move {
        let app = router(db.store.clone(), migration_auth());
        let (status, response) = send_json(
            &app,
            "POST",
            MARK_URI,
            &migration_headers(),
            Some(serde_json::json!({ "workosUserId": "user_never_signed_in" })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{response}");
        assert!(db.all("users").await.is_empty(), "no account is invented");

        for malformed in [
            serde_json::json!({}),
            serde_json::json!([]),
            serde_json::json!({ "workosUserId": "" }),
            serde_json::json!({ "workosUserId": "   " }),
            serde_json::json!({ "workosUserId": "a".repeat(257) }),
            serde_json::json!({ "workosUserId": "user_\u{0}nul" }),
            // The caller can never choose the recorded time or unset the flag.
            serde_json::json!({ "workosUserId": "user_x", "migratedAt": null }),
            serde_json::json!({ "workosUserId": "user_x", "now": "2020-01-01T00:00:00Z" }),
        ] {
            let (status, response) = send_json(
                &app,
                "POST",
                MARK_URI,
                &migration_headers(),
                Some(malformed.clone()),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{malformed}: {response}");
        }
    })
    .await;
}
