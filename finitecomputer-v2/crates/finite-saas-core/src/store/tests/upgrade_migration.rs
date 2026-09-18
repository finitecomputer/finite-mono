use super::*;

#[tokio::test]
async fn postgres_runtime_upgrade_migration_reapplies_and_rescue_refuses_active_work() {
    with_isolated_postgres(|store| async move {
            let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });

            raw.batch_execute(
                "ALTER TABLE runtime_control_requests
                   DROP CONSTRAINT runtime_control_requests_kind_check;
                 ALTER TABLE runtime_control_requests
                   ADD CONSTRAINT runtime_control_requests_kind_check
                   CHECK (kind IN ('restart', 'recover_known_good_chat_runtime', 'stop', 'destroy'));",
            )
            .await
            .unwrap();
            raw.batch_execute(include_str!("../../../migrations/0002_runtime_upgrade.sql"))
                .await
                .unwrap();
            let oid_before: u32 = raw
                .query_one(
                    "SELECT oid FROM pg_constraint
                     WHERE conrelid = 'runtime_control_requests'::regclass
                       AND conname = 'runtime_control_requests_kind_check'",
                    &[],
                )
                .await
                .unwrap()
                .get("oid");
            raw.batch_execute(include_str!("../../../migrations/0002_runtime_upgrade.sql"))
                .await
                .unwrap();
            let constraint = raw
                .query_one(
                    "SELECT oid, pg_get_constraintdef(oid) AS definition
                     FROM pg_constraint
                     WHERE conrelid = 'runtime_control_requests'::regclass
                       AND conname = 'runtime_control_requests_kind_check'",
                    &[],
                )
                .await
                .unwrap();
            assert_eq!(constraint.get::<_, u32>("oid"), oid_before);
            assert!(constraint.get::<_, String>("definition").contains("upgrade"));

            raw.batch_execute(
                r#"
                INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at)
                VALUES ('rescue-user', 'rescue@finite.vip', 'linked', 'workos-rescue', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                INSERT INTO customer_orgs (id, owner_user_id, name, billing_class, created_at, updated_at)
                VALUES ('rescue-org', 'rescue-user', 'Rescue', 'grandfathered', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, created_at, updated_at)
                VALUES ('rescue-project', 'rescue-org', 'rescue-user', 'Rescue', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                INSERT INTO runtime_artifacts (id, kind, reference, version_label, state_schema_version, created_at, promoted_at)
                VALUES ('rescue-artifact', 'oci_image', 'image@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'v1', 'state-v1', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                INSERT INTO agent_runtimes (
                  id, project_id, source_host_id, source_machine_id, source_import_key,
                  runtime_artifact_id, state_schema_version, host_facts, created_at, updated_at
                ) VALUES (
                  'rescue-runtime', 'rescue-project', 'rescue-host', 'rescue-machine',
                  'rescue-host/rescue-machine', 'rescue-artifact', 'state-v1', '{}'::jsonb,
                  CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
                );
                INSERT INTO runtime_control_requests (
                  id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                  requested_by_user_id, kind, target_runtime_artifact_id, status,
                  created_at, updated_at
                ) VALUES (
                  'rescue-request', 'rescue-project', 'rescue-runtime', 'rescue-host',
                  'rescue-machine', 'rescue-user', 'upgrade', 'rescue-artifact', 'requested',
                  CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
                );
                "#,
            )
            .await
            .unwrap();

            let active_error = raw
                .batch_execute(crate::RUNTIME_UPGRADE_ROLLBACK_RESCUE_SQL)
                .await
                .unwrap_err();
            let db_error = active_error
                .as_db_error()
                .expect("rollback rescue refusal must be a PostgreSQL error");
            assert_eq!(
                db_error.code(),
                &tokio_postgres::error::SqlState::RAISE_EXCEPTION
            );
            assert_eq!(
                db_error.message(),
                "runtime upgrade rollback rescue refused: active upgrade requests still exist"
            );
            raw.batch_execute("ROLLBACK").await.unwrap();
            assert_eq!(
                raw.query_one(
                    "SELECT kind FROM runtime_control_requests WHERE id = 'rescue-request'",
                    &[],
                )
                .await
                .unwrap()
                .get::<_, String>("kind"),
                "upgrade"
            );

            raw.execute(
                "UPDATE runtime_control_requests
                 SET status = 'succeeded', completed_at = CURRENT_TIMESTAMP
                 WHERE id = 'rescue-request'",
                &[],
            )
            .await
            .unwrap();
            raw.batch_execute(crate::RUNTIME_UPGRADE_ROLLBACK_RESCUE_SQL)
                .await
                .unwrap();
            assert_eq!(
                raw.query_one(
                    "SELECT kind FROM runtime_control_requests WHERE id = 'rescue-request'",
                    &[],
                )
                .await
                .unwrap()
                .get::<_, String>("kind"),
                "restart"
            );
            let audit_count: i64 = raw
                .query_one(
                    "SELECT count(*) FROM finite_private_admin_audit_events
                     WHERE action = 'runtime.upgrade.rollback_rescue'
                       AND target_id = 'rescue-request'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(audit_count, 1);
            drop(raw);
            connection.abort();
        })
        .await;
}
