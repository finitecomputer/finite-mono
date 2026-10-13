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

async fn linked(store: &TestDb, email: &str, workos_user_id: &str) -> CoreUser {
    store
        .link_verified_user(LinkVerifiedUserInput {
            verified_email: email.to_string(),
            workos_user_id: workos_user_id.to_string(),
            now: Some("2026-10-01T00:00:00Z".to_string()),
        })
        .await
        .unwrap()
}

fn on(at: &str) -> Option<FrontDoorPreference> {
    Some(FrontDoorPreference {
        use_new_finite: true,
        migrated_at: Some(at.to_string()),
    })
}

fn off() -> Option<FrontDoorPreference> {
    Some(FrontDoorPreference {
        use_new_finite: false,
        migrated_at: None,
    })
}

#[tokio::test]
async fn front_door_preference_sets_and_clears_idempotently_on_one_row() {
    with_isolated_postgres(|store| async move {
        let user = linked(&store, "moving-owner@finite.vip", "user_store_front_door").await;
        let bystander = linked(&store, "staying-owner@finite.vip", "user_store_staying").await;
        let set = |use_new_finite: bool, at: &str| {
            store.set_front_door_preference(
                "user_store_front_door",
                use_new_finite,
                Some(at.to_string()),
            )
        };

        // Clearing an unset preference changes nothing.
        assert_eq!(set(false, "2026-10-18T09:00:00Z").await.unwrap(), off());
        assert_eq!(
            user_timestamps(&store, &user.id).await,
            serde_json::json!({ "migrated_at": null, "updated_at": "2026-10-01T00:00:00Z" })
        );

        // Turning it on records the time once; repeating keeps the first.
        assert_eq!(
            set(true, "2026-10-19T09:00:00Z").await.unwrap(),
            on("2026-10-19T09:00:00Z")
        );
        assert_eq!(
            set(true, "2026-10-20T09:00:00Z").await.unwrap(),
            on("2026-10-19T09:00:00Z")
        );
        assert_eq!(
            user_timestamps(&store, &user.id).await,
            serde_json::json!({
                "migrated_at": "2026-10-19T09:00:00Z",
                "updated_at": "2026-10-19T09:00:00Z",
            })
        );
        assert_eq!(
            store
                .visible_projects_and_migration_for_workos_user("user_store_front_door")
                .await
                .unwrap()
                .1
                .as_deref(),
            Some("2026-10-19T09:00:00Z")
        );

        // Turning it off clears it; repeating changes nothing further.
        assert_eq!(set(false, "2026-10-21T09:00:00Z").await.unwrap(), off());
        assert_eq!(set(false, "2026-10-22T09:00:00Z").await.unwrap(), off());
        assert_eq!(
            user_timestamps(&store, &user.id).await,
            serde_json::json!({ "migrated_at": null, "updated_at": "2026-10-21T09:00:00Z" })
        );
        // A later opt-in starts a fresh timestamp.
        assert_eq!(
            set(true, "2026-10-23T09:00:00Z").await.unwrap(),
            on("2026-10-23T09:00:00Z")
        );

        // Row-scoped: the other account is untouched throughout.
        assert_eq!(
            user_timestamps(&store, &bystander.id).await,
            serde_json::json!({ "migrated_at": null, "updated_at": "2026-10-01T00:00:00Z" })
        );
    })
    .await;
}

#[tokio::test]
async fn front_door_preference_never_invents_an_account() {
    with_isolated_postgres(|store| async move {
        for use_new_finite in [true, false] {
            assert_eq!(
                store
                    .set_front_door_preference("user_never_linked", use_new_finite, None)
                    .await
                    .unwrap(),
                None
            );
        }
        assert!(store.all("users").await.is_empty());
        assert_eq!(
            store
                .visible_projects_and_migration_for_workos_user("user_never_linked")
                .await
                .unwrap(),
            (Vec::new(), None)
        );
        assert!(matches!(
            store.set_front_door_preference("   ", true, None).await,
            Err(CoreError::MissingWorkosUserId)
        ));
    })
    .await;
}
