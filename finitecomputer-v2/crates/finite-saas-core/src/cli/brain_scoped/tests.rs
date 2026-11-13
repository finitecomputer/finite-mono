//! Exercise the command's file/digest/connection path on disposable PostgreSQL.
use super::*;
use finite_saas_core::brain_identity::scoped::*;
use serde_json::Value;
mod fixture;
use fixture::*;

#[tokio::test]
async fn scoped_command_preview_apply_replay_retire_never_run_startup_ddl() {
    with_database(true, |db| async move {
        let before = db.unrelated_rows().await;
        db.reject_ddl().await;
        let apply = manifest();
        db.run(&apply, false).await.unwrap();
        assert_eq!(db.count("brain_scoped_records").await, 0);
        assert_eq!(db.count("brain_scoped_record_operations").await, 0);
        db.run(&apply, true).await.unwrap();
        let receipt = db.receipt(&apply.operation_id).await;
        db.run(&apply, true).await.unwrap();
        assert_eq!(db.receipt(&apply.operation_id).await, receipt);
        assert_eq!(db.count("brain_scoped_records").await, 2);
        assert_eq!(db.count("brain_scoped_record_operations").await, 1);

        let mut retire = apply.clone();
        retire.operation_id = "synthetic_retirement_0001".into();
        retire.action = ScopedAction::Retire;
        retire.target_operation_id = Some(apply.operation_id);
        retire.records.clear();
        db.run(&retire, false).await.unwrap();
        assert_eq!(db.count("brain_scoped_record_operations").await, 1);
        db.run(&retire, true).await.unwrap();
        let receipt = db.receipt(&retire.operation_id).await;
        db.run(&retire, true).await.unwrap();
        assert_eq!(db.receipt(&retire.operation_id).await, receipt);
        assert_eq!(db.count("brain_scoped_record_operations").await, 2);
        let active: i64 = db
            .client
            .query_one(
                "SELECT count(*) FROM brain_scoped_records WHERE retired_at IS NULL",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(active, 0);
        assert_eq!(db.unrelated_rows().await, before);
    })
    .await;
}

#[tokio::test]
async fn scoped_command_ignores_unrelated_locks_and_rolls_back_on_target_lock() {
    with_database(true, |mut db| async move {
        let mut blocker = db.connection().await;
        let lock = blocker.transaction().await.unwrap();
        lock.batch_execute("LOCK finite_private_limit_profiles IN ACCESS EXCLUSIVE MODE")
            .await
            .unwrap();
        // A schema sweep would block on this unrelated table. A scoped
        // command needs no limiter locks and remains usable while it is held.
        let mut unrelated = manifest();
        unrelated.operation_id = "synthetic_unrelated_0001".into();
        unrelated.brain_id = "unrelated-brain".into();
        tokio::time::timeout(Duration::from_secs(2), db.run(&unrelated, true))
            .await
            .expect("scoped command waited on an unrelated table")
            .unwrap();
        lock.rollback().await.unwrap();

        let lock = db.client.transaction().await.unwrap();
        lock.batch_execute("LOCK brain_scoped_records IN ACCESS EXCLUSIVE MODE")
            .await
            .unwrap();
        let apply = manifest();
        let started = Instant::now();
        let error = tokio::time::timeout(Duration::from_secs(8), run(&apply, true))
            .await
            .expect("scoped command exceeded its lock budget")
            .unwrap_err();
        let Some(finite_saas_core::CoreError::Database(detail)) =
            error.downcast_ref::<finite_saas_core::CoreError>()
        else {
            panic!("expected database lock refusal: {error:?}");
        };
        assert_eq!(detail.code.as_deref(), Some("55P03"));
        assert!(started.elapsed() >= Duration::from_secs(2));
        assert!(started.elapsed() < Duration::from_secs(8));
        lock.rollback().await.unwrap();
        assert_eq!(db.count("brain_scoped_records").await, 2);
        assert_eq!(db.count("brain_scoped_record_operations").await, 1);
        // Keep the exact operation and digest after the failed transaction.
        db.run(&apply, true).await.unwrap();
        let receipt = db.receipt(&apply.operation_id).await;
        db.run(&apply, true).await.unwrap();
        assert_eq!(db.receipt(&apply.operation_id).await, receipt);
        assert_eq!(db.count("brain_scoped_record_operations").await, 2);
    })
    .await;
}

#[tokio::test]
async fn scoped_command_missing_schema_fails_without_installing_it() {
    with_database(false, |db| async move {
        for execute in [false, true] {
            assert!(db.run(&manifest(), execute).await.is_err());
            let tables: i64 = db
                .client
                .query_one(
                    "SELECT count(*) FROM pg_tables WHERE schemaname='public'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(tables, 0, "operator command installed schema");
        }
    })
    .await;
}

#[tokio::test]
async fn scoped_command_requires_reviewed_digest_before_database_connection() {
    with_database(false, |_db| async move {
        let input = manifest();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&input).unwrap()).unwrap();
        let missing = run_scoped_records_command(file.path().into(), None, true)
            .await
            .unwrap_err();
        assert_eq!(
            missing.to_string(),
            "execute requires the reviewed manifest digest"
        );
        let mismatch = run_scoped_records_command(file.path().into(), Some("00".repeat(32)), true)
            .await
            .unwrap_err();
        assert_eq!(
            mismatch.to_string(),
            "scoped manifest digest differs from reviewed preview"
        );
    })
    .await;
}
