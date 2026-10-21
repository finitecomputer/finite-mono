use super::*;

#[tokio::test]
async fn scoped_normal_unknown_and_ambiguous_never_fall_back_to_saved_attribution() {
    with_isolated_postgres(|db| async move {
        let sam = setup(&db).await;
        let input = manifest(
            "precedence",
            vec![record(
                1,
                ScopedRecordKind::AccountChatKey,
                Some(&sam),
                None,
            )],
        );
        run(&db, &input).await.unwrap();
        observe(
            &db,
            &sam,
            &human_observation("current_action", &key(1), BRAIN),
        )
        .await
        .unwrap();
        db.exec(&format!(
            "UPDATE users SET normalized_email=repeat('x',255) WHERE id='{sam}'"
        ))
        .await;
        let row = &page(&db, BRAIN, &[1], &[1]).await[0];
        assert_eq!(row.state, DescriptionState::Unknown);
        assert_eq!(
            row.operator_record.as_ref().unwrap().reason,
            Some(OperatorRecordReason::NormalResultPrecedence)
        );
        assert!(row.account_email.is_none() && row.operator_annotation.is_none());
        db.exec(&format!(
            "UPDATE users SET normalized_email='sam@example.org' WHERE id='{sam}'"
        ))
        .await;
        project(&db, "later-pin", &sam, "Conflicting later Agent").await;
        runtime(&db, "later-runtime", "later-pin", &key(1), true, None).await;
        let row = &page(&db, BRAIN, &[1], &[1]).await[0];
        assert_eq!(row.state, DescriptionState::Ambiguous);
        assert_eq!(
            row.operator_record.as_ref().unwrap().reason,
            Some(OperatorRecordReason::IdentityEvidenceConflict)
        );
        assert!(
            row.account_email.is_none()
                && row.responsible_account.is_none()
                && row.operator_annotation.is_none()
        );
        // Manually corrupted durable metadata is not trusted for any row in
        // the batch. The strict whole-batch failure is intentional.
        db.exec("UPDATE brain_scoped_records SET evidence_reference=E'bad\\nreference'")
            .await;
        let request = ScopedDescriptionsRequest {
            version: DESCRIPTIONS_VERSION_V3.into(),
            brain_server: SERVER.into(),
            brain_id: BRAIN.into(),
            requested_by_public_key_hex: key(0xee),
            public_keys_hex: vec![key(1), key(2)],
            participating_public_keys_hex: vec![key(1)],
        };
        assert!(db.describe_scoped_brain_identities(&request).await.is_err());
    })
    .await;
}

#[tokio::test]
async fn scoped_account_locks_allow_foreign_key_activity_but_block_binding_changes() {
    with_isolated_postgres(|db| async move {
        let sam = setup(&db).await;
        let input = manifest("locks", vec![record(1, ScopedRecordKind::AccountChatKey, Some(&sam), None)]);
        let mut blocker = db.connection().await.unwrap();
        let pause = blocker.transaction().await.unwrap();
        // Hold the apply just after it locks accounts, before it inserts its
        // operation. This tests the actual writer's lock, not copied SQL.
        pause.batch_execute("LOCK TABLE brain_scoped_record_operations IN ACCESS EXCLUSIVE MODE").await.unwrap();
        let observer = db.connection().await.unwrap();
        let activity = async {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let row = observer.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE 'INSERT INTO brain_scoped_record_operations%')", &[]).await.unwrap();
                    if row.get::<_, bool>(0) { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }).await.expect("apply reached the account-locked boundary");
            observer.batch_execute("SET lock_timeout='150ms'").await.unwrap();
            observer.execute("INSERT INTO projects(id,customer_org_id,owner_user_id,display_name,hosting_tier,created_at,updated_at) SELECT 'fk-activity',id,$1,'Concurrent account activity','standard',now(),now() FROM customer_orgs WHERE owner_user_id=$1", &[&sam]).await.unwrap();
            let error = observer.execute("UPDATE users SET workos_user_id='changed' WHERE id=$1", &[&sam]).await.unwrap_err();
            assert_eq!(error.code(), Some(&tokio_postgres::error::SqlState::LOCK_NOT_AVAILABLE));
            pause.rollback().await.unwrap();
        };
        let (result, ()) = tokio::join!(run(&db, &input), activity);
        result.unwrap();
        assert_eq!(page(&db, BRAIN, &[1], &[]).await[0].state, DescriptionState::Resolved);
    }).await;
}

#[tokio::test]
async fn scoped_operator_drift_future_evidence_and_retirement_scope_refuse_atomically() {
    with_isolated_postgres(|db| async move {
        let sam = setup(&db).await;
        let input = manifest("guards", vec![record(1, ScopedRecordKind::AccountChatKey, Some(&sam), None)]);
        let mut wrong = input.clone();
        wrong.operator.email = "wrong@example.org".into();
        assert!(run(&db, &wrong).await.is_err());
        wrong = input.clone();
        wrong.records[0].evidence.observed_at = stamp(now() + time::Duration::minutes(1));
        assert!(run(&db, &wrong).await.is_err());
        let preview = CoreStore::connect_dry_run(&db.url).await.unwrap();
        run(&preview, &input).await.unwrap();
        db.exec("UPDATE users SET link_status='pending',workos_user_id=NULL WHERE normalized_email='operator@example.org'").await;
        assert!(run(&db, &input).await.is_err());
        assert!(db.all("brain_scoped_records").await.is_empty());
        assert!(db.all("brain_scoped_record_operations").await.is_empty());
        db.exec("UPDATE users SET link_status='linked',workos_user_id='workos_operator_example_org' WHERE normalized_email='operator@example.org'").await;
        run(&db, &input).await.unwrap();
        let mut undo = retirement(&input, "wrong_scope");
        undo.brain_id = "other-brain".into();
        assert!(run(&db, &undo).await.is_err());
        let undo = retirement(&input, "correct_scope");
        run(&db, &undo).await.unwrap();
        assert!(run(&db, &retirement(&undo, "cannot_undo_retirement")).await.is_err());
        assert_eq!(db.all("brain_scoped_record_operations").await.len(), 2);
    }).await;
}

#[tokio::test]
async fn scoped_human_agent_and_foreign_pin_conflicts_refuse_before_creation() {
    with_isolated_postgres(|db| async move {
        let sam = setup(&db).await;
        project(&db, "guard-project", &sam, "Example Agent").await;
        project(&db, "other-project", &sam, "Other Agent").await;
        project(&db, "human-project", &sam, "Human-conflicted Agent").await;
        runtime(&db, "guard-runtime", "guard-project", &key(1), true, None).await;
        let account_pin = manifest("account_pin", vec![record(1, ScopedRecordKind::AccountChatKey, Some(&sam), None)]);
        assert!(run(&db, &account_pin).await.is_err());
        observe(&db, &sam, &human_observation("human_pin", &key(2), BRAIN)).await.unwrap();
        runtime(&db, "human-runtime", "human-project", &key(2), true, None).await;
        let human_pin = manifest("human_pin", vec![record(2, ScopedRecordKind::ManagedAgent, Some(&sam), Some("human-project"))]);
        assert!(run(&db, &human_pin).await.is_err());
        db.exec("INSERT INTO project_runtime_links(id,project_id,agent_runtime_id,active,created_at) VALUES ('foreign-link','other-project','guard-runtime',true,now())").await;
        let foreign = manifest("foreign_pin", vec![record(1, ScopedRecordKind::ManagedAgent, Some(&sam), Some("guard-project"))]);
        assert!(run(&db, &foreign).await.is_err());
        assert!(db.all("brain_scoped_records").await.is_empty());
        assert!(db.all("brain_scoped_record_operations").await.is_empty());
    }).await;
}

#[tokio::test]
async fn scoped_hundred_record_page_fits_wire_bound_and_refuses_101() {
    with_isolated_postgres(|db| async move {
        setup(&db).await;
        let mut input = manifest(
            "full_page",
            (1..=100)
                .map(|n| record(n, ScopedRecordKind::HistoricalIdentity, None, None))
                .collect(),
        );
        let receipt = run(&db, &input).await.unwrap();
        assert_eq!(receipt.records.len(), 100);
        let keys: Vec<_> = (1..=100).collect();
        let rows = page(&db, BRAIN, &keys, &[]).await;
        assert_eq!(rows.len(), 100);
        assert!(
            rows.iter()
                .all(|r| r.operator_record.as_ref().unwrap().state == OperatorRecordState::Used)
        );
        let response = ScopedDescriptionsResponse {
            version: DESCRIPTIONS_VERSION_V3.into(),
            brain_id: BRAIN.into(),
            checked_at: stamp(now()),
            results: rows,
        };
        assert!(serde_json::to_vec(&response).unwrap().len() < 256 * 1024);
        input.records.push(record(
            101,
            ScopedRecordKind::HistoricalIdentity,
            None,
            None,
        ));
        input.operation_id = "operation_too_many_0000".into();
        assert!(run(&db, &input).await.is_err());
        let too_many = ScopedDescriptionsRequest {
            version: DESCRIPTIONS_VERSION_V3.into(),
            brain_server: SERVER.into(),
            brain_id: BRAIN.into(),
            requested_by_public_key_hex: key(0xee),
            public_keys_hex: (1..=101).map(key).collect(),
            participating_public_keys_hex: vec![],
        };
        assert!(
            db.describe_scoped_brain_identities(&too_many)
                .await
                .is_err()
        );
        assert_eq!(db.all("brain_scoped_record_operations").await.len(), 1);
    })
    .await;
}
