//! Store-level guards of operator creation (migration 0047): exact admin
//! identity, the owner's provisioned sign-in identity, fail-closed owner
//! refusals, concurrent creations, keys and the rollback-only preview.

use super::*;
use crate::owner_identity::test_support::test_chat_account_id;
use crate::{
    OperatorCreateAgentInput, OperatorCreationSurface, OperatorIdentity, OwnerSignInIdentity,
};

pub(super) const ADMIN_EMAIL: &str = "operator@finite.vip";
pub(super) const ADMIN_WORKOS: &str = "workos_operator";

pub(super) async fn link(db: &TestDb, email: &str, workos_user_id: &str) -> CoreUser {
    db.link_verified_user(LinkVerifiedUserInput {
        verified_email: email.to_string(),
        workos_user_id: workos_user_id.to_string(),
        now: None,
    })
    .await
    .unwrap()
}

/// The WorkOS user a test owner signs in as.
pub(super) fn owner_workos(email: &str) -> String {
    format!("user_owner_{}", email.replace(['@', '.'], "_"))
}

pub(super) fn identity(workos_user_id: &str) -> OwnerSignInIdentity {
    OwnerSignInIdentity {
        workos_user_id: workos_user_id.to_string(),
        chat_account_id: test_chat_account_id(workos_user_id),
    }
}

/// A creation as the provisioning step hands it to the store.
pub(super) fn input(owner_email: &str, key: &str) -> OperatorCreateAgentInput {
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
        owner_identity: Some(identity(&owner_workos(owner_email.trim()))),
        existing_account_confirmed: false,
        cli_os_user: Some("austin".to_string()),
        profile_picture_url: None,
        placement_override: None,
        now: None,
    }
}

/// A creation for an owner who signed in as `workos`, confirmed.
pub(super) fn confirmed(owner_email: &str, workos: &str, key: &str) -> OperatorCreateAgentInput {
    OperatorCreateAgentInput {
        owner_identity: Some(identity(workos)),
        existing_account_confirmed: true,
        ..input(owner_email, key)
    }
}

pub(super) fn refusal(error: CoreError) -> &'static str {
    match error {
        CoreError::OperatorAgentCreationRefused(message) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

pub(super) async fn org_of(db: &TestDb, email: &str) -> String {
    db.query_json(
        "SELECT to_jsonb(o.id) FROM customer_orgs o JOIN users u ON u.id = o.owner_user_id
         WHERE u.normalized_email = $1",
        &[&email],
    )
    .await[0]
        .as_str()
        .unwrap()
        .to_string()
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
        assert_eq!(created.record.actor.admin_user_id, admin.id);
        assert_eq!(created.record.actor.surface, OperatorCreationSurface::Cli);
        assert_eq!(created.record.actor.cli_os_user.as_deref(), Some("austin"));
        assert_eq!(
            created.request.target_source_host_id.as_deref(),
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

/// Open chat admission is never allowed: a committing store refuses any
/// creation without the owner's provisioned WorkOS user and chat account,
/// and the chat account it gets is the one the lease pins.
#[tokio::test]
async fn a_committed_creation_always_carries_the_owner_identity() {
    with_isolated_postgres(|db| async move {
        link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        let mut missing = input("lock@example.com", "lock");
        missing.owner_identity = None;
        let error = db.admin_create_agent_for_owner(missing).await.unwrap_err();
        assert!(
            matches!(error, CoreError::OperatorOwnerIdentityRequired),
            "{error:?}"
        );
        for bad in [
            OwnerSignInIdentity {
                workos_user_id: "  ".into(),
                chat_account_id: "ab".repeat(32),
            },
            OwnerSignInIdentity {
                workos_user_id: "user_x".into(),
                chat_account_id: String::new(),
            },
        ] {
            let mut bad_input = input("lock@example.com", "lock");
            bad_input.owner_identity = Some(bad);
            assert!(db.admin_create_agent_for_owner(bad_input).await.is_err());
        }
        assert!(db.all("operator_agent_creations").await.is_empty());
        assert_eq!(db.all("users").await.len(), 1, "nothing was written");

        let created = db
            .admin_create_agent_for_owner(input("lock@example.com", "lock"))
            .await
            .unwrap();
        let chat = test_chat_account_id(&owner_workos("lock@example.com"));
        assert!(created.owner_identity_provisioned);
        assert_eq!(created.record.owner.chat_account_id, chat);
        assert_eq!(
            created.record.owner.workos_user_id,
            owner_workos("lock@example.com")
        );
        assert_eq!(
            created.request.owner_chat_account_id.as_deref(),
            Some(chat.as_str())
        );
        let lease = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-lock".into(),
                lease_token: "lease-lock".into(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                source_host_id: None,
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        let RuntimeSpecEnvelope::V1(spec) = lease.request.runtime_spec.unwrap();
        assert_eq!(
            spec.environment
                .get("FINITECHAT_OWNER_NPUBS")
                .map(String::as_str),
            Some(chat.as_str())
        );
    })
    .await;
}

#[tokio::test]
async fn a_signed_in_account_needs_confirmation_and_its_own_workos_user() {
    with_isolated_postgres(|db| async move {
        link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        link(&db, "known@example.com", "workos_known").await;
        let mut unconfirmed = confirmed("known@example.com", "workos_known", "known");
        unconfirmed.existing_account_confirmed = false;
        let plan_error = db
            .operator_owner_identity_plan(&unconfirmed)
            .await
            .unwrap_err();
        assert!(refusal(plan_error).contains("confirm"));
        let error = db
            .admin_create_agent_for_owner(unconfirmed)
            .await
            .unwrap_err();
        assert!(refusal(error).contains("confirm"));
        // Confirmed, but bound to a WorkOS user the person does not sign in as.
        let error = db
            .admin_create_agent_for_owner(confirmed("known@example.com", "workos_other", "known"))
            .await
            .unwrap_err();
        assert!(refusal(error).contains("different WorkOS user"));
        assert!(db.all("operator_agent_creations").await.is_empty());

        let created = db
            .admin_create_agent_for_owner(confirmed("known@example.com", "workos_known", "known"))
            .await
            .unwrap();
        assert!(created.owner_linked);
        assert!(created.record.owner.existing_account_confirmed);
        assert!(!created.record.owner.account_created);
        // A pending account needs no confirmation.
        db.admin_create_agent_for_owner(input("pending@example.com", "pending"))
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn a_pending_owner_keeps_one_workos_user_no_one_else_holds() {
    with_isolated_postgres(|db| async move {
        link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        link(&db, "holder@example.com", "workos_held").await;
        let mut taken = input("pending@example.com", "taken");
        taken.owner_identity = Some(identity("workos_held"));
        let error = db.admin_create_agent_for_owner(taken).await.unwrap_err();
        assert!(refusal(error).contains("another Core account"));

        db.admin_create_agent_for_owner(input("pending@example.com", "first"))
            .await
            .unwrap();
        let mut second = input("pending@example.com", "second");
        second.owner_identity = Some(identity("workos_someone_new"));
        let error = db.admin_create_agent_for_owner(second).await.unwrap_err();
        assert!(refusal(error).contains("different WorkOS user"));
        // A retry of the first key with another identity is refused too.
        let mut retry = input("pending@example.com", "first");
        retry.owner_identity = Some(identity("workos_someone_new"));
        let error = db.admin_create_agent_for_owner(retry).await.unwrap_err();
        assert!(refusal(error).contains("different operator creation"));
        assert_eq!(db.all("operator_agent_creations").await.len(), 1);
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
        link(&db, "trial@example.com", "workos_trial").await;
        let trial_org = org_of(&db, "trial@example.com").await;
        db.query_json(
            "WITH campaign AS (
               INSERT INTO trial_campaigns (id, name, code_hash, seat_limit, created_by_workos_user_id)
               VALUES ('campaign', 'Workshop', 'hash', 5, 'w') RETURNING id)
             INSERT INTO trial_redemptions
               (id, campaign_id, customer_org_id, attempt_id, stripe_session_id, state, checkout_expires_at, redeemed_at)
             SELECT 'redemption', id, $1, 'attempt', 'cs_trial', 'reserved', NOW(), NULL FROM campaign
             RETURNING to_jsonb(id)",
            &[&trial_org],
        )
        .await;
        let trial = || confirmed("trial@example.com", "workos_trial", "trial");
        let error = db.admin_create_agent_for_owner(trial()).await.unwrap_err();
        assert!(refusal(error).contains("event trial"));
        // An expired checkout is not a trial: the same person is accepted.
        db.query_json(
            "UPDATE trial_redemptions SET state = 'expired' RETURNING to_jsonb(id)",
            &[],
        )
        .await;
        db.admin_create_agent_for_owner(trial()).await.unwrap();

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

/// Distinct keys racing for one new email: exactly one record says it
/// created the account and the org, because only one INSERT did.
#[tokio::test]
async fn concurrent_creations_for_one_new_email_record_one_account_creation() {
    with_isolated_postgres(|db| async move {
        link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        let attempts = (0..8).map(|n| {
            db.admin_create_agent_for_owner(input("audit-race@example.com", &format!("audit-{n}")))
        });
        let results: Vec<_> = futures_util::future::join_all(attempts)
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect();
        assert_eq!(results.len(), 8);
        let owners: BTreeSet<_> = results
            .iter()
            .map(|r| r.record.owner.user_id.clone())
            .collect();
        assert_eq!(owners.len(), 1);
        let count = |pick: fn(&crate::OperatorCreationOwner) -> bool| {
            results.iter().filter(|r| pick(&r.record.owner)).count()
        };
        assert_eq!(count(|owner| owner.account_created), 1);
        assert_eq!(count(|owner| owner.customer_org_created), 1);
        let stored = db.all("operator_agent_creations").await;
        assert_eq!(
            stored
                .iter()
                .filter(|row| row["owner_account_created"] == true)
                .count(),
            1
        );
    })
    .await;
}

#[tokio::test]
async fn operator_keys_are_refused_when_long_and_never_shared_with_self_serve() {
    with_isolated_postgres(|db| async move {
        link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        let long = "k".repeat(129);
        let error = db
            .admin_create_agent_for_owner(input("keys@example.com", &long))
            .await
            .unwrap_err();
        assert!(matches!(error, CoreError::InvalidOperatorIdempotencyKey), "{error:?}");
        let created = db
            .admin_create_agent_for_owner(input("keys@example.com", &"k".repeat(128)))
            .await
            .unwrap();
        assert_eq!(created.record.idempotency_key.len(), 128);

        // Self-serve can never take a key in the operator namespace.
        let owner = link(&db, "keys@example.com", &owner_workos("keys@example.com")).await;
        for key in ["operator:box1", "OPERATOR:box1", " operator:box1"] {
            let error = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "keys@example.com".into(),
                    workos_user_id: owner_workos("keys@example.com"),
                    display_name: "Own".into(),
                    launch_code: String::new(),
                    idempotency_key: key.into(),
                    now: None,
                })
                .await
                .unwrap_err();
            assert!(
                matches!(error, CoreError::ReservedAgentCreationIdempotencyKey),
                "{key}: {error:?}"
            );
        }
        // A request older Core let the owner write with such a key is a
        // clear conflict, not a database error.
        let org = org_of(&db, "keys@example.com").await;
        db.query_json(
            "INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, created_at, updated_at)
             VALUES ('project_legacy', $1, $2, 'Legacy', NOW(), NOW()) RETURNING to_jsonb(id)",
            &[&org, &owner.id],
        )
        .await;
        db.query_json(
            "INSERT INTO agent_creation_requests
               (id, customer_org_id, owner_user_id, project_id, idempotency_key, display_name,
                runner_class, status, created_at, updated_at)
             VALUES ('request_legacy', $1, $2, 'project_legacy', 'operator:box1-legacy', 'Legacy',
                     'kata', 'failed', NOW(), NOW())
             RETURNING to_jsonb(id)",
            &[&org, &owner.id],
        )
        .await;
        let error = db
            .admin_create_agent_for_owner(confirmed(
                "keys@example.com",
                &owner_workos("keys@example.com"),
                "box1-legacy",
            ))
            .await
            .unwrap_err();
        assert!(refusal(error).contains("operator key"));
    })
    .await;
}

#[tokio::test]
async fn the_preview_store_writes_nothing_and_never_needs_an_identity() {
    with_isolated_postgres(|db| async move {
        link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        let preview_store = CoreStore::connect_dry_run(&db.url).await.unwrap();
        let mut unprovisioned = input("preview@example.com", "preview");
        unprovisioned.owner_identity = None;
        let preview = preview_store
            .admin_create_agent_for_owner(unprovisioned)
            .await
            .unwrap();
        assert!(preview.record.owner.account_created);
        assert!(!preview.owner_identity_provisioned);
        assert!(db.all("operator_agent_creations").await.is_empty());
        assert!(db.all("projects").await.is_empty());
        assert_eq!(db.all("users").await.len(), 1);
    })
    .await;
}

/// Migration 0047 only adds its own table and indexes, reapplies at every
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
    assert_eq!(statements.len(), 3, "{statements:?}");
    assert!(statements[0].starts_with("CREATE TABLE IF NOT EXISTS operator_agent_creations ("));
    assert!(statements[1].starts_with("CREATE INDEX IF NOT EXISTS operator_agent_creations_owner"));
    assert!(statements[2].starts_with("CREATE INDEX IF NOT EXISTS operator_agent_creations_org"));
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
            ("reason", "'  '"),
            ("admission", "'free'"),
            ("surface", "'email'"),
            ("owner_email", "'Mixed@Case.example'"),
            ("owner_chat_account_id", "'not-hex'"),
            ("owner_chat_account_id", "NULL"),
            ("owner_workos_user_id", "' '"),
            ("cli_os_user", "NULL"),
            ("agent_creation_request_id", "'agent_request_missing'"),
        ] {
            let result = client
                .batch_execute(&format!(
                    "UPDATE operator_agent_creations SET {column} = {value}"
                ))
                .await;
            assert!(result.is_err(), "{column} accepted {value}");
        }
    })
    .await;
}
