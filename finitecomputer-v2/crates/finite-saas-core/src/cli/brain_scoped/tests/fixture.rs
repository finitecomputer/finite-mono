use super::*;
use futures_util::FutureExt;
use tokio_postgres::{Client, NoTls};

pub(super) const SERVER: &str = "https://brain.example.test";

pub(super) struct Database {
    pub client: Client,
    url: String,
}

impl Database {
    pub async fn connection(&self) -> Client {
        connect(&self.url).await
    }
    pub async fn run(&self, input: &ScopedManifest, execute: bool) -> Result<()> {
        run(input, execute).await
    }
    pub async fn count(&self, table: &str) -> i64 {
        self.client
            .query_one(&format!("SELECT count(*) FROM {table}"), &[])
            .await
            .unwrap()
            .get(0)
    }
    pub async fn receipt(&self, operation: &str) -> Value {
        self.client
            .query_one(
                "SELECT receipt FROM brain_scoped_record_operations WHERE operation_id=$1",
                &[&operation],
            )
            .await
            .unwrap()
            .get(0)
    }
    pub async fn unrelated_rows(&self) -> BTreeMap<String, Value> {
        let mut rows = BTreeMap::new();
        for table in self.client.query(
            "SELECT tablename FROM pg_tables WHERE schemaname='public' AND tablename NOT IN ('brain_scoped_records','brain_scoped_record_operations') ORDER BY tablename", &[]
        ).await.unwrap() {
            let name: String = table.get(0);
            let contents = self.client.query_one(&format!(
                "SELECT COALESCE(jsonb_agg(v ORDER BY v::text), '[]') FROM (SELECT to_jsonb(t) v FROM \"{name}\" t) x"
            ), &[]).await.unwrap().get(0);
            rows.insert(name, contents);
        }
        rows
    }
    pub async fn reject_ddl(&self) {
        self.client.batch_execute(
            "CREATE FUNCTION reject_scoped_ddl() RETURNS event_trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'scoped command attempted DDL'; END $$;
             CREATE EVENT TRIGGER reject_scoped_ddl ON ddl_command_start EXECUTE FUNCTION reject_scoped_ddl();"
        ).await.unwrap();
    }
}

async fn connect(url: &str) -> Client {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

pub(super) async fn with_database<F, Fut>(migrated: bool, test: F)
where
    F: FnOnce(Database) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let base = env::var("FC_CORE_POSTGRES_TEST_URL")
        .expect("FC_CORE_POSTGRES_TEST_URL is required; run through devfinity");
    let _guard = crate::cli::tests::DB_ENV_LOCK.lock().await;
    let admin = connect(&base).await;
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("fc_scoped_cli_{unique}");
    admin
        .batch_execute(&format!("CREATE DATABASE \"{name}\""))
        .await
        .unwrap();
    let mut url = reqwest::Url::parse(&base).unwrap();
    url.set_path(&format!("/{name}"));
    let url = url.to_string();
    let variables = [
        ("FC_CORE_DATABASE_URL", url.as_str()),
        ("FC_CORE_BRAIN_IDENTITY_BRAIN_SERVER", SERVER),
        ("FC_CORE_POSTGRES_CONNECT_TIMEOUT_SECS", "1"),
    ];
    let previous: Vec<_> = variables
        .iter()
        .map(|(key, _)| (*key, env::var_os(key)))
        .collect();
    // All command tests share DB_ENV_LOCK. Restore even on a failed assertion.
    for (key, value) in variables {
        unsafe { env::set_var(key, value) };
    }
    let outcome = std::panic::AssertUnwindSafe(async {
        if migrated {
            CoreStore::connect(&url).await.unwrap().migrate().await.unwrap();
        }
        let client = connect(&url).await;
        if migrated {
            client.batch_execute("INSERT INTO users (id,normalized_email,workos_user_id,link_status,created_at,updated_at)
                VALUES ('operator','operator@example.test','workos-operator','linked',now(),now())").await.unwrap();
        }
        test(Database { client, url }).await;
    }).catch_unwind().await;
    for (key, value) in previous {
        unsafe {
            if let Some(value) = value {
                env::set_var(key, value);
            } else {
                env::remove_var(key);
            }
        }
    }
    admin
        .batch_execute(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
        .await
        .unwrap();
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

pub(super) async fn run(input: &ScopedManifest, execute: bool) -> Result<()> {
    let file = tempfile::NamedTempFile::new()?;
    std::fs::write(file.path(), serde_json::to_vec(input)?)?;
    run_scoped_records_command(file.path().into(), Some(input.digest()), execute).await
}

pub(super) fn manifest() -> ScopedManifest {
    let now = time::OffsetDateTime::now_utc();
    let stamp = |t: time::OffsetDateTime| {
        t.format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    };
    ScopedManifest {
        version: SCOPED_MANIFEST_VERSION.into(),
        operation_id: "synthetic_apply_0001".into(),
        action: ScopedAction::Apply,
        brain_server: SERVER.into(),
        brain_id: "synthetic-brain".into(),
        operator: ScopedOperator {
            email: "operator@example.test".into(),
            workos_subject: "workos-operator".into(),
        },
        evidence_pack_sha256: "ab".repeat(32),
        not_before: stamp(now - time::Duration::minutes(1)),
        expires_at: stamp(now + time::Duration::hours(1)),
        target_operation_id: None,
        records: vec![
            ScopedRecordInput {
                public_key_hex: "01".repeat(32),
                kind: ScopedRecordKind::AccountChatKey,
                account_id: Some("operator".into()),
                workos_subject: Some("workos-operator".into()),
                project_id: None,
                annotation_label: None,
                evidence: ScopedEvidence {
                    sha256: "cd".repeat(32),
                    reference: "synthetic/evidence".into(),
                    observed_at: stamp(now),
                },
            },
            ScopedRecordInput {
                public_key_hex: "02".repeat(32),
                kind: ScopedRecordKind::HistoricalIdentity,
                account_id: None,
                workos_subject: None,
                project_id: None,
                annotation_label: Some("Synthetic historical identity".into()),
                evidence: ScopedEvidence {
                    sha256: "ef".repeat(32),
                    reference: "synthetic/evidence".into(),
                    observed_at: stamp(now),
                },
            },
        ],
    }
}
