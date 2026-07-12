use futures_util::FutureExt;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio_postgres::{Client, NoTls, error::SqlState};

const CORE_SCHEMA_V1: &str = include_str!("../migrations/0001_core.sql");
const RUNTIME_UPGRADE_V2: &str = include_str!("../migrations/0002_runtime_upgrade.sql");
const LAUNCH_CODES_V3: &str = include_str!("../migrations/0003_launch_codes.sql");
const MEMBERSHIP_ARCHIVE_V4: &str = include_str!("../migrations/0004_membership_archive.sql");
const PHALA_EXPAND_V5: &str = include_str!("../migrations/0005_phala_expand.sql");

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

#[tokio::test]
async fn phala_expand_backfills_standard_without_moving_running_placement() {
    with_legacy_database(|client| async move {
        client.batch_execute(CORE_SCHEMA_V1).await.unwrap();
        client.batch_execute(RUNTIME_UPGRADE_V2).await.unwrap();
        client.batch_execute(LAUNCH_CODES_V3).await.unwrap();
        client.batch_execute(MEMBERSHIP_ARCHIVE_V4).await.unwrap();
        client
            .batch_execute(
                r#"
                INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at)
                VALUES ('user-expand', 'expand@example.test', 'linked', 'workos-expand', now(), now());
                INSERT INTO customer_orgs (id, owner_user_id, name, billing_class, created_at, updated_at)
                VALUES ('org-expand', 'user-expand', 'Expand org', 'standard', now(), now());
                INSERT INTO customer_billing_accounts (
                  customer_org_id, stripe_customer_id, subscription_status, created_at, updated_at
                ) VALUES ('org-expand', 'cus_expand', 'active', now(), now());
                INSERT INTO agent_creation_entitlements (
                  id, customer_org_id, allowed_new_agent_runtimes, created_at, updated_at
                ) VALUES ('entitlement-expand', 'org-expand', 2, now(), now());
                INSERT INTO launch_code_batches (
                  id, name, code_count, expires_at, created_by_workos_user_id, created_at
                ) VALUES ('batch-expand', 'Expand batch', 1, now() + interval '1 day', 'operator-expand', now());
                INSERT INTO launch_codes (id, batch_id, code_hash, created_at)
                VALUES ('code-expand', 'batch-expand', 'expand-hash', now());

                INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, created_at, updated_at)
                VALUES
                  ('project-running', 'org-expand', 'user-expand', 'Running Phala', now(), now()),
                  ('project-unlaunched', 'org-expand', 'user-expand', 'Unlaunched default', now(), now());
                INSERT INTO agent_runtimes (
                  id, project_id, source_host_id, source_machine_id, source_import_key,
                  host_facts, created_at, updated_at
                ) VALUES (
                  'runtime-running', 'project-running', 'phala-host', 'phala-machine',
                  'phala-host:phala-machine',
                  '{"display_name":"Running Phala","hostname":null,"runtime_host":"phala-host","runtime_status":"online","active_inference_profile":null,"hermes_available":true,"published_app_urls":[]}'::jsonb,
                  now(), now()
                );
                INSERT INTO agent_creation_requests (
                  id, customer_org_id, owner_user_id, project_id, idempotency_key,
                  display_name, runner_class, status, agent_runtime_id, created_at, updated_at
                ) VALUES
                  ('request-running', 'org-expand', 'user-expand', 'project-running', 'running',
                   'Running Phala', 'phala', 'running', 'runtime-running', now(), now()),
                  ('request-unlaunched', 'org-expand', 'user-expand', 'project-unlaunched', 'unlaunched',
                   'Unlaunched default', 'phala', 'requested', NULL, now(), now());
                "#,
            )
            .await
            .unwrap();

        client.batch_execute(PHALA_EXPAND_V5).await.unwrap();

        let running = client
            .query_one(
                "SELECT hosting_tier, placement_runner_class, runtime_resource_class
                 FROM agent_creation_requests WHERE id = 'request-running'",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(running.get::<_, String>(0), "standard");
        assert_eq!(running.get::<_, String>(1), "phala");
        assert_eq!(running.get::<_, String>(2), "vcpu2_memory4_gib");

        let unlaunched = client
            .query_one(
                "SELECT runner_class, placement_runner_class, runtime_resource_class
                 FROM agent_creation_requests WHERE id = 'request-unlaunched'",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(unlaunched.get::<_, String>(0), "kata");
        assert_eq!(unlaunched.get::<_, String>(1), "kata");
        assert_eq!(unlaunched.get::<_, String>(2), "vcpu4_memory8_gib");

        let runtime = client
            .query_one(
                "SELECT placement_runner_class, runtime_resource_class
                 FROM agent_runtimes WHERE id = 'runtime-running'",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(runtime.get::<_, String>(0), "phala");
        assert_eq!(runtime.get::<_, String>(1), "vcpu2_memory4_gib");

        for (table, id_column, id) in [
            ("customer_billing_accounts", "customer_org_id", "org-expand"),
            ("agent_creation_entitlements", "id", "entitlement-expand"),
            ("launch_code_batches", "id", "batch-expand"),
        ] {
            let query = format!("SELECT hosting_tier FROM {table} WHERE {id_column} = $1");
            let row = client.query_one(&query, &[&id]).await.unwrap();
            assert_eq!(row.get::<_, String>(0), "standard");
        }

        let default = client
            .query_one(
                "SELECT column_default FROM information_schema.columns
                 WHERE table_name = 'agent_creation_requests' AND column_name = 'runner_class'",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(default.get::<_, Option<String>>(0), None);

        let unknown_tier = client
            .execute(
                "UPDATE projects SET hosting_tier = 'mystery' WHERE id = 'project-running'",
                &[],
            )
            .await
            .unwrap_err();
        assert_eq!(unknown_tier.code(), Some(&SqlState::CHECK_VIOLATION));
        let incomplete_placement = client
            .execute(
                "UPDATE projects SET runtime_resource_class = NULL WHERE id = 'project-running'",
                &[],
            )
            .await
            .unwrap_err();
        assert_eq!(incomplete_placement.code(), Some(&SqlState::CHECK_VIOLATION));

        // Expansion stays compatible with an N-1 writer that supplies the
        // legacy runner_class but knows nothing about the nullable new fields.
        client
            .batch_execute(
                r#"
                INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, created_at, updated_at)
                VALUES ('project-n-minus-one', 'org-expand', 'user-expand', 'N-1', now(), now());
                INSERT INTO agent_creation_requests (
                  id, customer_org_id, owner_user_id, project_id, idempotency_key,
                  display_name, runner_class, status, created_at, updated_at
                ) VALUES (
                  'request-n-minus-one', 'org-expand', 'user-expand', 'project-n-minus-one', 'n-minus-one',
                  'N-1', 'kata', 'requested', now(), now()
                );
                "#,
            )
            .await
            .unwrap();
        let old_row = client
            .query_one(
                "SELECT runner_class, placement_runner_class, runtime_spec
                 FROM agent_creation_requests WHERE id = 'request-n-minus-one'",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(old_row.get::<_, String>(0), "kata");
        assert_eq!(old_row.get::<_, Option<String>>(1), None);
        assert_eq!(old_row.get::<_, Option<serde_json::Value>>(2), None);

        client.batch_execute(PHALA_EXPAND_V5).await.unwrap();
    })
    .await;
}
