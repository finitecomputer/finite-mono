use super::*;

#[tokio::test]
async fn postgres_launch_codes_are_one_time_metadata_only_and_idempotent() {
    with_isolated_postgres(|store| async move {
        let issued = store
            .issue_launch_code_batch(IssueLaunchCodeBatchInput {
                name: "Internal canary".to_string(),
                code_count: 3,
                expires_in_hours: Some(1),
                hosting_tier: None,
                created_by_workos_user_id: "workos_operator".to_string(),
                now: Some("2026-07-10T12:00:00Z".to_string()),
            })
            .await
            .unwrap();
        let batch_id = issued.batch.id.clone();
        let plaintext = issued.codes[0].code.clone();
        let unused = issued.codes[1].code.clone();
        let expiring = issued.codes[2].code.clone();

        let later = store.list_launch_code_batches().await.unwrap();
        let later_json = serde_json::to_string(&later).unwrap();
        assert!(!later_json.contains(&plaintext));
        assert!(!later_json.contains(&unused));
        assert!(serde_json::to_string(&issued).unwrap().contains(&plaintext));

        let created = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "canary@finite.vip".to_string(),
                workos_user_id: "workos_canary".to_string(),
                display_name: "Canary Agent".to_string(),
                launch_code: plaintext.clone(),
                idempotency_key: "canary-request".to_string(),
                now: Some("2026-07-10T12:30:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_ne!(
            created.request.requested_launch_code.as_deref(),
            Some(plaintext.as_str())
        );

        store
            .revoke_launch_code_batch(RevokeLaunchCodeBatchInput {
                batch_id: batch_id.clone(),
                revoked_by_workos_user_id: "workos_operator".to_string(),
                now: Some("2026-07-10T12:45:00Z".to_string()),
            })
            .await
            .unwrap();

        // Exact retries remain idempotent after both revocation and expiry.
        let replay = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "canary@finite.vip".to_string(),
                workos_user_id: "workos_canary".to_string(),
                display_name: "Ignored retry name".to_string(),
                launch_code: plaintext.clone(),
                idempotency_key: "canary-request".to_string(),
                now: Some("2026-07-10T14:00:00Z".to_string()),
            })
            .await
            .unwrap();
        assert!(replay.reused);
        assert_eq!(replay.request.id, created.request.id);

        for (email, workos_id, key, code) in [
            (
                "canary@finite.vip",
                "workos_canary",
                "different-request",
                plaintext.as_str(),
            ),
            (
                "other@finite.vip",
                "workos_other",
                "other-request",
                plaintext.as_str(),
            ),
            (
                "unused@finite.vip",
                "workos_unused",
                "unused-request",
                unused.as_str(),
            ),
            (
                "expired@finite.vip",
                "workos_expired",
                "expired-request",
                expiring.as_str(),
            ),
        ] {
            let error = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: email.to_string(),
                    workos_user_id: workos_id.to_string(),
                    display_name: "Rejected Agent".to_string(),
                    launch_code: code.to_string(),
                    idempotency_key: key.to_string(),
                    now: Some("2026-07-10T14:00:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(error, CoreError::InvalidLaunchCode));
        }

        let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        let row = raw
            .query_one(
                "SELECT code_hash,
                            (SELECT launch_code FROM agent_creation_entitlements
                              WHERE customer_org_id = $1) AS entitlement_code,
                            (SELECT requested_launch_code FROM agent_creation_requests
                              WHERE id = $2) AS request_code
                       FROM launch_codes WHERE id = $3",
                &[
                    &created.request.customer_org_id,
                    &created.request.id,
                    &issued.codes[0].id,
                ],
            )
            .await
            .unwrap();
        let code_hash: String = row.get("code_hash");
        let entitlement_code: Option<String> = row.get("entitlement_code");
        let request_code: Option<String> = row.get("request_code");
        assert_ne!(code_hash, plaintext);
        assert_eq!(
            entitlement_code.as_deref(),
            Some(issued.codes[0].id.as_str())
        );
        assert_eq!(request_code.as_deref(), Some(issued.codes[0].id.as_str()));
        drop(raw);
        connection.abort();
    })
    .await;
}

#[tokio::test]
async fn postgres_rejects_selected_tier_mismatch_without_consuming_launch_code() {
    with_isolated_postgres(|store| async move {
        let issued = store
            .issue_launch_code_batch(IssueLaunchCodeBatchInput {
                name: "Tier mismatch".to_string(),
                code_count: 1,
                expires_in_hours: Some(1),
                hosting_tier: Some(HostingTier::Standard),
                created_by_workos_user_id: "workos_operator".to_string(),
                now: Some("2026-07-23T12:00:00Z".to_string()),
            })
            .await
            .unwrap();
        let input = RequestAgentCreationInput {
            verified_email: "tier-check@finite.vip".to_string(),
            workos_user_id: "workos_tier_check".to_string(),
            display_name: "Tier Check".to_string(),
            launch_code: issued.codes[0].code.clone(),
            idempotency_key: "tier-check-submit".to_string(),
            now: Some("2026-07-23T12:01:00Z".to_string()),
        };

        let denied = store
            .request_agent_creation_configured(
                input.clone(),
                AgentCreationConfiguration {
                    requested_hosting_tier: Some(HostingTier::Confidential),
                    ..AgentCreationConfiguration::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(denied, CoreError::HostingTierNotAuthorized));

        let created = store
            .request_agent_creation_configured(
                input,
                AgentCreationConfiguration {
                    requested_hosting_tier: Some(HostingTier::Standard),
                    ..AgentCreationConfiguration::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(created.request.runner_class, RunnerClass::Kata);
    })
    .await;
}

#[tokio::test]
async fn postgres_launch_code_redemption_serializes_with_revocation() {
    with_isolated_postgres(|store| async move {
        let issued = store
            .issue_launch_code_batch(IssueLaunchCodeBatchInput {
                name: "Revocation race".to_string(),
                code_count: 1,
                expires_in_hours: Some(24),
                hosting_tier: None,
                created_by_workos_user_id: "workos_operator".to_string(),
                now: Some("2026-07-10T12:00:00Z".to_string()),
            })
            .await
            .unwrap();
        let batch_id = issued.batch.id.clone();
        let plaintext = issued.codes[0].code.clone();

        // Hold an uncommitted batch revocation. Redemption must block on
        // the batch row, then observe the committed revocation and fail.
        let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        let mut raw = raw;
        let tx = raw.transaction().await.unwrap();
        tx.execute(
            "UPDATE launch_code_batches
                    SET revoked_at = '2026-07-10T12:05:00Z'::timestamptz,
                        revoked_by_workos_user_id = 'workos_operator'
                  WHERE id = $1",
            &[&batch_id],
        )
        .await
        .unwrap();

        let competing = CoreStore::connect(&store.url).await.unwrap();
        let redeem = tokio::spawn(async move {
            competing
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "race@finite.vip".to_string(),
                    workos_user_id: "workos_race".to_string(),
                    display_name: "Race Agent".to_string(),
                    launch_code: plaintext,
                    idempotency_key: "race-request".to_string(),
                    now: Some("2026-07-10T12:10:00Z".to_string()),
                })
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!redeem.is_finished(), "redemption must wait for batch lock");
        tx.commit().await.unwrap();
        let error = redeem.await.unwrap().unwrap_err();
        assert!(matches!(error, CoreError::InvalidLaunchCode));

        let redeemed: i64 = raw
            .query_one(
                "SELECT COUNT(*) FROM launch_codes
                      WHERE batch_id = $1 AND redeemed_at IS NOT NULL",
                &[&batch_id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(redeemed, 0);
        drop(raw);
        connection.abort();
    })
    .await;
}

#[tokio::test]
async fn postgres_launch_code_concurrent_redemption_has_one_winner() {
    with_isolated_postgres(|store| async move {
        let issued = store
            .issue_launch_code_batch(IssueLaunchCodeBatchInput {
                name: "Concurrent redemption".to_string(),
                code_count: 1,
                expires_in_hours: Some(24),
                hosting_tier: None,
                created_by_workos_user_id: "workos_operator".to_string(),
                now: Some("2026-07-10T12:00:00Z".to_string()),
            })
            .await
            .unwrap();
        let plaintext = issued.codes[0].code.clone();
        let first = CoreStore::connect(&store.url).await.unwrap();
        let second = CoreStore::connect(&store.url).await.unwrap();
        let (first_result, second_result) = tokio::join!(
            first.request_agent_creation(RequestAgentCreationInput {
                verified_email: "first@finite.vip".to_string(),
                workos_user_id: "workos_first".to_string(),
                display_name: "First Agent".to_string(),
                launch_code: plaintext.clone(),
                idempotency_key: "first-request".to_string(),
                now: Some("2026-07-10T12:30:00Z".to_string()),
            }),
            second.request_agent_creation(RequestAgentCreationInput {
                verified_email: "second@finite.vip".to_string(),
                workos_user_id: "workos_second".to_string(),
                display_name: "Second Agent".to_string(),
                launch_code: plaintext,
                idempotency_key: "second-request".to_string(),
                now: Some("2026-07-10T12:30:00Z".to_string()),
            }),
        );
        let successes = [first_result.as_ref(), second_result.as_ref()]
            .into_iter()
            .filter(|result| result.is_ok())
            .count();
        assert_eq!(successes, 1);
        let failures = [first_result, second_result]
            .into_iter()
            .filter_map(Result::err)
            .collect::<Vec<_>>();
        assert_eq!(failures.len(), 1);
        assert!(matches!(failures[0], CoreError::InvalidLaunchCode));

        let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        let redeemed: i64 = raw
            .query_one(
                "SELECT COUNT(*) FROM launch_codes WHERE redeemed_at IS NOT NULL",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        let requests: i64 = raw
            .query_one("SELECT COUNT(*) FROM agent_creation_requests", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(redeemed, 1);
        assert_eq!(requests, 1);
        drop(raw);
        connection.abort();
    })
    .await;
}

#[tokio::test]
async fn postgres_fresh_launch_code_tops_up_exhausted_org_once() {
    with_isolated_postgres(|store| async move {
        let first_code = issue_test_launch_code(&store, "2026-07-10T12:00:00Z").await;
        let input = |launch_code: String, idempotency_key: &str, display_name: &str| {
            RequestAgentCreationInput {
                verified_email: "top-up@finite.vip".to_string(),
                workos_user_id: "workos_top_up".to_string(),
                display_name: display_name.to_string(),
                launch_code,
                idempotency_key: idempotency_key.to_string(),
                now: Some("2026-07-10T12:30:00Z".to_string()),
            }
        };
        store
            .request_agent_creation(input(first_code, "first-request", "First Agent"))
            .await
            .unwrap();

        let second_code = issue_test_launch_code(&store, "2026-07-10T13:00:00Z").await;
        let second = store
            .request_agent_creation(input(second_code.clone(), "second-request", "Second Agent"))
            .await
            .expect("a fresh code adds one creation to an exhausted org");
        assert!(!second.reused);

        let retry = store
            .request_agent_creation(input(second_code, "second-request", "Second Agent"))
            .await
            .unwrap();
        assert!(retry.reused);

        let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        let entitlement: i32 = raw
            .query_one(
                "SELECT allowed_new_agent_runtimes FROM agent_creation_entitlements",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        let requests: i64 = raw
            .query_one("SELECT COUNT(*) FROM agent_creation_requests", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(entitlement, 2, "the retry must not increment twice");
        assert_eq!(requests, 2);
        drop(raw);
        connection.abort();
    })
    .await;
}
