use super::*;

async fn user_timestamps(store: &TestDb, user_id: &str) -> serde_json::Value {
    store
        .query_json(
            "SELECT jsonb_build_object(
                 'migrated_at', core_rfc3339(migrated_at),
                 'updated_at', core_rfc3339(updated_at))
             FROM users WHERE id = $1",
            &[&user_id],
        )
        .await
        .into_iter()
        .next()
        .expect("user row must read back")
}

#[tokio::test]
async fn account_migration_mark_keeps_the_first_timestamp() {
    with_isolated_postgres(|store| async move {
        let user = store
            .link_verified_user(LinkVerifiedUserInput {
                verified_email: "migrating-owner@finite.vip".to_string(),
                workos_user_id: "user_store_migration".to_string(),
                now: Some("2026-10-01T00:00:00Z".to_string()),
            })
            .await
            .unwrap();
        let bystander = store
            .link_verified_user(LinkVerifiedUserInput {
                verified_email: "staying-owner@finite.vip".to_string(),
                workos_user_id: "user_store_staying".to_string(),
                now: Some("2026-10-01T00:00:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(
            store
                .account_migrated_at_for_workos_user("user_store_migration")
                .await
                .unwrap(),
            None
        );

        let first = store
            .mark_account_migrated(
                "user_store_migration",
                Some("2026-10-19T09:00:00Z".to_string()),
            )
            .await
            .unwrap()
            .expect("a linked account is marked");
        assert_eq!(
            first,
            AccountMigrationMark {
                workos_user_id: "user_store_migration".to_string(),
                migrated_at: "2026-10-19T09:00:00Z".to_string(),
                newly_marked: true,
            }
        );

        // A replay (even with a later clock and padded id) never moves the
        // first mark and does not touch the row again.
        let replay = store
            .mark_account_migrated(
                "  user_store_migration  ",
                Some("2026-10-20T09:00:00Z".to_string()),
            )
            .await
            .unwrap()
            .unwrap();
        assert!(!replay.newly_marked);
        assert_eq!(replay.migrated_at, first.migrated_at);
        assert_eq!(
            user_timestamps(&store, &user.id).await,
            serde_json::json!({
                "migrated_at": "2026-10-19T09:00:00Z",
                "updated_at": "2026-10-19T09:00:00Z",
            })
        );
        assert_eq!(
            store
                .account_migrated_at_for_workos_user("user_store_migration")
                .await
                .unwrap()
                .as_deref(),
            Some("2026-10-19T09:00:00Z")
        );

        // Row-scoped: the other account is untouched.
        assert_eq!(
            user_timestamps(&store, &bystander.id).await,
            serde_json::json!({
                "migrated_at": null,
                "updated_at": "2026-10-01T00:00:00Z",
            })
        );
    })
    .await;
}

#[tokio::test]
async fn account_migration_mark_never_invents_an_account() {
    with_isolated_postgres(|store| async move {
        assert_eq!(
            store
                .mark_account_migrated("user_never_linked", None)
                .await
                .unwrap(),
            None
        );
        assert!(store.all("users").await.is_empty());
        assert!(matches!(
            store.mark_account_migrated("   ", None).await,
            Err(CoreError::MissingWorkosUserId)
        ));
    })
    .await;
}
