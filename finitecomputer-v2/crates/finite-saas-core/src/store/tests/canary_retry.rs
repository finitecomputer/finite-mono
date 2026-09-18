use super::*;

#[tokio::test]
async fn postgres_canary_retry_preserves_history_dry_run_and_n_minus_one_readers() {
    with_isolated_postgres(|db| async move {
            let (input, code) = retry_canary_fixture(&db).await;
            let before = [
                db.row("launch_codes", &input.previous_code_id).await,
                db.row("agent_creation_requests", &input.expected_previous_request_id).await,
                db.row("agent_runtimes", &input.expected_previous_runtime_id).await,
                db.row("projects", &input.expected_previous_project_id).await,
            ];
            let original_binding = db.query_json("SELECT to_jsonb(t) FROM launch_code_host_targets t WHERE launch_code_id=$1", &[&input.previous_code_id]).await;
            let preview = CoreStore::connect_dry_run(&db.url).await.unwrap();
            preview.retry_targeted_launch_code_exact(&input).await.unwrap();
            assert!(db.query_json("SELECT to_jsonb(t) FROM launch_code_host_targets t WHERE launch_code_id=$1", &[&input.code_id]).await.is_empty());
            assert!(db.query_json("SELECT to_jsonb(t) FROM finite_private_admin_audit_events t WHERE action='launch_code.retry_target_host'", &[]).await.is_empty());
            db.retry_targeted_launch_code_exact(&input).await.unwrap();
            db.retry_targeted_launch_code_exact(&input).await.unwrap();
            // N-1 startup keeps the partial host index; N-1 readers need no new
            // column or wire shape. Apply that exact old migration after retry.
            let client = db.connection().await.unwrap();
            client.batch_execute(include_str!("../../../migrations/0026_launch_code_host_targets.sql")).await.unwrap();
            for id in [&input.previous_code_id, &input.code_id] {
                let host: String = client.query_one("SELECT source_host_id FROM launch_code_host_targets WHERE launch_code_id=$1", &[id]).await.unwrap().get(0);
                assert_eq!(host, "retry-target");
            }
            drop(client);
            db.migrate().await.unwrap(); // retry schema is idempotent too
            assert_eq!(db.query_json("SELECT to_jsonb(t) FROM finite_private_admin_audit_events t WHERE action='launch_code.retry_target_host'", &[]).await.len(), 1);
            let created = db.request_agent_creation(RequestAgentCreationInput {
                verified_email: input.operator_email.clone(), workos_user_id: input.operator_workos_user_id.clone(),
                display_name: "Fresh canary".into(), launch_code: code, idempotency_key: "fresh".into(), now: None,
            }).await.unwrap();
            assert_eq!(created.request.target_source_host_id.as_deref(), Some("retry-target"));
            assert!(db.retry_targeted_launch_code_exact(&input).await.is_err()); // consumed retry is never reset
            for host in [None, Some("wrong-host"), Some("retry-target")] {
                let leased = db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "retry-runner".into(), source_host_id: host.map(String::from), lease_token: "retry-lease".into(),
                    lease_seconds: Some(300), runner_capacity: None, now: None,
                }).await.unwrap();
                if host == Some("retry-target") { assert_eq!(leased.unwrap().request.id, created.request.id); }
                else { assert!(leased.is_none()); }
            }
            assert_eq!(original_binding, db.query_json("SELECT to_jsonb(t) FROM launch_code_host_targets t WHERE launch_code_id=$1", &[&input.previous_code_id]).await);
            assert_eq!(before, [
                db.row("launch_codes", &input.previous_code_id).await,
                db.row("agent_creation_requests", &input.expected_previous_request_id).await,
                db.row("agent_runtimes", &input.expected_previous_runtime_id).await,
                db.row("projects", &input.expected_previous_project_id).await,
            ]);
        }).await;
}

#[tokio::test]
async fn postgres_canary_retry_rejects_mismatched_or_occupied_state_and_competing_retries() {
    with_isolated_postgres(|db| async move {
            let (input, _) = retry_canary_fixture(&db).await;
            for field in ["codeId", "expectedBatchId", "previousCodeId", "expectedPreviousRequestId", "expectedPreviousProjectId", "expectedPreviousRuntimeId", "expectedPreviousSourceHostId", "targetSourceHostId", "operatorEmail", "operatorWorkosUserId"] {
                let mut bad = serde_json::to_value(&input).unwrap();
                bad[field] = serde_json::json!("mismatch");
                let bad = serde_json::from_value(bad).unwrap();
                assert!(db.retry_targeted_launch_code_exact(&bad).await.is_err(), "accepted mismatched {field}");
            }
            let client = db.connection().await.unwrap();
            // A separate Runtime on the target refuses even when the exact
            // original misplacement still matches all expected identifiers.
            client.execute("INSERT INTO agent_runtimes SELECT (jsonb_populate_record(NULL::agent_runtimes, to_jsonb(r) || jsonb_build_object('id','occupied-runtime','source_host_id',$1::text,'source_machine_id','occupied-machine','source_import_key','occupied-key'))).* FROM agent_runtimes r WHERE id=$2", &[&input.target_source_host_id, &input.expected_previous_runtime_id]).await.unwrap();
            assert!(db.retry_targeted_launch_code_exact(&input).await.is_err());
            client.execute("DELETE FROM agent_runtimes WHERE id='occupied-runtime'", &[]).await.unwrap();
            client.execute("UPDATE agent_creation_requests SET target_source_host_id=$1 WHERE id=$2", &[&input.target_source_host_id, &input.expected_previous_request_id]).await.unwrap();
            assert!(db.retry_targeted_launch_code_exact(&input).await.is_err());
            client.execute("UPDATE agent_creation_requests SET target_source_host_id=NULL WHERE id=$1", &[&input.expected_previous_request_id]).await.unwrap();
            client.execute("UPDATE launch_code_batches SET revoked_at=CURRENT_TIMESTAMP, revoked_by_workos_user_id='retry-operator' WHERE id=$1", &[&input.expected_batch_id]).await.unwrap();
            assert!(db.retry_targeted_launch_code_exact(&input).await.is_err());
            client.execute("UPDATE launch_code_batches SET revoked_at=NULL, revoked_by_workos_user_id=NULL WHERE id=$1", &[&input.expected_batch_id]).await.unwrap();
            drop(client);
            let restart = db.request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: input.operator_email.clone(), workos_user_id: input.operator_workos_user_id.clone(),
                project_id: input.expected_previous_project_id.clone(), now: None,
            }).await.unwrap();
            let client = db.connection().await.unwrap();
            for status in ["requested", "launching", "compute_up", "ready"] {
                client.execute("UPDATE runtime_control_requests SET status=$1 WHERE id=$2", &[&status, &restart.id]).await.unwrap();
                assert!(db.retry_targeted_launch_code_exact(&input).await.is_err(), "accepted active control status {status}");
            }
            client.execute("UPDATE runtime_control_requests SET status='failed' WHERE id=$1", &[&restart.id]).await.unwrap();
            drop(client);
            let another = db.issue_launch_code_batch(IssueLaunchCodeBatchInput {
                name: "competing retry".into(), code_count: 1, expires_in_hours: Some(1), hosting_tier: Some(HostingTier::Standard),
                created_by_workos_user_id: input.operator_workos_user_id.clone(), now: None,
            }).await.unwrap();
            let mut competing = input.clone(); competing.code_id = another.codes[0].id.clone(); competing.expected_batch_id = another.batch.id;
            let (a,b) = tokio::join!(db.retry_targeted_launch_code_exact(&input), db.retry_targeted_launch_code_exact(&competing));
            assert!(a.is_ok() ^ b.is_ok());
            assert_eq!(db.query_json("SELECT to_jsonb(t) FROM launch_code_host_targets t WHERE retry_of_launch_code_id=$1", &[&input.previous_code_id]).await.len(), 1);
            assert_eq!(db.query_json("SELECT to_jsonb(t) FROM finite_private_admin_audit_events t WHERE action='launch_code.retry_target_host'", &[]).await.len(), 1);
            let rejected = if a.is_ok() { &competing } else { &input };
            // The normal command still cannot create a second root reservation.
            assert!(db.target_launch_code_exact(&rejected.code_id, &rejected.expected_batch_id, "retry-target", &input.operator_email, &input.operator_workos_user_id).await.is_err());
        }).await;
}
