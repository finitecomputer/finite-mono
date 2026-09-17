use super::*;

async fn release_canary_fixture(
    db: &crate::test_support::TestDb,
) -> (
    crate::ReleaseLaunchHostInput,
    crate::RetryTargetedLaunchCodeInput,
) {
    let (retry, code) = retry_canary_fixture(db).await;
    db.retry_targeted_launch_code_exact(&retry).await.unwrap();
    let created = db
        .request_agent_creation(RequestAgentCreationInput {
            verified_email: retry.operator_email.clone(),
            workos_user_id: retry.operator_workos_user_id.clone(),
            display_name: "Healthy canary".into(),
            launch_code: code,
            idempotency_key: "healthy-canary".into(),
            now: None,
        })
        .await
        .unwrap();
    db.lease_agent_creation_request(shared_pool_lease(None))
        .await
        .unwrap()
        .unwrap();
    let completed = db
        .complete_agent_creation_request(CompleteAgentCreationRequestInput {
            request_id: created.request.id,
            runner_id: "pool-runner".into(),
            lease_token: "pool-lease".into(),
            source_host_id: "retry-target".into(),
            source_machine_id: "healthy-machine".into(),
            runtime_artifact_id: Some("artifact-postgres-fixture".into()),
            state_schema_version: Some("state-v1".into()),
            provider_runtime_handle: None,
            contact_endpoint: None,
            runtime_capabilities: Some(kata_runtime_capabilities()),
            display_name: Some("Healthy canary".into()),
            hostname: None,
            runtime_host: Some("retry-target".into()),
            runtime_status: Some(RuntimeSummaryStatus::Online),
            active_inference_profile: None,
            hermes_available: Some(true),
            published_app_urls: vec![],
            agent_npub: None,
            now: None,
        })
        .await
        .unwrap();
    let runtime = completed.request.agent_runtime_id.unwrap();
    db.record_runtime_health_report(RecordRuntimeHealthReportInput {
        source_host_id: "retry-target".into(),
        agent_runtime_id: runtime.clone(),
        ready: true,
        reason: None,
        observed_at: current_time_iso().unwrap(),
        agent_npub: None,
        report_interval_seconds: Some(60),
        now: None,
    })
    .await
    .unwrap();
    (
        crate::ReleaseLaunchHostInput {
            reservation_code_id: retry.previous_code_id.clone(),
            source_host_id: "retry-target".into(),
            expected_canary_runtime_id: runtime,
            operator_email: retry.operator_email.clone(),
            operator_workos_user_id: retry.operator_workos_user_id.clone(),
        },
        retry,
    )
}

fn shared_pool_lease(
    runner_capacity: Option<RunnerLeaseCapacity>,
) -> LeaseAgentCreationRequestInput {
    LeaseAgentCreationRequestInput {
        runner_id: "pool-runner".into(),
        source_host_id: Some("retry-target".into()),
        lease_token: "pool-lease".into(),
        lease_seconds: Some(300),
        runner_capacity,
        now: None,
    }
}

async fn release_preserved_state(db: &crate::test_support::TestDb) -> Vec<serde_json::Value> {
    let mut rows = Vec::new();
    for table in [
        "launch_code_host_targets",
        "launch_codes",
        "launch_code_batches",
        "agent_creation_requests",
        "agent_runtimes",
        "projects",
        "project_runtime_links",
        "users",
    ] {
        rows.extend(
            db.query_json(
                &format!("SELECT to_jsonb(t) FROM {table} t ORDER BY to_jsonb(t)::text"),
                &[],
            )
            .await,
        );
    }
    rows
}

#[tokio::test]
async fn postgres_host_release_preserves_state_preview_concurrency_and_old_readers() {
    with_isolated_postgres(|db| async move {
            let (input, retry) = release_canary_fixture(&db).await;
            let before = release_preserved_state(&db).await;
            let preview = CoreStore::connect_dry_run(&db.url).await.unwrap();
            preview.release_launch_host_exact(&input).await.unwrap();
            assert!(db.query_json("SELECT to_jsonb(t) FROM launch_host_reservation_releases t", &[]).await.is_empty());
            assert!(db.query_json("SELECT to_jsonb(t) FROM finite_private_admin_audit_events t WHERE action='launch_host.release_reservation'", &[]).await.is_empty());
            assert_eq!(before, release_preserved_state(&db).await);
            let (a,b) = tokio::join!(db.release_launch_host_exact(&input), db.release_launch_host_exact(&input));
            a.unwrap(); b.unwrap();
            assert_eq!(db.query_json("SELECT to_jsonb(t) FROM launch_host_reservation_releases t", &[]).await.len(), 1);
            assert_eq!(db.query_json("SELECT to_jsonb(t) FROM finite_private_admin_audit_events t WHERE action='launch_host.release_reservation'", &[]).await.len(), 1);
            let client = db.connection().await.unwrap();
            // Actual N-1 startup and reservation predicate: the old binary stays
            // conservative after release, without erasing either canary.
            for migration in [include_str!("../../../migrations/0026_launch_code_host_targets.sql"), include_str!("../../../migrations/0027_launch_code_target_retry.sql"), include_str!("../../../migrations/0028_launch_code_cohort_targets.sql")] {
                client.batch_execute(migration).await.unwrap();
            }
            let old_reader_allows: bool = client.query_one("SELECT NOT EXISTS(SELECT 1 FROM launch_code_host_targets targets WHERE targets.source_host_id=$1)", &[&input.source_host_id]).await.unwrap().get(0);
            assert!(!old_reader_allows);
            drop(client);
            db.migrate().await.unwrap();
            assert_eq!(before, release_preserved_state(&db).await);
            assert!(db.retry_targeted_launch_code_exact(&retry).await.is_err());
        }).await;
}

#[tokio::test]
async fn postgres_host_release_opens_normal_queue_but_keeps_drain_capacity_and_target_checks() {
    with_isolated_postgres(|db| async move {
            let (input, _) = release_canary_fixture(&db).await;
            let batch = db.issue_launch_code_batch(IssueLaunchCodeBatchInput {
                name: "Ordinary onboarding".into(), code_count: 2, expires_in_hours: Some(1), hosting_tier: Some(HostingTier::Standard),
                created_by_workos_user_id: input.operator_workos_user_id.clone(), now: None,
            }).await.unwrap();
            let mut requests = Vec::new();
            for (index, code) in batch.codes.iter().enumerate() {
                requests.push(db.request_agent_creation(RequestAgentCreationInput {
                    verified_email: input.operator_email.clone(), workos_user_id: input.operator_workos_user_id.clone(),
                    display_name: "Ordinary onboarding".into(), launch_code: code.code.clone(), idempotency_key: format!("ordinary-{index}"), now: None,
                }).await.unwrap().request);
            }
            // Existing explicit placement must still exclude this host.
            let client = db.connection().await.unwrap();
            client.execute("UPDATE agent_creation_requests SET target_source_host_id='different-host' WHERE id=$1", &[&requests[0].id]).await.unwrap();
            drop(client);
            assert!(db.lease_agent_creation_request(shared_pool_lease(None)).await.unwrap().is_none());
            db.release_launch_host_exact(&input).await.unwrap();
            for (draining, active) in [(true,1), (false,42)] {
                assert!(db.lease_agent_creation_request(shared_pool_lease(Some(RunnerLeaseCapacity {
                    draining, max_sandbox_count: Some(42), active_sandbox_count: Some(active),
                    runner_classes: vec![crate::RunnerClass::Kata], runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..RunnerLeaseCapacity::default()
                }))).await.unwrap().is_none());
            }
            let leased = db.lease_agent_creation_request(shared_pool_lease(Some(RunnerLeaseCapacity {
                max_sandbox_count: Some(42), active_sandbox_count: Some(1), runner_classes: vec![crate::RunnerClass::Kata],
                runtime_capabilities: Some(kata_runtime_capabilities()), ..RunnerLeaseCapacity::default()
            }))).await.unwrap().unwrap();
            assert_eq!(leased.request.id, requests[1].id);
            assert!(leased.request.target_source_host_id.is_none());
            assert!(db.lease_agent_creation_request(shared_pool_lease(None)).await.unwrap().is_none());
        }).await;
}

#[tokio::test]
async fn postgres_host_release_rejects_ambiguous_unhealthy_and_unfinished_state() {
    with_isolated_postgres(|db| async move {
            let (input, _) = release_canary_fixture(&db).await;
            let before = release_preserved_state(&db).await;
            for field in ["reservationCodeId", "sourceHostId", "expectedCanaryRuntimeId", "operatorEmail", "operatorWorkosUserId"] {
                let mut bad = serde_json::to_value(&input).unwrap(); bad[field] = serde_json::json!("mismatch");
                assert!(db.release_launch_host_exact(&serde_json::from_value(bad).unwrap()).await.is_err(), "accepted {field}");
            }
            assert_eq!(before, release_preserved_state(&db).await);
            let client = db.connection().await.unwrap();
            for change in ["health_ready=false", "health_reported_at=CURRENT_TIMESTAMP-INTERVAL '1 hour'", "health_reported_at=CURRENT_TIMESTAMP+INTERVAL '1 hour'"] {
                client.execute(&format!("UPDATE agent_runtimes SET {change} WHERE id=$1"), &[&input.expected_canary_runtime_id]).await.unwrap();
                assert!(db.release_launch_host_exact(&input).await.is_err());
                client.execute("UPDATE agent_runtimes SET health_ready=true, health_reported_at=CURRENT_TIMESTAMP WHERE id=$1", &[&input.expected_canary_runtime_id]).await.unwrap();
            }
            client.execute("UPDATE launch_code_host_targets SET retry_of_launch_code_id=NULL, cohort_of_launch_code_id=$1 WHERE launch_code_id=(SELECT requested_launch_code FROM agent_creation_requests WHERE agent_runtime_id=$2)", &[&input.reservation_code_id, &input.expected_canary_runtime_id]).await.unwrap();
            assert!(db.release_launch_host_exact(&input).await.is_err());
            client.execute("UPDATE launch_code_host_targets SET retry_of_launch_code_id=$1, cohort_of_launch_code_id=NULL WHERE launch_code_id=(SELECT requested_launch_code FROM agent_creation_requests WHERE agent_runtime_id=$2)", &[&input.reservation_code_id, &input.expected_canary_runtime_id]).await.unwrap();
            // Model a preserved 0028 child created by the retired writer.
            let unused = db.issue_launch_code_batch(IssueLaunchCodeBatchInput {
                name: "Old cohort child".into(), code_count: 1, expires_in_hours: Some(1), hosting_tier: Some(HostingTier::Standard),
                created_by_workos_user_id: input.operator_workos_user_id.clone(), now: None,
            }).await.unwrap();
            client.execute("INSERT INTO launch_code_host_targets (launch_code_id,source_host_id,created_by_workos_user_id,created_at,cohort_of_launch_code_id) VALUES ($1,$2,$3,CURRENT_TIMESTAMP,$4)", &[&unused.codes[0].id, &input.source_host_id, &input.operator_workos_user_id, &input.reservation_code_id]).await.unwrap();
            assert!(db.release_launch_host_exact(&input).await.is_err());
            db.revoke_launch_code_batch(RevokeLaunchCodeBatchInput { batch_id: unused.batch.id, revoked_by_workos_user_id: input.operator_workos_user_id.clone(), now: None }).await.unwrap();
            client.execute("UPDATE agent_creation_requests SET status='requested' WHERE agent_runtime_id=$1", &[&input.expected_canary_runtime_id]).await.unwrap();
            assert!(db.release_launch_host_exact(&input).await.is_err());
            client.execute("UPDATE agent_creation_requests SET status='running' WHERE agent_runtime_id=$1", &[&input.expected_canary_runtime_id]).await.unwrap();
            let project: String = client.query_one("SELECT project_id FROM agent_runtimes WHERE id=$1", &[&input.expected_canary_runtime_id]).await.unwrap().get(0);
            drop(client);
            let restart = db.request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: input.operator_email.clone(), workos_user_id: input.operator_workos_user_id.clone(), project_id: project, now: None,
            }).await.unwrap();
            let client = db.connection().await.unwrap();
            for status in ["requested", "launching", "compute_up", "ready"] {
                client.execute("UPDATE runtime_control_requests SET status=$1 WHERE id=$2", &[&status, &restart.id]).await.unwrap();
                assert!(db.release_launch_host_exact(&input).await.is_err(), "accepted active {status}");
            }
            assert!(db.query_json("SELECT to_jsonb(t) FROM launch_host_reservation_releases t", &[]).await.is_empty());
        }).await;
}

#[tokio::test]
async fn postgres_host_release_fences_old_cohort_writer_in_both_lock_orders() {
    for release_first in [false, true] {
        with_isolated_postgres(|db| async move {
                let (input, _) = release_canary_fixture(&db).await;
                let unused = db.issue_launch_code_batch(IssueLaunchCodeBatchInput {
                    name: "Old writer race".into(), code_count: 1, expires_in_hours: Some(1), hosting_tier: Some(HostingTier::Standard),
                    created_by_workos_user_id: input.operator_workos_user_id.clone(), now: None,
                }).await.unwrap();
                // These are the root-lock and INSERT statements from the 0028
                // writer at 98cc02be; it has no knowledge of release receipts.
                let old_lock = "SELECT launch_code_id FROM launch_code_host_targets
                    WHERE launch_code_id=$1 AND source_host_id=$2 AND created_by_workos_user_id=$3
                      AND retry_of_launch_code_id IS NULL AND cohort_of_launch_code_id IS NULL FOR UPDATE";
                let old_insert = "INSERT INTO launch_code_host_targets
                    (launch_code_id, source_host_id, created_by_workos_user_id, created_at, cohort_of_launch_code_id)
                    VALUES ($1,$2,$3,CURRENT_TIMESTAMP,$4)";
                let mut blocker = db.connection().await.unwrap();
                let tx = blocker.transaction().await.unwrap();
                if release_first {
                    // Pause release after it owns the root, at the canary lock.
                    tx.query_one("SELECT id FROM agent_runtimes WHERE id=$1 FOR UPDATE", &[&input.expected_canary_runtime_id]).await.unwrap();
                    let release_store = db.store.clone(); let release_input = input.clone();
                    let release = tokio::spawn(async move { release_store.release_launch_host_exact(&release_input).await });
                    wait_for_targeting_lock(&tx, "%SELECT runtime.id FROM agent_runtimes runtime%").await;
                    let writer_store = db.store.clone(); let writer_input = input.clone();
                    let old_writer = tokio::spawn(async move {
                        let mut connection = writer_store.connection().await.unwrap();
                        let old_tx = connection.transaction().await.unwrap();
                        old_tx.query_one(old_lock, &[&writer_input.reservation_code_id, &writer_input.source_host_id, &writer_input.operator_workos_user_id]).await.unwrap();
                        let result = old_tx.execute(old_insert, &[&unused.codes[0].id, &writer_input.source_host_id, &writer_input.operator_workos_user_id, &writer_input.reservation_code_id]).await;
                        old_tx.rollback().await.unwrap(); result
                    });
                    wait_for_targeting_lock(&tx, "%SELECT launch_code_id FROM launch_code_host_targets%").await;
                    tx.commit().await.unwrap();
                    release.await.unwrap().unwrap();
                    assert_eq!(old_writer.await.unwrap().unwrap_err().as_db_error().unwrap().code(), &tokio_postgres::error::SqlState::CHECK_VIOLATION);
                } else {
                    tx.query_one(old_lock, &[&input.reservation_code_id, &input.source_host_id, &input.operator_workos_user_id]).await.unwrap();
                    let release_store = db.store.clone(); let release_input = input.clone();
                    let release = tokio::spawn(async move { release_store.release_launch_host_exact(&release_input).await });
                    wait_for_targeting_lock(&tx, "%SELECT launch_code_id FROM launch_code_host_targets%").await;
                    tx.execute(old_insert, &[&unused.codes[0].id, &input.source_host_id, &input.operator_workos_user_id, &input.reservation_code_id]).await.unwrap();
                    tx.commit().await.unwrap();
                    assert!(matches!(release.await.unwrap(), Err(CoreError::InvalidLaunchCode)));
                    assert!(db.query_json("SELECT to_jsonb(t) FROM launch_host_reservation_releases t", &[]).await.is_empty());
                }
            }).await;
    }
}
