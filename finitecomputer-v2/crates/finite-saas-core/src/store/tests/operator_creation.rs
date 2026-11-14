//! Store-level guards of operator creation (migration 0047): exact admin
//! identity, fail-closed owner refusals, the admission's effect on the
//! owner's own allowance, concurrent retries and the rollback-only preview.

use super::*;
use crate::{OperatorCreateAgentInput, OperatorCreationSurface, OperatorIdentity};

const ADMIN_EMAIL: &str = "operator@finite.vip";
const ADMIN_WORKOS: &str = "workos_operator";

async fn link(db: &TestDb, email: &str, workos_user_id: &str) -> CoreUser {
    db.link_verified_user(LinkVerifiedUserInput {
        verified_email: email.to_string(),
        workos_user_id: workos_user_id.to_string(),
        now: None,
    })
    .await
    .unwrap()
}

fn input(owner_email: &str, key: &str) -> OperatorCreateAgentInput {
    OperatorCreateAgentInput {
        admin: OperatorIdentity {
            email: ADMIN_EMAIL.to_string(),
            workos_user_id: ADMIN_WORKOS.to_string(),
        },
        surface: OperatorCreationSurface::Cli,
        owner_email: owner_email.to_string(),
        display_name: "Migrated Bot".to_string(),
        reason: "box1 migration".to_string(),
        idempotency_key: key.to_string(),
        hosting_tier: None,
        runtime_resource_class: None,
        target_source_host_id: None,
        owner_chat_account_id: None,
        profile_picture_url: None,
        placement_override: None,
        now: None,
    }
}

fn refusal(error: CoreError) -> &'static str {
    match error {
        CoreError::OperatorAgentCreationRefused(message) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

async fn activate_billing(db: &TestDb, email: &str, workos_user_id: &str, run: &str) {
    let org_id = db
        .link_stripe_customer(LinkStripeCustomerInput {
            verified_email: email.to_string(),
            workos_user_id: workos_user_id.to_string(),
            stripe_customer_id: format!("cus_{run}"),
            now: None,
        })
        .await
        .unwrap()
        .customer_org_id;
    db.sync_stripe_subscription(SyncStripeSubscriptionInput {
        customer_org_id: Some(org_id),
        stripe_customer_id: format!("cus_{run}"),
        stripe_subscription_id: format!("sub_{run}"),
        stripe_price_id: Some("price_standard".to_string()),
        expected_stripe_price_id: Some("price_standard".to_string()),
        subscription_status: BillingSubscriptionStatus::Active,
        current_period_end: Some("2099-01-01T00:00:00Z".to_string()),
        cancel_at_period_end: false,
        stripe_event_id: Some(format!("evt_{run}")),
        stripe_event_created: None,
        now: None,
    })
    .await
    .unwrap();
}

async fn self_serve(db: &TestDb, email: &str, workos_user_id: &str, key: &str) -> CoreResult<()> {
    db.request_agent_creation(RequestAgentCreationInput {
        verified_email: email.to_string(),
        workos_user_id: workos_user_id.to_string(),
        display_name: "Own Agent".to_string(),
        launch_code: String::new(),
        idempotency_key: key.to_string(),
        now: None,
    })
    .await
    .map(|_| ())
}

#[tokio::test]
async fn operator_creation_requires_one_exact_linked_admin_and_never_relinks_it() {
    with_isolated_postgres(|db| async move {
        let admin = link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        for (email, workos) in [
            (ADMIN_EMAIL, "workos_someone_else"),
            ("nobody@finite.vip", ADMIN_WORKOS),
            ("nobody@finite.vip", "workos_nobody"),
        ] {
            let mut wrong = input("owner@example.com", "admin-check");
            wrong.admin = OperatorIdentity {
                email: email.to_string(),
                workos_user_id: workos.to_string(),
            };
            let error = db.admin_create_agent_for_owner(wrong).await.unwrap_err();
            assert!(
                matches!(error, CoreError::OperatorIdentityMismatch),
                "{error:?}"
            );
        }
        // The admin row was not rewritten and no one else was enrolled.
        let users = db.all("users").await;
        assert_eq!(users.len(), 1);
        assert_eq!(users[0]["workos_user_id"], ADMIN_WORKOS);

        let mut targeted = input("owner@example.com", "admin-check");
        targeted.target_source_host_id = Some("Lat-3".to_string());
        let created = db.admin_create_agent_for_owner(targeted).await.unwrap();
        assert_eq!(created.record.admin_user_id, admin.id);
        assert_eq!(created.record.surface, OperatorCreationSurface::Cli);
        assert_eq!(
            created.request.target_source_host_id.as_deref(),
            Some("lat-3")
        );
        assert_eq!(
            created.record.target_source_host_id.as_deref(),
            Some("lat-3")
        );

        let mut phala_targeted = input("owner@example.com", "phala-target");
        phala_targeted.hosting_tier = Some(HostingTier::Confidential);
        phala_targeted.target_source_host_id = Some("lat-3".to_string());
        let error = db
            .admin_create_agent_for_owner(phala_targeted)
            .await
            .unwrap_err();
        assert!(refusal(error).contains("Kata"));
    })
    .await;
}

#[tokio::test]
async fn operator_creation_refuses_ambiguous_or_trial_owners() {
    with_isolated_postgres(|db| async move {
        let admin = link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        // A completed email change moved old@ to another address: old@ may
        // name that person or a new one.
        db.query_json(
            "INSERT INTO account_email_changes
               (id, user_id, workos_user_id, old_email, new_email, status, prepared_by,
                evidence_reference, created_at, updated_at)
             VALUES ('change_done', $1, 'w', 'old@example.com', 'renamed@example.com',
                     'completed', 'w', 'ticket', NOW(), NOW()),
                    ('change_open', $1, 'w', 'renamed@example.com', 'incoming@example.com',
                     'prepared', 'w', 'ticket', NOW(), NOW())
             RETURNING to_jsonb(id)",
            &[&admin.id],
        )
        .await;
        for email in ["old@example.com", "incoming@example.com"] {
            let error = db
                .admin_create_agent_for_owner(input(email, &format!("change-{email}")))
                .await
                .unwrap_err();
            assert!(refusal(error).contains("email change"), "{email}");
        }

        // An Agent's own Finite address is not a person.
        let agent = db
            .admin_create_agent_for_owner(input("person@example.com", "first"))
            .await
            .unwrap();
        let agent_email = agent.project.agent_email.clone().unwrap();
        let error = db
            .admin_create_agent_for_owner(input(&agent_email.to_uppercase(), "agent-address"))
            .await
            .unwrap_err();
        assert!(refusal(error).contains("Agent's own address"));

        // A live event trial would stop the Agent when it ends unpaid.
        let trial_user = link(&db, "trial@example.com", "workos_trial").await;
        let trial_org = db
            .query_json(
                "SELECT to_jsonb(id) FROM customer_orgs WHERE owner_user_id = $1",
                &[&trial_user.id],
            )
            .await[0]
            .as_str()
            .unwrap()
            .to_string();
        db.query_json(
            "WITH campaign AS (
               INSERT INTO trial_campaigns (id, name, code_hash, seat_limit, created_by_workos_user_id)
               VALUES ('campaign', 'Workshop', 'hash', 5, 'w') RETURNING id)
             INSERT INTO trial_redemptions
               (id, campaign_id, customer_org_id, attempt_id, stripe_session_id, state, checkout_expires_at, redeemed_at)
             SELECT 'redemption', id, $1, 'attempt', 'cs_trial', 'redeemed', NOW(), NOW() FROM campaign
             RETURNING to_jsonb(id)",
            &[&trial_org],
        )
        .await;
        let error = db
            .admin_create_agent_for_owner(input("trial@example.com", "trial"))
            .await
            .unwrap_err();
        assert!(refusal(error).contains("event trial"));
        // An expired checkout is not a trial: the same person is accepted.
        db.query_json(
            "UPDATE trial_redemptions SET state = 'expired' RETURNING to_jsonb(id)",
            &[],
        )
        .await;
        db.admin_create_agent_for_owner(input("trial@example.com", "trial"))
            .await
            .unwrap();

        // Every refusal wrote nothing: one Agent each for person@ and trial@.
        assert_eq!(db.all("operator_agent_creations").await.len(), 2);
        assert_eq!(db.all("projects").await.len(), 2);
        let emails: BTreeSet<String> = db
            .all("users")
            .await
            .iter()
            .map(|user| user["normalized_email"].as_str().unwrap().to_string())
            .collect();
        assert!(!emails.contains("old@example.com"));
        assert!(!emails.contains("incoming@example.com"));
    })
    .await;
}

#[tokio::test]
async fn operator_creation_leaves_the_owners_own_allowance_and_billing_unchanged() {
    with_isolated_postgres(|db| async move {
        link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        // Paid before: the standard entitlement row exists.
        activate_billing(&db, "paid@example.com", "workos_paid", "paid").await;
        // Signed in, never paid: no entitlement row yet.
        link(&db, "later@example.com", "workos_later").await;
        for email in ["paid@example.com", "later@example.com"] {
            let created = db
                .admin_create_agent_for_owner(input(email, &format!("op-{email}")))
                .await
                .unwrap();
            assert!(created.owner_linked);
            assert!(!created.record.customer_org_created);
            let org = db
                .row("customer_orgs", &created.record.customer_org_id)
                .await
                .unwrap();
            assert_eq!(org["billing_class"], "standard", "{email}");
        }
        activate_billing(&db, "later@example.com", "workos_later", "later").await;

        // Each can still create the one Agent their subscription allows.
        for (email, workos) in [
            ("paid@example.com", "workos_paid"),
            ("later@example.com", "workos_later"),
        ] {
            self_serve(&db, email, workos, "own-1").await.unwrap();
            let error = self_serve(&db, email, workos, "own-2").await.unwrap_err();
            assert!(
                matches!(error, CoreError::AgentCreationEntitlementExhausted),
                "{email}: {error:?}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn concurrent_retries_of_one_key_create_one_agent() {
    with_isolated_postgres(|db| async move {
        link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        let attempts =
            (0..4).map(|_| db.admin_create_agent_for_owner(input("race@example.com", "race")));
        let results = futures_util::future::join_all(attempts).await;
        let results: Vec<_> = results.into_iter().map(Result::unwrap).collect();
        assert_eq!(results.iter().filter(|result| !result.reused).count(), 1);
        assert!(
            results
                .iter()
                .all(|result| result.request.id == results[0].request.id)
        );
        assert_eq!(db.all("projects").await.len(), 1);
        assert_eq!(db.all("operator_agent_creations").await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn the_cli_preview_writes_nothing() {
    with_isolated_postgres(|db| async move {
        link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        let preview_store = CoreStore::connect_dry_run(&db.url).await.unwrap();
        let preview = preview_store
            .admin_create_agent_for_owner(input("preview@example.com", "preview"))
            .await
            .unwrap();
        assert!(preview.record.owner_account_created);
        assert!(db.all("operator_agent_creations").await.is_empty());
        assert!(db.all("projects").await.is_empty());
        assert_eq!(db.all("users").await.len(), 1);
    })
    .await;
}

/// Migration 0047 only adds its own table and index, reapplies at every
/// startup without touching rows, and its constraints hold the record to
/// what Core writes. Older Core never names the table, so a rollback keeps
/// operator-created Agents working as ordinary rows.
#[tokio::test]
async fn migration_0047_is_additive_reapplies_and_constrains_its_record() {
    let migration = include_str!("../../../migrations/0047_operator_agent_creations.sql");
    let statements: Vec<String> = migration
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
        .split(';')
        .map(|statement| statement.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|statement| !statement.is_empty())
        .collect();
    assert_eq!(statements.len(), 2, "{statements:?}");
    assert!(statements[0].starts_with("CREATE TABLE IF NOT EXISTS operator_agent_creations ("));
    assert!(statements[1].starts_with("CREATE INDEX IF NOT EXISTS operator_agent_creations_owner"));
    for forbidden in [
        "ALTER ",
        "DROP ",
        "UPDATE ",
        "DELETE ",
        "INSERT ",
        "TRUNCATE ",
    ] {
        assert!(!migration.to_uppercase().contains(forbidden), "{forbidden}");
    }
    assert!(CORE_SCHEMA_SQL.contains(migration));

    with_isolated_postgres(|db| async move {
        link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        db.admin_create_agent_for_owner(input("reapply@example.com", "reapply"))
            .await
            .unwrap();
        let before = (
            db.all("operator_agent_creations").await,
            db.all("agent_creation_requests").await,
            db.all("users").await,
        );
        let client = db.connection().await.unwrap();
        client.batch_execute(CORE_SCHEMA_SQL).await.unwrap();
        client.batch_execute(CORE_SCHEMA_SQL).await.unwrap();
        let after = (
            db.all("operator_agent_creations").await,
            db.all("agent_creation_requests").await,
            db.all("users").await,
        );
        assert_eq!(after, before);
        for (column, value) in [
            ("reason", "  "),
            ("admission", "free"),
            ("surface", "email"),
            ("owner_email", "Mixed@Case.example"),
            ("owner_chat_account_id", "not-hex"),
            ("agent_creation_request_id", "agent_request_missing"),
        ] {
            let result = client
                .batch_execute(&format!(
                    "UPDATE operator_agent_creations SET {column} = '{value}'"
                ))
                .await;
            assert!(result.is_err(), "{column} accepted {value}");
        }
    })
    .await;
}
