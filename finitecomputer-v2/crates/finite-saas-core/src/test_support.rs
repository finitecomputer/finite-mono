//! Shared Postgres harness for Core's tests.
//!
//! Core has exactly one store implementation, so every test runs against real
//! SQL. This module owns the per-test database lifecycle that used to live in
//! `store.rs`'s private test module, so `api.rs` and `lib.rs` tests can use it
//! too.

use crate::store::CoreStore;
use crate::{RuntimeArtifactKind, UpsertRuntimeArtifactInput};
use futures_util::FutureExt;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio_postgres::NoTls;

/// A migrated, isolated database plus its URL.
///
/// Derefs to the store so tests read `db.some_store_method(..)` directly.
pub(crate) struct TestDb {
    pub(crate) store: CoreStore,
    pub(crate) url: String,
}

impl std::ops::Deref for TestDb {
    type Target = CoreStore;
    fn deref(&self) -> &CoreStore {
        &self.store
    }
}

impl TestDb {
    /// Run a query and return each row's first column as JSON.
    ///
    /// Tests that used to read `BridgeCoreState`'s public maps directly need to
    /// inspect durable rows the store exposes no reader for. Selecting
    /// `to_jsonb(t)` keeps that to one helper instead of a typed accessor per
    /// entity, and avoids widening the production store API for tests.
    pub(crate) async fn query_json(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Vec<serde_json::Value> {
        let (client, connection) = tokio_postgres::connect(&self.url, NoTls).await.unwrap();
        let handle = tokio::spawn(async move {
            let _ = connection.await;
        });
        let rows = client
            .query(sql, params)
            .await
            .unwrap_or_else(|error| panic!("test query failed: {error}\n{sql}"));
        let out = rows
            .iter()
            .map(|row| row.get::<_, serde_json::Value>(0))
            .collect();
        drop(client);
        handle.abort();
        out
    }

    /// One row of `table` by primary key, as JSON.
    ///
    /// A few tables key on something other than `id`.
    pub(crate) async fn row(&self, table: &str, id: &str) -> Option<serde_json::Value> {
        let key = Self::primary_key(table);
        self.query_json(
            &format!("SELECT to_jsonb(t) FROM {table} t WHERE t.{key} = $1"),
            &[&id],
        )
        .await
        .into_iter()
        .next()
    }

    /// Every row of `table`, as JSON, ordered by primary key for determinism.
    pub(crate) async fn all(&self, table: &str) -> Vec<serde_json::Value> {
        let key = Self::primary_key(table);
        self.query_json(
            &format!("SELECT to_jsonb(t) FROM {table} t ORDER BY t.{key}"),
            &[],
        )
        .await
    }

    fn primary_key(table: &str) -> &'static str {
        match table {
            "runtime_retirement_snapshots" => "request_id",
            "runtime_relay_credentials" => "agent_runtime_id",
            _ => "id",
        }
    }
}

static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Swap the database name in a `postgres://user:pass@host:port/db?query` URL,
/// preserving auth, host, and any query string.
fn replace_database(url: &str, db_name: &str) -> String {
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (url, None),
    };
    let scheme_end = base.find("://").map(|idx| idx + 3).unwrap_or(0);
    let new_base = match base[scheme_end..].find('/') {
        Some(rel) => format!("{}/{db_name}", &base[..scheme_end + rel]),
        None => format!("{base}/{db_name}"),
    };
    match query {
        Some(query) => format!("{new_base}?{query}"),
        None => new_base,
    }
}

/// Run `test` against an isolated, migrated Postgres database.
///
/// Each test gets its OWN freshly-created database, migrated from the schema
/// and dropped afterward. The agent-creation lease queue is global (the
/// `WHERE status = 'requested' ... ORDER BY created_at` scan in
/// `postgres_lease_agent_creation_request` picks the oldest row across ALL
/// orgs), so a shared database would let one test's leftover request be leased
/// by another. Per-test databases keep tests independent and parallel.
///
/// The database is dropped even if the test body panics; the panic is re-raised
/// so the test still fails. `just test` supplies the required maintenance
/// connection through devfinity; running this suite without that
/// infrastructure is an error.
pub(crate) async fn with_isolated_postgres<F, Fut>(test: F)
where
    F: FnOnce(TestDb) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let admin_url = std::env::var("FC_CORE_POSTGRES_TEST_URL")
        .expect("FC_CORE_POSTGRES_TEST_URL is required for Core Postgres tests; run `just test`");

    // Maintenance connection used only to CREATE/DROP the per-test database.
    let (admin, admin_conn) = tokio_postgres::connect(&admin_url, NoTls).await.unwrap();
    let admin_conn = tokio::spawn(async move {
        let _ = admin_conn.await;
    });

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let db_name = format!(
        "fc_test_{unique}_{}",
        TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    admin
        .execute(&format!("CREATE DATABASE \"{db_name}\""), &[])
        .await
        .unwrap();

    let url = replace_database(&admin_url, &db_name);
    let store = CoreStore::connect(&url).await.unwrap();
    store.migrate().await.unwrap();
    // Every creation test exercises the current Core contract: a request is
    // bound to an exact, promoted OCI artifact before it can lease. Keep this
    // older than test-specific promotions so focused artifact tests still
    // select their own fixture.
    store
        .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
            id: "artifact-postgres-fixture".to_string(),
            kind: RuntimeArtifactKind::OciImage,
            reference: format!(
                "ghcr.io/finitecomputer/agent-runtime:postgres-fixture@sha256:{}",
                "f".repeat(64)
            ),
            version_label: "postgres-fixture".to_string(),
            source_git_sha: None,
            finitec_version: None,
            hermes_source_ref: None,
            finite_platform_plugin_ref: None,
            state_schema_version: "state-v1".to_string(),
            base_image: Some("python:3.11-trixie".to_string()),
            recover_known_good_chat: false,
            promoted: true,
            now: Some("2000-01-01T00:00:00Z".to_string()),
        })
        .await
        .unwrap();

    // Capture panics so the database is always torn down, then re-raise.
    let outcome = std::panic::AssertUnwindSafe(test(TestDb { store, url }))
        .catch_unwind()
        .await;

    // FORCE terminates any lingering connection (Postgres 13+), so teardown
    // never races the store/raw clients the test opened.
    let _ = admin
        .execute(
            &format!("DROP DATABASE IF EXISTS \"{db_name}\" WITH (FORCE)"),
            &[],
        )
        .await;
    drop(admin);
    admin_conn.abort();

    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
