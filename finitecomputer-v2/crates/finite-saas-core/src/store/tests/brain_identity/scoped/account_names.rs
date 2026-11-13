use super::*;

#[tokio::test]
async fn scoped_account_names_follow_account_and_keep_mapping_scope_and_provenance() {
    with_isolated_postgres(|db| async move {
        let sam = setup(&db).await;
        let input = manifest(
            "named_account",
            vec![record(
                1,
                ScopedRecordKind::AccountChatKey,
                Some(&sam),
                None,
            )],
        );
        run(&db, &input).await.unwrap();
        let before = db.all("brain_scoped_records").await;
        let first = page(&db, BRAIN, &[1], &[]).await.remove(0);
        assert_eq!(first.account_email.as_deref(), Some("sam@example.org"));
        assert!(first.display_name.is_none());
        for name in [Some("Sam Rivera"), Some("Sam Renamed"), None] {
            db.link_verified_user_with_name(
                LinkVerifiedUserInput {
                    verified_email: "sam@example.org".into(),
                    workos_user_id: "workos_sam_example_org".into(),
                    now: None,
                },
                name,
            )
            .await
            .unwrap();
            let row = page(&db, BRAIN, &[1], &[]).await.remove(0);
            assert_eq!(row.state, DescriptionState::Resolved);
            assert_eq!(row.display_name.as_deref(), name);
            assert_eq!(row.account_email, first.account_email);
            assert_eq!(row.source, first.source);
            assert_eq!(row.operator_record, first.operator_record);
            assert_eq!(
                page(&db, "other_brain", &[1], &[]).await[0].state,
                DescriptionState::NotShared
            );
        }
        assert_eq!(db.all("brain_scoped_records").await, before);
    })
    .await;
}
