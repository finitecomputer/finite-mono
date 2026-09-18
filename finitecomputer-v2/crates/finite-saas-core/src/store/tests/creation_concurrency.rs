use super::*;

/// Agent creation is single-flight per project: one ordinary creation row
/// can ever exist per project. The same idempotency key reuses the row,
/// and a different key mints a NEW project (it can never append a second
/// row to the project an earlier attempt created), so the per-project
/// request count stays one until an operator relocation appends its own
/// transaction rows.
#[tokio::test]
async fn postgres_agent_creation_is_single_flight_per_project() {
    with_isolated_postgres(|store| async move {
        let run = "single-flight";
        let email = format!("{run}@finite.vip");
        let workos = format!("workos_{run}");
        let launch_code = issue_test_launch_code(&store, "2026-09-14T12:00:00Z").await;

        let created = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: email.clone(),
                workos_user_id: workos.clone(),
                display_name: "Single Flight Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: format!("{run}-submit"),
                now: None,
            })
            .await
            .unwrap();
        assert!(!created.reused);

        // A retried identical attempt (same draft, same key) reuses the row.
        let retried = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: email.clone(),
                workos_user_id: workos.clone(),
                display_name: "Single Flight Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: format!("{run}-submit"),
                now: None,
            })
            .await
            .unwrap();
        assert!(retried.reused);
        assert_eq!(retried.request.id, created.request.id);
        assert_eq!(retried.project.id, created.project.id);

        // A genuinely different attempt (new key, new launch code) mints a
        // new PROJECT; it must not add a second request row to the old one.
        let second_code = issue_test_launch_code(&store, "2026-09-14T12:05:00Z").await;
        let second = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: email.clone(),
                workos_user_id: workos.clone(),
                display_name: "Second Flight Agent".to_string(),
                launch_code: second_code,
                idempotency_key: format!("{run}-submit-2"),
                now: None,
            })
            .await
            .unwrap();
        assert_ne!(second.request.project_id, created.request.project_id);
        assert_ne!(second.request.id, created.request.id);

        let rows = store.all("agent_creation_requests").await;
        assert_eq!(rows.len(), 2);
        let project_ids: BTreeSet<&str> = rows
            .iter()
            .map(|row| row["project_id"].as_str().unwrap())
            .collect();
        assert_eq!(project_ids.len(), 2, "one creation row per project");
    })
    .await;
}

/// Concurrent identical creation attempts must not insert two rows: the
/// owner's user-row upsert serializes the transactions, so the loser's
/// idempotency lookup sees the winner's committed row and reuses it.
#[tokio::test]
async fn postgres_concurrent_identical_agent_creation_reuses_one_request() {
    with_isolated_postgres(|store| async move {
        let run = "single-flight-race";
        let email = format!("{run}@finite.vip");
        let workos = format!("workos_{run}");
        let launch_code = issue_test_launch_code(&store, "2026-09-14T12:10:00Z").await;
        let attempt = |launch_code: String| {
            let store = store.store.clone();
            let email = email.clone();
            let workos = workos.clone();
            async move {
                store
                    .request_agent_creation(RequestAgentCreationInput {
                        verified_email: email,
                        workos_user_id: workos,
                        display_name: "Race Agent".to_string(),
                        launch_code,
                        idempotency_key: format!("{run}-submit"),
                        now: None,
                    })
                    .await
            }
        };
        let (first, second) = tokio::join!(attempt(launch_code.clone()), attempt(launch_code));
        let first = first.expect("first concurrent attempt must succeed");
        let second = second.expect("second concurrent attempt must reuse, not duplicate");
        assert_eq!(first.request.id, second.request.id);
        assert_eq!(first.project.id, second.project.id);
        assert!(first.reused || second.reused);

        let rows = store.all("agent_creation_requests").await;
        assert_eq!(rows.len(), 1, "the race must not insert two requests");
    })
    .await;
}

/// The reuse path must not drop a pre-mint it now carries: a retry with
/// the same idempotency key backfills the owner hosted-chat account id
/// only while the column is still empty, and never overwrites a value
/// that a previous attempt already recorded.
#[tokio::test]
async fn postgres_agent_creation_retry_backfills_owner_chat_account_id() {
    with_isolated_postgres(|store| async move {
        let run = "premint-backfill";
        let email = format!("{run}@finite.vip");
        let workos = format!("workos_{run}");
        let launch_code = issue_test_launch_code(&store, "2026-09-14T12:20:00Z").await;
        let input = |key: String| RequestAgentCreationInput {
            verified_email: email.clone(),
            workos_user_id: workos.clone(),
            display_name: "Pre-mint Agent".to_string(),
            launch_code: launch_code.clone(),
            idempotency_key: key,
            now: None,
        };

        // First attempt: the hosted device was unavailable, so the
        // dashboard's fail-open pre-mint produced no account id.
        let created = store
            .request_agent_creation_configured(
                input(format!("{run}-submit")),
                AgentCreationConfiguration::default(),
            )
            .await
            .unwrap();
        assert_eq!(created.request.owner_chat_account_id, None);

        // Retry with the same key now carrying the pre-minted id.
        let premint = "a".repeat(64);
        let retried = store
            .request_agent_creation_configured(
                input(format!("{run}-submit")),
                AgentCreationConfiguration {
                    owner_chat_account_id: Some(premint.clone()),
                    ..AgentCreationConfiguration::default()
                },
            )
            .await
            .unwrap();
        assert!(retried.reused);
        assert_eq!(retried.request.owner_chat_account_id, Some(premint.clone()));
        let row = store
            .row("agent_creation_requests", &created.request.id)
            .await
            .unwrap();
        assert_eq!(
            row["owner_chat_account_id"].as_str(),
            Some(premint.as_str())
        );

        // A later retry with a different id must not overwrite the
        // recorded value.
        let overwritten = store
            .request_agent_creation_configured(
                input(format!("{run}-submit")),
                AgentCreationConfiguration {
                    owner_chat_account_id: Some("b".repeat(64)),
                    ..AgentCreationConfiguration::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            overwritten.request.owner_chat_account_id,
            Some(premint.clone()),
            "an existing pre-mint stays authoritative"
        );

        // The complementary ordering: when the backfill lands while the
        // row is still `requested`, the runner lease that follows reads
        // the owner id and injects it into the runtime spec environment.
        let lease = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-premint-backfill".to_string(),
                source_host_id: None,
                lease_token: "premint-backfill-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: None,
            })
            .await
            .unwrap()
            .expect("the backfilled request must still be leasable");
        assert_eq!(
            lease.request.owner_chat_account_id,
            Some(premint.clone()),
            "the lease must observe the backfilled owner id after the row lock"
        );
        let spec = runtime_spec_v1(
            lease
                .request
                .runtime_spec
                .as_ref()
                .expect("the lease must build a runtime spec"),
        );
        assert_eq!(
            spec.environment.get(crate::OWNER_CHAT_NPUBS_ENV),
            Some(&premint),
            "the lease-time spec must scope chat admission to the backfilled owner"
        );
    })
    .await;
}

/// The owner-identity backfill must be atomic against a runner lease
/// (review point "Backfill update"): reading `status = 'requested'` and
/// then updating is not enough, because a runner can lease between the
/// two statements and build its runtime spec without the owner identity —
/// a backfill that still landed afterwards would make the database claim
/// a configuration the runtime never received. This reproduces that
/// interleaving: one transaction READS `requested`, a real runner lease
/// commits `launching`, and the backfill UPDATE — carrying the full
/// `status = 'requested'` predicate — must then match zero rows and leave
/// the row untouched; the reuse path reports the persisted reality.
#[tokio::test]
async fn postgres_agent_creation_backfill_loses_to_runner_lease() {
    with_isolated_postgres(|store| async move {
        let run = "premint-lease-race";
        let email = format!("{run}@finite.vip");
        let workos = format!("workos_{run}");
        let launch_code = issue_test_launch_code(&store, "2026-09-14T13:00:00Z").await;
        let created = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: email.clone(),
                workos_user_id: workos.clone(),
                display_name: "Lease Race Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: format!("{run}-submit"),
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(
            created.request.status,
            AgentCreationRequestStatus::Requested
        );
        assert_eq!(created.request.owner_chat_account_id, None);

        // The reuse-read half of the racy interleaving: an open
        // transaction observes `requested` (plain read, no row lock, so
        // the lease below is free to claim and commit).
        let (racy, racy_connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        let racy_connection = tokio::spawn(async move {
            let _ = racy_connection.await;
        });
        racy.execute("BEGIN", &[]).await.unwrap();
        let observed: String = racy
            .query_one(
                "SELECT status FROM agent_creation_requests WHERE id = $1",
                &[&created.request.id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(observed, "requested");

        // The runner leases the request for real: status flips to
        // `launching` and the spec is built WITHOUT the owner identity
        // (the column is still empty).
        let leased = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: format!("runner-{run}"),
                source_host_id: None,
                lease_token: "lease-race-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: None,
            })
            .await
            .unwrap()
            .expect("the request must be leasable");
        assert_eq!(leased.request.id, created.request.id);
        assert_eq!(
            leased.request.owner_chat_account_id, None,
            "the lease-time spec was built without the owner identity"
        );

        // The update half of the interleaving, from the SAME transaction
        // that read `requested`: the predicate is re-evaluated against
        // the now-committed row, so the backfill must match zero rows.
        let landed = racy
            .execute(
                "UPDATE agent_creation_requests
                     SET owner_chat_account_id = $2
                     WHERE id = $1
                       AND owner_chat_account_id IS NULL
                       AND status = 'requested'",
                &[&created.request.id, &"a".repeat(64)],
            )
            .await
            .unwrap();
        assert_eq!(landed, 0, "a leased request must not be backfilled");
        racy.execute("COMMIT", &[]).await.unwrap();
        drop(racy);
        racy_connection.abort();

        let row = store
            .row("agent_creation_requests", &created.request.id)
            .await
            .unwrap();
        assert_eq!(
            row["owner_chat_account_id"].as_str(),
            None,
            "the database must not claim an owner identity the runtime never received"
        );
        assert_eq!(row["status"].as_str(), Some("launching"));

        // The reuse path must report that reality, not the requested
        // pre-mint: no error, no fake configuration.
        let retried = store
            .request_agent_creation_configured(
                RequestAgentCreationInput {
                    verified_email: email,
                    workos_user_id: workos,
                    display_name: "Lease Race Agent".to_string(),
                    launch_code,
                    idempotency_key: format!("{run}-submit"),
                    now: None,
                },
                AgentCreationConfiguration {
                    owner_chat_account_id: Some("a".repeat(64)),
                    ..AgentCreationConfiguration::default()
                },
            )
            .await
            .unwrap();
        assert!(retried.reused);
        assert_eq!(retried.request.id, created.request.id);
        assert_eq!(
            retried.request.status,
            AgentCreationRequestStatus::Launching,
            "the returned row must reflect the lease, not the backfill attempt"
        );
        assert_eq!(
            retried.request.owner_chat_account_id, None,
            "losing the race to a lease must leave the row unconfigured"
        );
    })
    .await;
}
