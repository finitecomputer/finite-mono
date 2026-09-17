use super::*;

#[tokio::test]
async fn postgres_finite_private_usage_accepts_linked_user_without_grant() {
    with_isolated_postgres(|store| async move {
        let workos_user_id = "workos_dashboard_summary_no_grant";
        store
            .link_verified_user(LinkVerifiedUserInput {
                verified_email: "dashboard-summary-no-grant@finite.vip".to_string(),
                workos_user_id: workos_user_id.to_string(),
                now: None,
            })
            .await
            .unwrap();

        assert_eq!(
            store
                .finite_private_usage_status_for_workos_user(workos_user_id, None)
                .await
                .unwrap(),
            None
        );
    })
    .await;
}

#[tokio::test]
async fn postgres_finite_private_default_survives_reapply_and_n_minus_one_schema() {
    with_isolated_postgres(|store| async move {
            // Reapplying the current concat is how this service migrates on
            // every startup. A rolled-back N-1 binary would then replay 0010,
            // which still names the old 50M policy. The compatibility trigger
            // must preserve the doubled limit in both cases.
            store.migrate().await.unwrap();
            let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });
            raw.batch_execute(include_str!(
                "../../../migrations/0010_align_finite_private_generous.sql"
            ))
            .await
            .unwrap();

            let old_limit: i64 = raw
                .query_one(
                    "SELECT burst_limit_units FROM finite_private_limit_profiles WHERE id = 'finite-private-generous'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            let new_limit: i64 = raw
                .query_one(
                    "SELECT burst_limit_units FROM finite_private_limit_profiles WHERE id = 'finite-private-generous-v2'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            let five_x_limit: i64 = raw
                .query_one(
                    "SELECT burst_limit_units FROM finite_private_limit_profiles WHERE id = $1",
                    &[&crate::FINITE_PRIVATE_5X_LIMIT_PROFILE],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(old_limit, 100_000_000);
            assert_eq!(new_limit, 100_000_000);
            assert_eq!(five_x_limit, 500_000_000);
            let usage_index_exists: bool = raw
                .query_one(
                    "SELECT to_regclass('finite_private_reservations_grant_status_epoch_created_idx') IS NOT NULL",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert!(usage_index_exists);

            let run = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let raw_key = format!("fpk_live_schema_replay_{run}");
            let issued = store
                .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                    admin_verified_email: "schema-replay-admin@finite.vip".to_string(),
                    friend_email: format!("schema-replay-{run}@finite.vip"),
                    limit_profile_id: None,
                    raw_key,
                    now: None,
                })
                .await
                .unwrap();
            let profile_id: String = raw
                .query_one(
                    "SELECT limit_profile_id FROM finite_private_grants WHERE id = $1",
                    &[&issued.grant.id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(profile_id, "finite-private-generous-v2");

            drop(raw);
            connection.abort();
        })
        .await;
}

#[tokio::test]
async fn postgres_finite_private_same_window_reservations_share_epoch_and_settle() {
    with_isolated_postgres(|store| async move {
        let issued = store
            .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                admin_verified_email: "epoch-admin@finite.vip".to_string(),
                friend_email: "epoch-user@finite.vip".to_string(),
                limit_profile_id: None,
                raw_key: "fpk_live_postgres_epoch".to_string(),
                now: Some("2026-07-21T12:00:00Z".to_string()),
            })
            .await
            .unwrap();

        let reserve = |request_id: &str, units: i64, now: &str| ReserveFinitePrivateUsageInput {
            request_id: request_id.to_string(),
            presented_api_key: "fpk_live_postgres_epoch".to_string(),
            endpoint: "/v1/chat/completions".to_string(),
            model: "glm-5.2".to_string(),
            estimated_prompt_tokens: units,
            estimated_completion_tokens: 0,
            estimated_usage_units: units,
            usage_formula_version: "v1".to_string(),
            dashboard_url: "https://finite.computer/dashboard".to_string(),
            now: Some(now.to_string()),
        };
        let first = store
            .reserve_finite_private_usage(reserve(
                "req-postgres-epoch-1",
                30_000_000,
                "2026-07-21T12:00:01Z",
            ))
            .await
            .unwrap();
        let second = store
            .reserve_finite_private_usage(reserve(
                "req-postgres-epoch-2",
                30_000_000,
                "2026-07-21T12:01:00Z",
            ))
            .await
            .unwrap();

        let settle = |reservation_id: String, request_id: &str, units: i64, now: &str| {
            SettleFinitePrivateReservationInput {
                reservation_id,
                request_id: request_id.to_string(),
                settlement: crate::FinitePrivateSettlementKind::Actual,
                prompt_tokens: Some(units),
                completion_tokens: Some(0),
                usage_units: Some(units),
                usage_formula_version: "v1".to_string(),
                upstream_status: Some(200),
                upstream_error_class: None,
                now: Some(now.to_string()),
            }
        };
        store
            .settle_finite_private_reservation(settle(
                first.reservation_id.unwrap(),
                "req-postgres-epoch-1",
                20_000_000,
                "2026-07-21T12:01:10Z",
            ))
            .await
            .unwrap();
        store
            .settle_finite_private_reservation(settle(
                second.reservation_id.unwrap(),
                "req-postgres-epoch-2",
                55_000_000,
                "2026-07-21T12:01:20Z",
            ))
            .await
            .unwrap();

        let status = store
            .finite_private_usage_status_for_api_key(
                "fpk_live_postgres_epoch",
                true,
                Some("2026-07-21T12:02:00Z".to_string()),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.burst_used_units, 75_000_000);
        assert_eq!(
            status
                .notice
                .as_ref()
                .map(|notice| notice.threshold_remaining_percent),
            Some(25)
        );

        let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        let epochs = raw
            .query(
                "SELECT DISTINCT burst_window_epoch
                     FROM finite_private_reservations
                     WHERE grant_id = $1
                     ORDER BY burst_window_epoch",
                &[&issued.grant.id],
            )
            .await
            .unwrap();
        assert_eq!(
            epochs.len(),
            1,
            "same-window reservations must share one epoch"
        );
        let settled_count: i64 = raw
            .query_one(
                "SELECT COUNT(*) FROM finite_private_reservations
                     WHERE grant_id = $1 AND status = 'settled'",
                &[&issued.grant.id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(settled_count, 2);
        drop(raw);
        connection.abort();
    })
    .await;
}
