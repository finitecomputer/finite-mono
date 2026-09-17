use super::*;

fn production_store_sources(directory: &std::path::Path, sources: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().unwrap() == "tests"
            || path.file_name().unwrap() == "tests.rs"
            || path.file_name().unwrap() == "test_rows.rs"
        {
            continue;
        }
        if path.is_dir() {
            production_store_sources(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}

/// Keep PERSISTENCE.md's row-scoped persistence rule enforced across every
/// production store module, including modules added after this decomposition.
#[test]
fn postgres_store_never_uses_full_state_persistence() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources = vec![src.join("store.rs")];
    production_store_sources(&src.join("store"), &mut sources);
    let forbidden = [
        "async fn lock_state<C>",
        "async fn load_state<C>",
        "async fn persist_state<C>",
        "async fn delete_missing_rows<C>",
        "pg_advisory_xact_lock",
        "lock_state(",
        "load_state(",
        "persist_state(",
    ];
    for path in sources {
        let source = std::fs::read_to_string(&path).unwrap();
        for pattern in forbidden {
            assert!(
                !source.contains(pattern),
                "{} must stay on row-scoped SQL helpers: found `{pattern}`",
                path.display(),
            );
        }
    }
}

#[tokio::test]
async fn postgres_pool_does_not_head_of_line_block_independent_reads() {
    with_isolated_postgres(|database| async move {
        let store = database.store.clone();
        assert_eq!(store.pool.status().max_size, DEFAULT_POSTGRES_POOL_SIZE);

        let slow_store = store.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let slow_query = tokio::spawn(async move {
            let client = slow_store.connection().await.unwrap();
            started_tx.send(()).unwrap();
            client.query_one("SELECT pg_sleep(1)", &[]).await.unwrap();
        });
        started_rx.await.unwrap();
        // Give Postgres time to enter pg_sleep. The assertion's 500 ms
        // budget is still comfortably below the one-second slow query.
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;

        tokio::time::timeout(
            std::time::Duration::from_millis(500),
            store.list_launch_code_batches(),
        )
        .await
        .expect("an unrelated read must use another pooled connection")
        .unwrap();
        slow_query.await.unwrap();
    })
    .await;
}

/// A forced constraint violation must surface as a typed, structured
/// `CoreError::Database` carrying the SQLSTATE code / constraint / table /
/// DETAIL for the logs, while the user-facing `Display` stays the generic
/// "database error" — NOT the old bare "db error" that leaked to browsers.
#[tokio::test]
async fn postgres_constraint_violation_surfaces_structured_detail() {
    with_isolated_postgres(|store| async move {
            let launch_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
        let run = "constraint-detail";
        let email = format!("constraint-detail-{run}@finite.vip");

        // Materialize one org + one entitlement row via the launch-code path,
        // which needs no Stripe setup.
        let created = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: email.clone(),
                workos_user_id: format!("workos_constraint_detail_{run}"),
                display_name: "Constraint Detail Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: format!("constraint-detail-{run}"),
                now: None,
            })
            .await
            .unwrap();
        let org_id = created.request.customer_org_id;

        // Raw client so we can force a duplicate entitlement for the same org,
        // violating the UNIQUE(customer_org_id) constraint this Phase 0 adds.
        let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let duplicate_id = format!("dup-entitlement-{run}");
        let db_error = raw
            .execute(
                "INSERT INTO agent_creation_entitlements
                   (id, customer_org_id, allowed_new_agent_runtimes, launch_code, created_at, updated_at)
                 VALUES ($1, $2, 1, NULL, now(), now())",
                &[&duplicate_id, &org_id],
            )
            .await
            .expect_err("duplicate customer_org_id must violate the UNIQUE constraint");

        let core_error = store_error(db_error);
        // User-facing surface is generic and safe to show verbatim.
        assert_eq!(core_error.to_string(), "database error");
        match &core_error {
            CoreError::Database(detail) => {
                assert_eq!(detail.code.as_deref(), Some("23505"), "unique_violation");
                assert_eq!(
                    detail.constraint.as_deref(),
                    Some("agent_creation_entitlements_customer_org_id_key"),
                    "the constraint this hotfix adds must be named in the detail"
                );
                assert_eq!(detail.table.as_deref(), Some("agent_creation_entitlements"));
                assert!(
                    detail.detail.is_some(),
                    "Postgres DETAIL line must be preserved for the logs"
                );
                // The whole point: the real message survives, not "db error".
                assert_ne!(detail.message, "db error");
                assert!(!detail.message.is_empty());
            }
            other => panic!("expected CoreError::Database, got {other:?}"),
        }
        })
        .await;
}
