//! Explicit operator transition after WorkOS verification. Ordinary enrollment
//! cannot perform this transition or infer an account merge from an email.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountEmailChangeRequest {
    pub operation_id: String,
    pub user_id: String,
    pub workos_user_id: String,
    pub expected_email: String,
    pub new_email: String,
    /// Private operator evidence reference; never credentials or customer data.
    pub evidence_reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountEmailChangePreview {
    pub request: AccountEmailChangeRequest,
    pub status: String,
    pub project_ids: Vec<String>,
    pub customer_org_ids: Vec<String>,
    pub finite_private_grant_ids: Vec<String>,
    /// This report establishes only Core eligibility, not product migration readiness.
    pub external_checks_required: Vec<String>,
    pub blockers: Vec<String>,
}

fn conflict() -> CoreError {
    CoreError::AccountEmailChangeConflict
}

impl AccountEmailChangeRequest {
    fn validate(&self) -> CoreResult<()> {
        for value in [
            &self.operation_id,
            &self.user_id,
            &self.workos_user_id,
            &self.evidence_reference,
        ] {
            if value.is_empty()
                || value.len() > 256
                || value.trim() != value
                || value.chars().any(char::is_control)
            {
                return Err(conflict());
            }
        }
        for email in [&self.expected_email, &self.new_email] {
            if normalize_owner_email(Some(email)).as_ref() != Some(email)
                || email.len() > 320
                || !email.contains('@')
                || email.chars().any(char::is_whitespace)
            {
                return Err(conflict());
            }
        }
        if self.expected_email == self.new_email {
            return Err(conflict());
        }
        Ok(())
    }
}

impl CoreStore {
    /// SELECT-only repeatable-read inventory. No schema initialization or locks.
    pub async fn preview_account_email_change(
        &self,
        request: AccountEmailChangeRequest,
    ) -> CoreResult<AccountEmailChangePreview> {
        request.validate()?;
        let mut client = self.connection().await?;
        let tx = client
            .build_transaction()
            .read_only(true)
            .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
            .start()
            .await
            .map_err(store_error)?;
        let result = preview(&*tx, request).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    /// Persists intent only. WorkOS and Sites are separate operator-owned steps.
    pub async fn prepare_account_email_change(
        &self,
        request: AccountEmailChangeRequest,
        actor: &str,
        verified_email: &str,
    ) -> CoreResult<AccountEmailChangePreview> {
        request.validate()?;
        if actor.is_empty() || verified_email != request.expected_email {
            return Err(conflict());
        }
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        lock_users(&*tx).await?;
        let result = preview(&*tx, request.clone()).await?;
        if !result.blockers.is_empty() {
            return Err(conflict());
        }
        if result.status == "unprepared" {
            let now = current_time_iso()?;
            tx.execute("INSERT INTO account_email_changes
                (id, user_id, workos_user_id, old_email, new_email, status, prepared_by, evidence_reference, created_at, updated_at)
                VALUES ($1,$2,$3,$4,$5,'prepared',$6,$7,$8::text::timestamptz,$8::text::timestamptz)",
                &[&request.operation_id,&request.user_id,&request.workos_user_id,&request.expected_email,&request.new_email,&actor,&request.evidence_reference,&now]).await.map_err(store_error)?;
        } else if result.status != "prepared" {
            return Err(conflict());
        }
        let result = preview(&*tx, request).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    /// Finishes only the persisted intent, after a fresh server-side WorkOS
    /// lookup confirms the same subject has the verified destination email.
    pub async fn complete_account_email_change(
        &self,
        request: AccountEmailChangeRequest,
        actor: &str,
        verified_email: &str,
    ) -> CoreResult<AccountEmailChangePreview> {
        request.validate()?;
        if actor.is_empty() || verified_email != request.new_email {
            return Err(conflict());
        }
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        lock_users(&*tx).await?;
        let before = preview(&*tx, request.clone()).await?;
        if !before.blockers.is_empty() {
            return Err(conflict());
        }
        match before.status.as_str() {
            "completed" => {}
            "prepared" => {
                let now = current_time_iso()?;
                let count = tx
                    .execute(
                        "UPDATE users SET normalized_email=$1, updated_at=$2::text::timestamptz
                    WHERE id=$3 AND workos_user_id=$4 AND normalized_email=$5",
                        &[
                            &request.new_email,
                            &now,
                            &request.user_id,
                            &request.workos_user_id,
                            &request.expected_email,
                        ],
                    )
                    .await
                    .map_err(store_error)?;
                if count != 1 {
                    return Err(conflict());
                }
                tx.execute("UPDATE account_email_changes SET status='completed', completed_by=$1, updated_at=$2::text::timestamptz WHERE id=$3 AND status='prepared'",
                    &[&actor,&now,&request.operation_id]).await.map_err(store_error)?;
            }
            _ => return Err(conflict()),
        }
        let result = preview(&*tx, request).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    /// Cancels only an uncommitted intent after WorkOS still/again verifies the
    /// source email. It never rolls back a completed change or product data.
    pub async fn cancel_account_email_change(
        &self,
        request: AccountEmailChangeRequest,
        actor: &str,
        verified_email: &str,
    ) -> CoreResult<AccountEmailChangePreview> {
        request.validate()?;
        if actor.is_empty() || verified_email != request.expected_email {
            return Err(conflict());
        }
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        lock_users(&*tx).await?;
        let before = preview(&*tx, request.clone()).await?;
        if !matches!(before.status.as_str(), "prepared" | "cancelled") {
            return Err(conflict());
        }
        tx.execute("UPDATE account_email_changes SET status='cancelled', completed_by=$1, updated_at=now() WHERE id=$2 AND status='prepared'",
            &[&actor,&request.operation_id]).await.map_err(store_error)?;
        let result = preview(&*tx, request).await?;
        self.finish(tx).await?;
        Ok(result)
    }
}

// Short transaction, no network calls. Serialize against *existing* enrollment
// writers too, including old binaries that do not know about this operation.
async fn lock_users<C: GenericClient + Sync>(tx: &C) -> CoreResult<()> {
    tx.batch_execute("SET LOCAL lock_timeout = '5s'; LOCK TABLE users IN SHARE ROW EXCLUSIVE MODE")
        .await
        .map_err(store_error)
}

async fn preview<C: GenericClient + Sync>(
    tx: &C,
    request: AccountEmailChangeRequest,
) -> CoreResult<AccountEmailChangePreview> {
    let operation = tx.query_opt("SELECT user_id, workos_user_id, old_email, new_email, evidence_reference, status FROM account_email_changes WHERE id=$1", &[&request.operation_id]).await.map_err(store_error)?;
    let status = if let Some(row) = operation {
        for (column, expected) in [
            ("user_id", &request.user_id),
            ("workos_user_id", &request.workos_user_id),
            ("old_email", &request.expected_email),
            ("new_email", &request.new_email),
            ("evidence_reference", &request.evidence_reference),
        ] {
            if row.get::<_, String>(column) != *expected {
                return Err(conflict());
            }
        }
        row.get::<_, String>("status")
    } else {
        "unprepared".into()
    };
    let user = tx
        .query_opt(
            "SELECT normalized_email, workos_user_id FROM users WHERE id=$1",
            &[&request.user_id],
        )
        .await
        .map_err(store_error)?
        .ok_or_else(conflict)?;
    let expected = if status == "completed" {
        &request.new_email
    } else {
        &request.expected_email
    };
    if user.get::<_, String>("normalized_email") != *expected
        || user.get::<_, Option<String>>("workos_user_id").as_ref() != Some(&request.workos_user_id)
    {
        return Err(conflict());
    }
    let mut blockers = Vec::new();
    if tx
        .query_opt(
            "SELECT id FROM users WHERE normalized_email=$1 AND id<>$2",
            &[&request.new_email, &request.user_id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        blockers.push("destination_account_exists".to_string());
    }
    if tx.query_opt("SELECT id FROM account_email_changes WHERE status='prepared' AND id<>$1 AND (user_id=$2 OR new_email=$3)", &[&request.operation_id,&request.user_id,&request.new_email]).await.map_err(store_error)?.is_some() { blockers.push("another_operation_pending".to_string()); }
    let project_ids = ids(
        tx,
        "SELECT id FROM projects WHERE owner_user_id=$1 ORDER BY id",
        &request.user_id,
    )
    .await?;
    let customer_org_ids = ids(
        tx,
        "SELECT id FROM customer_orgs WHERE owner_user_id=$1 ORDER BY id",
        &request.user_id,
    )
    .await?;
    let finite_private_grant_ids = ids(
        tx,
        "SELECT id FROM finite_private_grants WHERE user_id=$1 ORDER BY id",
        &request.user_id,
    )
    .await?;
    Ok(AccountEmailChangePreview {
        request,
        status,
        project_ids,
        customer_org_ids,
        finite_private_grant_ids,
        blockers,
        external_checks_required: [
            "WorkOS destination collision and Google/SSO eligibility",
            "Sites mailbox ownership, authorized keys, shares and outstanding sessions/assertions",
            "Hosted session refresh and same-identity chat/Brain verification",
            "Connected accounts and billing contact disposition",
        ]
        .map(String::from)
        .to_vec(),
    })
}

async fn ids<C: GenericClient + Sync>(tx: &C, sql: &str, user_id: &str) -> CoreResult<Vec<String>> {
    Ok(tx
        .query(sql, &[&user_id])
        .await
        .map_err(store_error)?
        .iter()
        .map(|r| r.get(0))
        .collect())
}

#[cfg(test)]
mod tests {
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

    #[tokio::test]
    async fn account_email_change_preserves_existing_identity_and_resources_across_restart_and_retry()
     {
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
}
