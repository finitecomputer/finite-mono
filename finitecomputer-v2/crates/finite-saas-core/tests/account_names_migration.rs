//! Upgrade from the deployed scoped-record schema without rewriting its rows.
use finite_saas_core::CORE_SCHEMA_SQL;
use futures_util::FutureExt;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio_postgres::{Client, NoTls};

const NAMES_SQL: &str = include_str!("../migrations/0043_account_names_and_key_aliases.sql");

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
    let admin_url = std::env::var("FC_CORE_POSTGRES_TEST_URL")
        .expect("FC_CORE_POSTGRES_TEST_URL is required for Core migration tests; run `just test`");

    let (admin, admin_connection) = tokio_postgres::connect(&admin_url, NoTls).await.unwrap();
    let admin_connection = tokio::spawn(async move {
        let _ = admin_connection.await;
    });
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let database = format!(
        "fc_names_migration_{unique}_{}",
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

async fn snapshot(client: &Client, table: &str) -> Vec<serde_json::Value> {
    client
        .query(
            &format!("SELECT to_jsonb(t) FROM {table} t ORDER BY id"),
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}

#[tokio::test]
async fn account_names_upgrade_and_reapply_preserve_existing_scoped_mappings() {
    with_legacy_database(|client| async move {
        let previous = CORE_SCHEMA_SQL.strip_suffix(NAMES_SQL).expect("0043 is the last migration");
        client.batch_execute(previous).await.unwrap();
        client.batch_execute(r#"
            INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at)
            VALUES ('owner', 'owner@example.test', 'linked', 'workos-owner', now(), now()),
                   ('viewer', 'viewer@example.test', 'linked', 'workos-viewer', now(), now());
            INSERT INTO customer_orgs (id, owner_user_id, name, billing_class, created_at, updated_at)
            VALUES ('org', 'owner', 'Existing org', 'standard', now(), now());
            INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, agent_email, hosting_tier, created_at, updated_at)
            VALUES ('project', 'org', 'owner', 'Existing Agent', 'existing@finite.vip', 'standard', now(), now());
            INSERT INTO brain_scoped_record_operations
                (id, operation_id, manifest_sha256, action, brain_server, brain_id,
                 operator_account_id, operator_workos_subject, operator_email, recorded_at, receipt)
            VALUES ('operation-row', 'existing-operation', repeat('a',64), 'apply', 'https://brain.test', 'existing',
                    'owner', 'workos-owner', 'owner@example.test', now(), '{"synthetic":"existing receipt"}');
            INSERT INTO brain_scoped_records
                (id, revision, brain_server, brain_id, public_key_hex, kind, account_id, workos_subject,
                 project_id, annotation_label, evidence_sha256, evidence_reference, evidence_observed_at,
                 creating_operation_id, created_at, retired_at)
            SELECT 'record-' || n, 'existing-revision-' || n, 'https://brain.test', 'existing', repeat(n::text,64),
                   CASE n WHEN 1 THEN 'account_chat_key' WHEN 2 THEN 'managed_agent'
                          WHEN 3 THEN 'historical_identity' ELSE 'invitation_key' END,
                   CASE WHEN n <= 2 THEN 'owner' END, CASE WHEN n <= 2 THEN 'workos-owner' END,
                   CASE WHEN n = 2 THEN 'project' END, CASE WHEN n >= 3 THEN 'Existing annotation' END,
                   repeat('b',64), 'synthetic/existing-mapping', '2026-10-01T00:00:00Z',
                   'existing-operation', now(), CASE WHEN n = 4 THEN now() END
            FROM generate_series(1,4) AS n;
            INSERT INTO account_brain_principals
                (id,user_id,public_key_hex,source,issuer,status,first_observed_at,last_observed_at,revision)
            VALUES ('principal','owner',repeat('5',64),'hosted_device_observation','synthetic','active',now(),now(),1);
            INSERT INTO account_brain_sharing_scopes
                (id,user_id,brain_server,brain_id,issuer,established_at,revision)
            VALUES ('scope','owner','https://brain.test','existing','synthetic',now(),1);
        "#).await.unwrap();
        let tables = ["brain_scoped_records", "brain_scoped_record_operations", "projects",
                      "account_brain_principals", "account_brain_sharing_scopes"];
        let mut before = Vec::new();
        for table in tables { before.push(snapshot(&client, table).await); }
        let users = snapshot(&client, "users").await;
        assert!(!client.query_one("SELECT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='users' AND column_name='workos_display_name')", &[]).await.unwrap().get::<_,bool>(0));
        client.batch_execute(NAMES_SQL).await.unwrap();
        for (mut actual, expected) in snapshot(&client, "users").await.into_iter().zip(users) {
            assert!(actual.as_object_mut().unwrap().remove("workos_display_name").unwrap().is_null());
            assert!(actual.as_object_mut().unwrap().remove("workos_display_name_updated_at").unwrap().is_null());
            assert_eq!(actual, expected, "upgrade does not rewrite existing users");
        }
        client.batch_execute(r#"
            UPDATE users SET workos_display_name='Sam Rivera', workos_display_name_updated_at=now() WHERE id='owner';
            INSERT INTO account_key_aliases VALUES ('viewer',repeat('1',64),'My colleague',now(),now());
        "#).await.unwrap();
        let names = snapshot(&client, "users").await;
        let aliases: serde_json::Value = client.query_one("SELECT jsonb_agg(t) FROM account_key_aliases t", &[]).await.unwrap().get(0);
        client.batch_execute(NAMES_SQL).await.unwrap();
        client.batch_execute(CORE_SCHEMA_SQL).await.unwrap();
        for (table, expected) in tables.into_iter().zip(before) {
            assert_eq!(snapshot(&client, table).await, expected, "preserve {table}");
        }
        assert_eq!(snapshot(&client, "users").await, names);
        assert_eq!(client.query_one("SELECT jsonb_agg(t) FROM account_key_aliases t", &[]).await.unwrap().get::<_,serde_json::Value>(0), aliases);
        // An older Core writer can still create accounts after the expansion.
        client.batch_execute("INSERT INTO users (id,normalized_email,link_status,workos_user_id,created_at,updated_at) VALUES ('old-writer','old@example.test','linked','workos-old',now(),now())").await.unwrap();
        assert!(client.query_one("SELECT workos_display_name IS NULL FROM users WHERE id='old-writer'", &[]).await.unwrap().get::<_,bool>(0));
    }).await;
}
