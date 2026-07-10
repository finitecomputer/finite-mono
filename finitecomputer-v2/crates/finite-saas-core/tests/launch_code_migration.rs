use futures_util::FutureExt;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio_postgres::{Client, NoTls, error::SqlState};

const CORE_SCHEMA_V1: &str = include_str!("../migrations/0001_core.sql");
const RUNTIME_UPGRADE_V2: &str = include_str!("../migrations/0002_runtime_upgrade.sql");
const LAUNCH_CODES_V3: &str = include_str!("../migrations/0003_launch_codes.sql");

static TEST_DATABASE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn replace_database(url: &str, database: &str) -> String {
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (url, None),
    };
    let scheme_end = base.find("://").map(|index| index + 3).unwrap_or(0);
    let replaced = match base[scheme_end..].find('/') {
        Some(relative_index) => format!("{}/{database}", &base[..scheme_end + relative_index]),
        None => format!("{base}/{database}"),
    };
    match query {
        Some(query) => format!("{replaced}?{query}"),
        None => replaced,
    }
}

async fn with_legacy_database<F, Fut>(test: F)
where
    F: FnOnce(Client) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let Ok(admin_url) = std::env::var("FC_CORE_POSTGRES_TEST_URL") else {
        return;
    };

    let (admin, admin_connection) = tokio_postgres::connect(&admin_url, NoTls).await.unwrap();
    let admin_connection = tokio::spawn(async move {
        let _ = admin_connection.await;
    });
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let database = format!(
        "fc_launch_migration_{unique}_{}",
        TEST_DATABASE_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    admin
        .execute(&format!("CREATE DATABASE \"{database}\""), &[])
        .await
        .unwrap();

    let test_url = replace_database(&admin_url, &database);
    let (client, connection) = tokio_postgres::connect(&test_url, NoTls).await.unwrap();
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });
    let outcome = std::panic::AssertUnwindSafe(test(client))
        .catch_unwind()
        .await;

    connection.abort();
    let _ = admin
        .execute(
            &format!("DROP DATABASE IF EXISTS \"{database}\" WITH (FORCE)"),
            &[],
        )
        .await;
    drop(admin);
    admin_connection.abort();

    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
async fn populated_legacy_schema_migrates_launch_codes_without_retaining_plaintext() {
    with_legacy_database(|client| async move {
        client.batch_execute(CORE_SCHEMA_V1).await.unwrap();
        client.batch_execute(RUNTIME_UPGRADE_V2).await.unwrap();
        client
            .batch_execute(
                r#"
                INSERT INTO users (
                  id, normalized_email, link_status, workos_user_id, created_at, updated_at
                ) VALUES (
                  'user-legacy', 'legacy@example.test', 'linked', 'workos-legacy', now(), now()
                );
                INSERT INTO customer_orgs (
                  id, owner_user_id, name, billing_class, created_at, updated_at
                ) VALUES (
                  'org-legacy', 'user-legacy', 'Legacy org', 'off2026', now(), now()
                );
                INSERT INTO projects (
                  id, customer_org_id, owner_user_id, display_name, created_at, updated_at
                ) VALUES (
                  'project-legacy', 'org-legacy', 'user-legacy', 'Legacy project', now(), now()
                );
                INSERT INTO agent_creation_entitlements (
                  id, customer_org_id, allowed_new_agent_runtimes, launch_code, created_at, updated_at
                ) VALUES (
                  'entitlement-legacy', 'org-legacy', 1, 'off2026', now(), now()
                );
                INSERT INTO agent_creation_requests (
                  id, customer_org_id, owner_user_id, project_id, idempotency_key,
                  display_name, runner_class, status, requested_launch_code, created_at, updated_at
                ) VALUES (
                  'request-legacy', 'org-legacy', 'user-legacy', 'project-legacy', 'legacy-request',
                  'Legacy agent', 'kata', 'requested', 'off2026', now(), now()
                );
                "#,
            )
            .await
            .unwrap();

        client.batch_execute(LAUNCH_CODES_V3).await.unwrap();

        let migrated = client
            .query_one(
                "SELECT billing_class FROM customer_orgs WHERE id = 'org-legacy'",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(migrated.get::<_, String>(0), "sponsored");
        let entitlement = client
            .query_one(
                "SELECT launch_code FROM agent_creation_entitlements WHERE id = 'entitlement-legacy'",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(entitlement.get::<_, Option<String>>(0), None);
        let request = client
            .query_one(
                "SELECT requested_launch_code FROM agent_creation_requests WHERE id = 'request-legacy'",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(request.get::<_, Option<String>>(0), None);

        let invalid_entitlement_reference = client
            .execute(
                "UPDATE agent_creation_entitlements SET launch_code = 'missing-code' WHERE id = 'entitlement-legacy'",
                &[],
            )
            .await
            .unwrap_err();
        assert_eq!(invalid_entitlement_reference.code(), Some(&SqlState::FOREIGN_KEY_VIOLATION));
        let invalid_request_reference = client
            .execute(
                "UPDATE agent_creation_requests SET requested_launch_code = 'missing-code' WHERE id = 'request-legacy'",
                &[],
            )
            .await
            .unwrap_err();
        assert_eq!(invalid_request_reference.code(), Some(&SqlState::FOREIGN_KEY_VIOLATION));

        client
            .batch_execute(
                r#"
                INSERT INTO users (
                  id, normalized_email, link_status, workos_user_id, created_at, updated_at
                ) VALUES (
                  'user-second', 'second@example.test', 'linked', 'workos-second', now(), now()
                );
                INSERT INTO customer_orgs (
                  id, owner_user_id, name, billing_class, created_at, updated_at
                ) VALUES (
                  'org-second', 'user-second', 'Second org', 'sponsored', now(), now()
                );
                INSERT INTO agent_creation_entitlements (
                  id, customer_org_id, allowed_new_agent_runtimes, created_at, updated_at
                ) VALUES (
                  'entitlement-second', 'org-second', 1, now(), now()
                );
                INSERT INTO launch_code_batches (
                  id, name, code_count, expires_at, created_by_workos_user_id, created_at
                ) VALUES (
                  'batch-one', 'Migration constraint test', 1, now() + interval '1 day',
                  'workos-operator', now()
                );
                INSERT INTO launch_codes (id, batch_id, code_hash, created_at)
                VALUES ('code-one', 'batch-one', 'test-hash-not-plaintext', now());
                UPDATE agent_creation_entitlements
                  SET launch_code = 'code-one'
                  WHERE id = 'entitlement-legacy';
                "#,
            )
            .await
            .unwrap();
        let duplicate_code_association = client
            .execute(
                "UPDATE agent_creation_entitlements SET launch_code = 'code-one' WHERE id = 'entitlement-second'",
                &[],
            )
            .await
            .unwrap_err();
        assert_eq!(duplicate_code_association.code(), Some(&SqlState::UNIQUE_VIOLATION));

        client.batch_execute(LAUNCH_CODES_V3).await.unwrap();
        let still_migrated = client
            .query_one(
                "SELECT billing_class, launch_code FROM customer_orgs JOIN agent_creation_entitlements ON customer_org_id = customer_orgs.id WHERE customer_orgs.id = 'org-legacy'",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(still_migrated.get::<_, String>(0), "sponsored");
        assert_eq!(
            still_migrated.get::<_, Option<String>>(1).as_deref(),
            Some("code-one")
        );
    })
    .await;
}
