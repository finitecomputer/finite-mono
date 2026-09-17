use super::*;
use crate::{
    LinkVerifiedUserInput,
    test_support::{TestDb, with_isolated_postgres},
};

async fn fixture(db: &TestDb) -> AccountEmailChangeRequest {
    let user = db
        .link_verified_user(LinkVerifiedUserInput {
            verified_email: "before@example.test".into(),
            workos_user_id: "user_email_fixture".into(),
            now: None,
        })
        .await
        .unwrap();
    let client = db.connection().await.unwrap();
    client.execute("INSERT INTO projects(id, customer_org_id, owner_user_id, display_name, created_at, updated_at)
            SELECT 'project-email-fixture', id, owner_user_id, 'Fixture', now(), now() FROM customer_orgs WHERE owner_user_id=$1", &[&user.id]).await.unwrap();
    client.execute("INSERT INTO finite_private_grants (id,user_id,limit_profile_id,status,current_window_used_units,created_at,updated_at) VALUES ('grant-email-fixture',$1,'finite-private-generous-v2','active',17,now(),now())", &[&user.id]).await.unwrap();
    client.execute("INSERT INTO finite_private_api_keys (id,grant_id,project_id,key_hash,status,created_at,updated_at) VALUES ('key-email-fixture','grant-email-fixture','project-email-fixture',repeat('a',64),'active',now(),now())", &[]).await.unwrap();
    client.execute("INSERT INTO customer_billing_accounts (customer_org_id,stripe_customer_id,created_at,updated_at) SELECT id,'cus_email_fixture',now(),now() FROM customer_orgs WHERE owner_user_id=$1", &[&user.id]).await.unwrap();
    AccountEmailChangeRequest {
        operation_id: "email-change-fixture".into(),
        user_id: user.id,
        workos_user_id: "user_email_fixture".into(),
        expected_email: "before@example.test".into(),
        new_email: "after@example.test".into(),
        evidence_reference: "private-evidence-fixture".into(),
    }
}

async fn resources(db: &TestDb) -> Value {
    json!({
        "grants": db.query_json("SELECT to_jsonb(g) FROM finite_private_grants g ORDER BY id", &[]).await,
        "keys": db.query_json("SELECT to_jsonb(k) FROM finite_private_api_keys k ORDER BY id", &[]).await,
        "billing": db.query_json("SELECT to_jsonb(b) FROM customer_billing_accounts b ORDER BY customer_org_id", &[]).await,
        "projects": db.query_json("SELECT to_jsonb(p) FROM projects p ORDER BY id", &[]).await,
        "orgs": db.query_json("SELECT to_jsonb(o) FROM customer_orgs o ORDER BY id", &[]).await,
        "entitlements": db.query_json("SELECT to_jsonb(e) FROM agent_creation_entitlements e ORDER BY id", &[]).await,
    })
}

// Laboratory-only disposition: no production retirement API is implied.
#[tokio::test]
async fn duplicate_account_rehearsal_preserves_rows_and_reverses_email_cutover() {
    with_isolated_postgres(|db| async move {
            let r = fixture(&db).await;
            let duplicate_subject = "user_duplicate_rehearsal";
            let duplicate = db.link_verified_user(LinkVerifiedUserInput {
                verified_email: r.new_email.clone(),
                workos_user_id: duplicate_subject.into(),
                now: None,
            }).await.unwrap();
            let before = resources(&db).await;
            let original_row = db.row("users", &r.user_id).await.unwrap();
            let duplicate_row = db.row("users", &duplicate.id).await.unwrap();
            assert_eq!(db.preview_account_email_change(r.clone()).await.unwrap().blockers,
                ["destination_account_exists"]);
            assert!(db.prepare_account_email_change(r.clone(), "operator_fixture", &r.expected_email).await.is_err());

            // An enrolled user owns a personal organization even with zero Projects.
            // Deleting just that user is rejected, rather than cascading into data.
            let mut client = db.connection().await.unwrap();
            let err = client.execute("DELETE FROM users WHERE id=$1", &[&duplicate.id]).await.unwrap_err();
            assert_eq!(err.code(), Some(&tokio_postgres::error::SqlState::FOREIGN_KEY_VIOLATION));
            assert_eq!(db.row("users", &duplicate.id).await.unwrap(), duplicate_row);
            assert_eq!(resources(&db).await, before);

            // Prove the proposed parking step can be rolled back before cutover.
            // WorkOS deletion itself is not simulated as reversible by this test.
            let tx = client.transaction().await.unwrap();
            tx.execute("UPDATE users SET normalized_email='holding@example.test' WHERE id=$1 AND normalized_email=$2",
                &[&duplicate.id, &r.new_email]).await.unwrap();
            tx.rollback().await.unwrap();
            assert_eq!(db.row("users", &duplicate.id).await.unwrap(), duplicate_row);

            // TEST FIXTURE ONLY: stand in for a separately reviewed duplicate
            // retirement operation after provider credentials/sessions are retired.
            assert_eq!(client.execute("UPDATE users SET normalized_email='holding@example.test' WHERE id=$1 AND normalized_email=$2",
                &[&duplicate.id, &r.new_email]).await.unwrap(), 1);
            db.prepare_account_email_change(r.clone(), "operator_fixture", &r.expected_email).await.unwrap();
            // Provider still reports source: cannot prematurely complete Core.
            assert!(db.complete_account_email_change(r.clone(), "operator_fixture", &r.expected_email).await.is_err());
            assert_eq!(db.row("users", &r.user_id).await.unwrap(), original_row);
            let reopened = CoreStore::connect(&db.url).await.unwrap();
            reopened.complete_account_email_change(r.clone(), "operator_fixture", &r.new_email).await.unwrap();
            let linked = db.link_verified_user(LinkVerifiedUserInput {
                verified_email: r.new_email.clone(), workos_user_id: r.workos_user_id.clone(), now: None,
            }).await.unwrap();
            assert_eq!(linked.id, r.user_id);
            assert_eq!(resources(&db).await, before);
            // A stale duplicate subject cannot take back the destination email.
            assert!(db.link_verified_user(LinkVerifiedUserInput {
                verified_email: r.new_email.clone(), workos_user_id: duplicate_subject.into(), now: None,
            }).await.is_err());

            // Reverse Core using a new intent and matching fresh provider evidence.
            let mut reverse = r.clone();
            reverse.operation_id = "email-change-reversal-fixture".into();
            reverse.expected_email = r.new_email.clone();
            reverse.new_email = r.expected_email.clone();
            reopened.prepare_account_email_change(reverse.clone(), "operator_fixture", &reverse.expected_email).await.unwrap();
            reopened.complete_account_email_change(reverse.clone(), "operator_fixture", &reverse.new_email).await.unwrap();
            client.execute("UPDATE users SET normalized_email=$2 WHERE id=$1", &[&duplicate.id, &r.new_email]).await.unwrap();
            assert_eq!(db.row("users", &r.user_id).await.unwrap()["normalized_email"], r.expected_email);
            assert_eq!(db.row("users", &duplicate.id).await.unwrap(), duplicate_row);
            assert_eq!(resources(&db).await, before);
            // An old completed receipt must not replay over a newer reversal.
            assert!(reopened.complete_account_email_change(r.clone(), "operator_fixture", &r.new_email).await.is_err());
        }).await;
}

#[tokio::test]
async fn account_email_change_preserves_existing_identity_and_resources_across_restart_and_retry() {
    with_isolated_postgres(|db| async move {
        let r = fixture(&db).await;
        let before = resources(&db).await;
        let preview = db.preview_account_email_change(r.clone()).await.unwrap();
        assert_eq!(preview.status, "unprepared");
        assert_eq!(preview.project_ids, ["project-email-fixture"]);
        assert!(
            db.query_json("SELECT to_jsonb(c) FROM account_email_changes c", &[])
                .await
                .is_empty()
        );
        // Existing ordinary enrollment must continue rejecting an implicit rename.
        assert!(matches!(
            db.link_verified_user(LinkVerifiedUserInput {
                verified_email: r.new_email.clone(),
                workos_user_id: r.workos_user_id.clone(),
                now: None
            })
            .await,
            Err(CoreError::WorkosUserConflict)
        ));
        db.prepare_account_email_change(r.clone(), "operator_fixture", &r.expected_email)
            .await
            .unwrap();
        db.prepare_account_email_change(r.clone(), "operator_fixture", &r.expected_email)
            .await
            .unwrap();
        let reopened = CoreStore::connect(&db.url).await.unwrap();
        let result = reopened
            .complete_account_email_change(r.clone(), "operator_fixture", &r.new_email)
            .await
            .unwrap();
        assert_eq!(result.status, "completed");
        reopened
            .complete_account_email_change(r.clone(), "operator_fixture", &r.new_email)
            .await
            .unwrap();
        let user = db
            .link_verified_user(LinkVerifiedUserInput {
                verified_email: r.new_email.clone(),
                workos_user_id: r.workos_user_id.clone(),
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(user.id, r.user_id);
        assert_eq!(
            user.workos_user_id.as_deref(),
            Some(r.workos_user_id.as_str())
        );
        assert_eq!(before, resources(&db).await);
        assert_eq!(
            db.query_json("SELECT to_jsonb(c) FROM account_email_changes c", &[])
                .await
                .len(),
            1
        );
        // Stale source sessions cannot reverse the rename through old enrollment code.
        assert!(
            db.link_verified_user(LinkVerifiedUserInput {
                verified_email: r.expected_email.clone(),
                workos_user_id: r.workos_user_id.clone(),
                now: None
            })
            .await
            .is_err()
        );
        assert!(
            db.cancel_account_email_change(r.clone(), "operator_fixture", &r.expected_email)
                .await
                .is_err()
        );
    })
    .await;
}

#[tokio::test]
async fn account_email_change_fails_closed_on_collisions_and_changed_workos_evidence() {
    with_isolated_postgres(|db| async move {
        let r = fixture(&db).await;
        assert!(
            db.complete_account_email_change(r.clone(), "operator_fixture", &r.new_email)
                .await
                .is_err()
        );
        assert!(
            db.prepare_account_email_change(r.clone(), "operator_fixture", &r.new_email)
                .await
                .is_err()
        );
        db.prepare_account_email_change(r.clone(), "operator_fixture", &r.expected_email)
            .await
            .unwrap();
        assert!(
            db.complete_account_email_change(r.clone(), "operator_fixture", &r.expected_email)
                .await
                .is_err()
        );
        let mut changed = r.clone();
        changed.new_email = "other@example.test".into();
        assert!(db.preview_account_email_change(changed).await.is_err());
        let mut concurrent = r.clone();
        concurrent.operation_id = "other-operation".into();
        assert!(
            db.prepare_account_email_change(concurrent, "operator_fixture", &r.expected_email)
                .await
                .is_err()
        );
        // A preexisting-version enrollment writer can claim the destination
        // after preparation. Completion must detect this, never overwrite it.
        db.link_verified_user(LinkVerifiedUserInput {
            verified_email: r.new_email.clone(),
            workos_user_id: "user_destination_fixture".into(),
            now: None,
        })
        .await
        .unwrap();
        assert!(
            db.complete_account_email_change(r.clone(), "operator_fixture", &r.new_email)
                .await
                .is_err()
        );
        assert_eq!(
            db.row("users", &r.user_id).await.unwrap()["normalized_email"],
            r.expected_email
        );
        let cancelled = db
            .cancel_account_email_change(r.clone(), "operator_fixture", &r.expected_email)
            .await
            .unwrap();
        assert_eq!(cancelled.status, "cancelled");
        assert_eq!(cancelled.blockers, ["destination_account_exists"]);
    })
    .await;
}

#[tokio::test]
async fn account_email_change_dry_run_and_cancel_do_not_rewrite_account() {
    with_isolated_postgres(|db| async move {
        let r = fixture(&db).await;
        db.prepare_account_email_change(r.clone(), "operator_fixture", &r.expected_email)
            .await
            .unwrap();
        let dry = CoreStore::connect_dry_run(&db.url).await.unwrap();
        assert_eq!(
            dry.complete_account_email_change(r.clone(), "operator_fixture", &r.new_email)
                .await
                .unwrap()
                .status,
            "completed"
        );
        assert_eq!(
            db.preview_account_email_change(r.clone())
                .await
                .unwrap()
                .status,
            "prepared"
        );
        db.cancel_account_email_change(r.clone(), "operator_fixture", &r.expected_email)
            .await
            .unwrap();
        db.cancel_account_email_change(r.clone(), "operator_fixture", &r.expected_email)
            .await
            .unwrap();
        assert!(
            db.complete_account_email_change(r.clone(), "operator_fixture", &r.new_email)
                .await
                .is_err()
        );
        assert_eq!(
            db.row("users", &r.user_id).await.unwrap()["normalized_email"],
            r.expected_email
        );
    })
    .await;
}
