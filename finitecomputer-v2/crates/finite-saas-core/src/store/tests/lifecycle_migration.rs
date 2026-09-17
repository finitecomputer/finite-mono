use super::*;

#[tokio::test]
async fn postgres_lifecycle_migration_remaps_legacy_statuses_exactly() {
    // A scratch database at the pre-H1 schema, migrated forward by 0021
    // alone, mirrors the production upgrade path byte for byte.
    let admin_url = std::env::var("FC_CORE_POSTGRES_TEST_URL")
        .expect("FC_CORE_POSTGRES_TEST_URL is required for Core Postgres tests");
    let (admin, admin_connection) = tokio_postgres::connect(&admin_url, NoTls).await.unwrap();
    let admin_connection = tokio::spawn(async move {
        let _ = admin_connection.await;
    });
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let db_name = format!("fc_test_lifecycle_{unique}");
    admin
        .execute(&format!("CREATE DATABASE \"{db_name}\""), &[])
        .await
        .unwrap();
    let (base, query) = match admin_url.split_once('?') {
        Some((base, query)) => (base.to_string(), Some(query.to_string())),
        None => (admin_url.clone(), None),
    };
    let scheme_end = base.find("://").map(|idx| idx + 3).unwrap_or(0);
    let db_url = match base[scheme_end..].find('/') {
        Some(rel) => format!("{}/{db_name}", &base[..scheme_end + rel]),
        None => format!("{base}/{db_name}"),
    };
    let db_url = match query {
        Some(query) => format!("{db_url}?{query}"),
        None => db_url,
    };
    let (raw, connection) = tokio_postgres::connect(&db_url, NoTls).await.unwrap();
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });

    let outcome = std::panic::AssertUnwindSafe(async {
            raw.batch_execute(PRE_LIFECYCLE_SCHEMA_SQL).await.unwrap();
            raw.batch_execute(
                "INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at)
                 VALUES ('legacy-user', 'legacy@finite.vip', 'linked', 'workos-legacy', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO customer_orgs (id, owner_user_id, name, billing_class, created_at, updated_at)
                 VALUES ('legacy-org', 'legacy-user', 'Legacy', 'grandfathered', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, created_at, updated_at)
                 VALUES ('legacy-project', 'legacy-org', 'legacy-user', 'Legacy', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO runtime_artifacts (id, kind, reference, version_label, state_schema_version, created_at, promoted_at)
                 VALUES ('legacy-artifact', 'oci_image', 'image@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'v1', 'state-v1', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO agent_runtimes (
                   id, project_id, source_host_id, source_machine_id, source_import_key,
                   runtime_artifact_id, state_schema_version, host_facts, created_at, updated_at
                 ) VALUES
                   ('legacy-runtime', 'legacy-project', 'legacy-host', 'legacy-machine',
                    'legacy-host/legacy-machine', 'legacy-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('legacy-runtime-2', 'legacy-project', 'legacy-host', 'legacy-machine-2',
                    'legacy-host/legacy-machine-2', 'legacy-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO runtime_control_requests (
                   id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                   requested_by_user_id, kind, status, created_at, updated_at
                 ) VALUES
                   ('legacy-requested', 'legacy-project', 'legacy-runtime', 'legacy-host', 'legacy-machine', 'legacy-user', 'restart', 'requested', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('legacy-running', 'legacy-project', 'legacy-runtime-2', 'legacy-host', 'legacy-machine-2', 'legacy-user', 'restart', 'running', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('legacy-succeeded-restart', 'legacy-project', 'legacy-runtime', 'legacy-host', 'legacy-machine', 'legacy-user', 'restart', 'succeeded', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('legacy-succeeded-stop', 'legacy-project', 'legacy-runtime', 'legacy-host', 'legacy-machine', 'legacy-user', 'stop', 'succeeded', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('legacy-succeeded-destroy', 'legacy-project', 'legacy-runtime-2', 'legacy-host', 'legacy-machine-2', 'legacy-user', 'destroy', 'succeeded', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('legacy-failed', 'legacy-project', 'legacy-runtime', 'legacy-host', 'legacy-machine', 'legacy-user', 'upgrade', 'failed', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);",
            )
            .await
            .unwrap();

            raw.batch_execute(include_str!("../../../migrations/0021_runtime_lifecycle.sql"))
                .await
                .unwrap();

            let rows = raw
                .query(
                    "SELECT id, status, failure_stage FROM runtime_control_requests ORDER BY id",
                    &[],
                )
                .await
                .unwrap();
            let mapped: BTreeMap<String, (String, String)> = rows
                .iter()
                .map(|row| {
                    (
                        row.get::<_, String>("id"),
                        (row.get::<_, String>("status"), row.get::<_, String>("failure_stage")),
                    )
                })
                .collect();
            assert_eq!(
                mapped.get("legacy-requested"),
                Some(&("requested".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("legacy-running"),
                Some(&("launching".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("legacy-succeeded-restart"),
                Some(&("succeeded".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("legacy-succeeded-stop"),
                Some(&("stopped".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("legacy-succeeded-destroy"),
                Some(&("stopped".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("legacy-failed"),
                Some(&("failed".to_string(), "unknown".to_string()))
            );

            // The new CHECK rejects the legacy vocabulary.
            let legacy_write = raw
                .execute(
                    "UPDATE runtime_control_requests SET status = 'running' WHERE id = 'legacy-requested'",
                    &[],
                )
                .await;
            assert!(legacy_write.is_err());

            // The one-active index spans every non-terminal state.
            let index_definition: String = raw
                .query_one(
                    "SELECT pg_get_indexdef(indexrelid) FROM pg_index
                     WHERE indexrelid = 'runtime_control_requests_one_active_per_runtime'::regclass",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert!(index_definition.contains("'launching'"));
            assert!(index_definition.contains("'compute_up'"));
            assert!(!index_definition.contains("'running'"));

            // Reapplying is a no-op: the migration runs at every Core startup.
            raw.batch_execute(include_str!("../../../migrations/0021_runtime_lifecycle.sql"))
                .await
                .unwrap();
            let remapped_count: i64 = raw
                .query_one(
                    "SELECT count(*) FROM runtime_control_requests WHERE status IN ('running')",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(remapped_count, 0);
        })
        .catch_unwind()
        .await;

    drop(raw);
    connection.abort();
    let _ = admin
        .execute(
            &format!("DROP DATABASE IF EXISTS \"{db_name}\" WITH (FORCE)"),
            &[],
        )
        .await;
    drop(admin);
    admin_connection.abort();
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

/// The symmetric counterpart to
/// `postgres_lifecycle_migration_remaps_legacy_statuses_exactly`: 0021
/// applies forward onto a pre-H1 database, post-H1 writers mix in the new
/// vocabulary, and the operator-initiated reverse remap lands the table on
/// a shape the PREVIOUS generation of Core accepts — legacy CHECK and
/// index predicate restored, legacy-vocabulary writes admitted once more,
/// and the N-1 lease scan finding every relaunched row.
#[tokio::test]
async fn postgres_lifecycle_reverse_remap_restores_previous_generation_shape() {
    // A scratch database at the pre-H1 schema migrated forward by 0021
    // alone mirrors the production state an H1 rollback would face.
    let admin_url = std::env::var("FC_CORE_POSTGRES_TEST_URL")
        .expect("FC_CORE_POSTGRES_TEST_URL is required for Core Postgres tests");
    let (admin, admin_connection) = tokio_postgres::connect(&admin_url, NoTls).await.unwrap();
    let admin_connection = tokio::spawn(async move {
        let _ = admin_connection.await;
    });
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let db_name = format!("fc_test_reverse_remap_{unique}");
    admin
        .execute(&format!("CREATE DATABASE \"{db_name}\""), &[])
        .await
        .unwrap();
    let (base, query) = match admin_url.split_once('?') {
        Some((base, query)) => (base.to_string(), Some(query.to_string())),
        None => (admin_url.clone(), None),
    };
    let scheme_end = base.find("://").map(|idx| idx + 3).unwrap_or(0);
    let db_url = match base[scheme_end..].find('/') {
        Some(rel) => format!("{}/{db_name}", &base[..scheme_end + rel]),
        None => format!("{base}/{db_name}"),
    };
    let db_url = match query {
        Some(query) => format!("{db_url}?{query}"),
        None => db_url,
    };
    let (raw, connection) = tokio_postgres::connect(&db_url, NoTls).await.unwrap();
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });

    let outcome = std::panic::AssertUnwindSafe(async {
            raw.batch_execute(PRE_LIFECYCLE_SCHEMA_SQL).await.unwrap();
            raw.batch_execute(
                "INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at)
                 VALUES ('remap-user', 'remap@finite.vip', 'linked', 'workos-remap', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO customer_orgs (id, owner_user_id, name, billing_class, created_at, updated_at)
                 VALUES ('remap-org', 'remap-user', 'Remap', 'grandfathered', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, created_at, updated_at)
                 VALUES ('remap-project', 'remap-org', 'remap-user', 'Remap', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO runtime_artifacts (id, kind, reference, version_label, state_schema_version, created_at, promoted_at)
                 VALUES ('remap-artifact', 'oci_image', 'image@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'v1', 'state-v1', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO agent_runtimes (
                   id, project_id, source_host_id, source_machine_id, source_import_key,
                   runtime_artifact_id, state_schema_version, host_facts, created_at, updated_at
                 ) VALUES
                   ('remap-runtime-r0', 'remap-project', 'remap-host', 'remap-runtime-r0',
                    'remap-host/remap-runtime-r0', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r1', 'remap-project', 'remap-host', 'remap-runtime-r1',
                    'remap-host/remap-runtime-r1', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r2', 'remap-project', 'remap-host', 'remap-runtime-r2',
                    'remap-host/remap-runtime-r2', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r3', 'remap-project', 'remap-host', 'remap-runtime-r3',
                    'remap-host/remap-runtime-r3', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r4', 'remap-project', 'remap-host', 'remap-runtime-r4',
                    'remap-host/remap-runtime-r4', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r5', 'remap-project', 'remap-host', 'remap-runtime-r5',
                    'remap-host/remap-runtime-r5', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO runtime_control_requests (
                   id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                   requested_by_user_id, kind, status, created_at, updated_at
                 ) VALUES
                   ('remap-legacy-requested', 'remap-project', 'remap-runtime-r0', 'remap-host', 'remap-runtime-r0', 'remap-user', 'restart', 'requested', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-legacy-running', 'remap-project', 'remap-runtime-r1', 'remap-host', 'remap-runtime-r1', 'remap-user', 'restart', 'running', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-legacy-succ-restart', 'remap-project', 'remap-runtime-r2', 'remap-host', 'remap-runtime-r2', 'remap-user', 'restart', 'succeeded', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-legacy-succ-stop', 'remap-project', 'remap-runtime-r3', 'remap-host', 'remap-runtime-r3', 'remap-user', 'stop', 'succeeded', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-legacy-succ-destroy', 'remap-project', 'remap-runtime-r4', 'remap-host', 'remap-runtime-r4', 'remap-user', 'destroy', 'succeeded', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-legacy-failed', 'remap-project', 'remap-runtime-r5', 'remap-host', 'remap-runtime-r5', 'remap-user', 'upgrade', 'failed', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);",
            )
            .await
            .unwrap();

            raw.batch_execute(include_str!("../../../migrations/0021_runtime_lifecycle.sql"))
                .await
                .unwrap();

            // Post-H1 writers continue against the new vocabulary: active rows
            // span launching/compute_up/ready, stop/destroy completions land on
            // 'stopped', and one upgrade request stays in flight so the refusal
            // guard has something to catch.
            raw.batch_execute(
                "INSERT INTO agent_runtimes (
                   id, project_id, source_host_id, source_machine_id, source_import_key,
                   runtime_artifact_id, state_schema_version, host_facts, created_at, updated_at
                 ) VALUES
                   ('remap-runtime-r6', 'remap-project', 'remap-host', 'remap-runtime-r6',
                    'remap-host/remap-runtime-r6', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r7', 'remap-project', 'remap-host', 'remap-runtime-r7',
                    'remap-host/remap-runtime-r7', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r8', 'remap-project', 'remap-host', 'remap-runtime-r8',
                    'remap-host/remap-runtime-r8', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r9', 'remap-project', 'remap-host', 'remap-runtime-r9',
                    'remap-host/remap-runtime-r9', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r10', 'remap-project', 'remap-host', 'remap-runtime-r10',
                    'remap-host/remap-runtime-r10', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r11', 'remap-project', 'remap-host', 'remap-runtime-r11',
                    'remap-host/remap-runtime-r11', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO runtime_control_requests (
                   id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                   requested_by_user_id, kind, status, created_at, updated_at
                 ) VALUES
                   ('remap-launching', 'remap-project', 'remap-runtime-r6', 'remap-host', 'remap-runtime-r6', 'remap-user', 'restart', 'launching', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-compute-up', 'remap-project', 'remap-runtime-r7', 'remap-host', 'remap-runtime-r7', 'remap-user', 'recover_known_good_chat_runtime', 'compute_up', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-ready', 'remap-project', 'remap-runtime-r8', 'remap-host', 'remap-runtime-r8', 'remap-user', 'restart', 'ready', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-stopped-stop', 'remap-project', 'remap-runtime-r9', 'remap-host', 'remap-runtime-r9', 'remap-user', 'stop', 'stopped', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-stopped-destroy', 'remap-project', 'remap-runtime-r10', 'remap-host', 'remap-runtime-r10', 'remap-user', 'destroy', 'stopped', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-upgrade-bait', 'remap-project', 'remap-runtime-r11', 'remap-host', 'remap-runtime-r11', 'remap-user', 'upgrade', 'requested', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);",
            )
            .await
            .unwrap();

            // The refusal guard aborts the whole rescue while any upgrade-kind
            // request is still active.
            let active_error = raw
                .batch_execute(crate::RUNTIME_LIFECYCLE_REVERSE_REMAP_SQL)
                .await
                .unwrap_err();
            let db_error = active_error
                .as_db_error()
                .expect("reverse remap refusal must be a PostgreSQL error");
            assert_eq!(
                db_error.code(),
                &tokio_postgres::error::SqlState::RAISE_EXCEPTION
            );
            assert_eq!(
                db_error.message(),
                "runtime lifecycle reverse remap refused: active upgrade requests still exist"
            );
            raw.batch_execute("ROLLBACK").await.unwrap();
            let untouched_status: String = raw
                .query_one(
                    "SELECT status FROM runtime_control_requests WHERE id = 'remap-launching'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(untouched_status, "launching");
            let refused_audit_count: i64 = raw
                .query_one(
                    "SELECT count(*) FROM finite_private_admin_audit_events
                     WHERE action = 'runtime.lifecycle.reverse_remap'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(refused_audit_count, 0);

            // The operator makes the blocking request terminal; now the rescue
            // applies — twice, since the rolled-back generation may start
            // against a schema this script has already reversed.
            raw.execute(
                "UPDATE runtime_control_requests
                 SET status = 'failed', completed_at = CURRENT_TIMESTAMP
                 WHERE id = 'remap-upgrade-bait'",
                &[],
            )
            .await
            .unwrap();
            raw.batch_execute(crate::RUNTIME_LIFECYCLE_REVERSE_REMAP_SQL)
                .await
                .unwrap();
            raw.batch_execute(crate::RUNTIME_LIFECYCLE_REVERSE_REMAP_SQL)
                .await
                .unwrap();

            // Every row is back inside the legacy vocabulary. The relaunched
            // rows carry 'running'; the stopped stop/destroy rows are terminal
            // successes again; nothing else moved.
            let rows = raw
                .query(
                    "SELECT id, status, failure_stage FROM runtime_control_requests ORDER BY id",
                    &[],
                )
                .await
                .unwrap();
            let mapped: BTreeMap<String, (String, String)> = rows
                .iter()
                .map(|row| {
                    (
                        row.get::<_, String>("id"),
                        (row.get::<_, String>("status"), row.get::<_, String>("failure_stage")),
                    )
                })
                .collect();
            assert_eq!(
                mapped.get("remap-legacy-requested"),
                Some(&("requested".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-legacy-running"),
                Some(&("running".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-legacy-succ-restart"),
                Some(&("succeeded".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-legacy-succ-stop"),
                Some(&("succeeded".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-legacy-succ-destroy"),
                Some(&("succeeded".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-legacy-failed"),
                Some(&("failed".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-launching"),
                Some(&("running".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-compute-up"),
                Some(&("running".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-ready"),
                Some(&("running".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-stopped-stop"),
                Some(&("succeeded".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-stopped-destroy"),
                Some(&("succeeded".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-upgrade-bait"),
                Some(&("failed".to_string(), "unknown".to_string()))
            );

            // The legacy CHECK admits the legacy vocabulary again: the same
            // write 0021's test proved rejected must now succeed.
            raw.execute(
                "UPDATE runtime_control_requests SET status = 'running' WHERE id = 'remap-legacy-requested'",
                &[],
            )
            .await
            .unwrap();

            // The previous generation of Core inserts with an explicit column
            // list that predates failure_stage; the NOT NULL DEFAULT 'unknown'
            // column left in place must fill silently behind it.
            raw.execute(
                "INSERT INTO runtime_control_requests (
                   id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                   requested_by_user_id, kind, target_runtime_artifact_id, status,
                   runner_id, lease_token, lease_expires_at,
                   failure_message, created_at, updated_at, completed_at
                 )
                 VALUES ('remap-n1-insert', 'remap-project', 'remap-runtime-r2', 'remap-host',
                         'remap-runtime-r2', 'remap-user', 'restart', NULL, 'requested',
                         NULL, NULL, NULL, NULL, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, NULL)",
                &[],
            )
            .await
            .unwrap();

            // The N-1 lease scan keys active work off requested/running only;
            // every relaunched row must be visible to it. 4 relaunched rows +
            // remap-legacy-running round-tripped to running + the probe above
            // flipped remap-legacy-requested onto 'running' + the inserted
            // 'remap-n1-insert' at 'requested'.
            let n_minus_one_active_count: i64 = raw
                .query_one(
                    "SELECT count(*) FROM runtime_control_requests
                     WHERE status IN ('requested', 'running')",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(n_minus_one_active_count, 6);

            // Constraint and index speak the legacy shape again.
            let constraint_definition: String = raw
                .query_one(
                    "SELECT pg_get_constraintdef(constraint_row.oid)
                     FROM pg_constraint AS constraint_row
                     WHERE constraint_row.conrelid = 'runtime_control_requests'::regclass
                       AND constraint_row.conname = 'runtime_control_requests_status_check'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert!(!constraint_definition.contains("'launching'"));
            assert!(!constraint_definition.contains("'stopped'"));
            let index_definition: String = raw
                .query_one(
                    "SELECT pg_get_indexdef(indexrelid) FROM pg_index
                     WHERE indexrelid = 'runtime_control_requests_one_active_per_runtime'::regclass",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert!(index_definition.contains("'running'"));
            assert!(!index_definition.contains("'launching'"));
            assert!(!index_definition.contains("'compute_up'"));

            // One audit row per rewritten request, preserving prior values.
            let audit_count: i64 = raw
                .query_one(
                    "SELECT count(*) FROM finite_private_admin_audit_events
                     WHERE action = 'runtime.lifecycle.reverse_remap'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(audit_count, 8);
            let ready_original: String = raw
                .query_one(
                    "SELECT metadata->>'originalStatus'
                     FROM finite_private_admin_audit_events
                     WHERE action = 'runtime.lifecycle.reverse_remap'
                       AND target_id = 'remap-ready'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(ready_original, "ready");
            let stopped_stop_kind: Option<String> = raw
                .query_one(
                    "SELECT metadata->>'originalKind'
                     FROM finite_private_admin_audit_events
                     WHERE action = 'runtime.lifecycle.reverse_remap'
                       AND target_id = 'remap-stopped-stop'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(stopped_stop_kind.as_deref(), Some("stop"));
        })
        .catch_unwind()
        .await;

    drop(raw);
    connection.abort();
    let _ = admin
        .execute(
            &format!("DROP DATABASE IF EXISTS \"{db_name}\" WITH (FORCE)"),
            &[],
        )
        .await;
    drop(admin);
    admin_connection.abort();
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
