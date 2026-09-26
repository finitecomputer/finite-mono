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
async fn postgres_finite_private_200m_preserves_usage_and_survives_old_schema_replay() {
    with_isolated_postgres(|store| async move {
        let migration = include_str!("../../../migrations/0033_finite_private_200m_default.sql");
        let previous_schema = CORE_SCHEMA_SQL.split_once(migration).unwrap().0;
        let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });

        // Start from the actual N-1 schema and its 100M profile, not an
        // all-candidate fixture. No keys or usage have been seeded yet.
        raw.batch_execute(
            "DROP TRIGGER preserve_finite_private_200m_default ON finite_private_limit_profiles;
             DROP FUNCTION preserve_finite_private_200m_default();",
        )
        .await
        .unwrap();
        raw.batch_execute(previous_schema).await.unwrap();
        let old_limit: i64 = raw
            .query_one(
                "SELECT burst_limit_units FROM finite_private_limit_profiles
             WHERE id = 'finite-private-generous-v2'",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(old_limit, 100_000_000);

        for (email, key, profile) in [
            ("standard@fixture.test", "fpk_live_200m_standard", None),
            (
                "extended@fixture.test",
                "fpk_live_200m_extended",
                Some(crate::FINITE_PRIVATE_5X_LIMIT_PROFILE.to_string()),
            ),
        ] {
            store
                .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                    admin_verified_email: "operator@fixture.test".to_string(),
                    friend_email: email.to_string(),
                    limit_profile_id: profile,
                    raw_key: key.to_string(),
                    now: Some("2026-09-22T12:00:00Z".to_string()),
                })
                .await
                .unwrap();
        }
        let reserve = |request_id: &str, units: i64| ReserveFinitePrivateUsageInput {
            request_id: request_id.to_string(),
            presented_api_key: "fpk_live_200m_standard".to_string(),
            endpoint: "/v1/chat/completions".to_string(),
            model: "glm-5-3-flash".to_string(),
            estimated_prompt_tokens: units,
            estimated_completion_tokens: 0,
            estimated_usage_units: units,
            usage_formula_version: "2026-05-26.v1".to_string(),
            dashboard_url: "https://finite.computer/dashboard".to_string(),
            now: Some("2026-09-22T12:01:00Z".to_string()),
        };
        let existing = store
            .reserve_finite_private_usage(reserve("before-increase", 90_000_000))
            .await
            .unwrap();
        assert_eq!(existing.decision, "allow");
        assert_eq!(
            store
                .reserve_finite_private_usage(reserve("too-large-before", 20_000_000))
                .await
                .unwrap()
                .decision,
            "deny"
        );
        let tables = [
            "finite_private_grants",
            "finite_private_api_keys",
            "finite_private_reservations",
            "finite_private_admin_audit_events",
        ];
        let mut retained = Vec::new();
        for table in tables {
            retained.push(store.all(table).await);
        }

        // Candidate startup, candidate restart, then the complete N-1
        // startup (including 0013 replacing its own trigger), then upgrade.
        for schema in [
            CORE_SCHEMA_SQL,
            CORE_SCHEMA_SQL,
            previous_schema,
            CORE_SCHEMA_SQL,
        ] {
            raw.batch_execute(schema).await.unwrap();
            for (table, before) in tables.iter().zip(&retained) {
                assert_eq!(
                    &store.all(table).await,
                    before,
                    "{table} changed during schema replay"
                );
            }
            for (profile, expected) in [
                ("finite-private-generous", 200_000_000_i64),
                ("finite-private-generous-v2", 200_000_000),
                (crate::FINITE_PRIVATE_5X_LIMIT_PROFILE, 500_000_000),
            ] {
                let row = raw
                    .query_one(
                        "SELECT burst_limit_units, burst_window_seconds, weekly_limit_units
                     FROM finite_private_limit_profiles WHERE id = $1",
                        &[&profile],
                    )
                    .await
                    .unwrap();
                assert_eq!(row.get::<_, i64>(0), expected);
                assert_eq!(row.get::<_, i64>(1), 18_000);
                assert_eq!(row.get::<_, Option<i64>>(2), None);
            }
            let status = store
                .finite_private_usage_status_for_api_key(
                    "fpk_live_200m_standard",
                    false,
                    Some("2026-09-22T12:02:00Z".to_string()),
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(status.burst_used_units, 90_000_000);
            assert_eq!(status.burst_remaining_units, 110_000_000);
            assert_eq!(status.burst_reset_at, "2026-09-22T17:01:00Z");
        }
        // The existing issued key can now spend beyond the former 100M cap,
        // while admission still enforces exactly 200M.
        assert_eq!(
            store
                .reserve_finite_private_usage(reserve("after-increase", 110_000_000))
                .await
                .unwrap()
                .decision,
            "allow"
        );
        assert_eq!(
            store
                .reserve_finite_private_usage(reserve("over-200m", 1))
                .await
                .unwrap()
                .decision,
            "deny"
        );
        // A reservation from before the increase still settles in its original epoch.
        store
            .settle_finite_private_reservation(SettleFinitePrivateReservationInput {
                reservation_id: existing.reservation_id.unwrap(),
                request_id: "before-increase".to_string(),
                settlement: crate::FinitePrivateSettlementKind::Actual,
                prompt_tokens: Some(80_000_000),
                completion_tokens: Some(0),
                usage_units: Some(80_000_000),
                usage_formula_version: "2026-05-26.v1".to_string(),
                upstream_status: Some(200),
                upstream_error_class: None,
                now: Some("2026-09-22T12:03:00Z".to_string()),
            })
            .await
            .unwrap();
        let status = store
            .finite_private_usage_status_for_api_key(
                "fpk_live_200m_standard",
                false,
                Some("2026-09-22T12:03:00Z".to_string()),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.burst_used_units, 190_000_000);
        assert_eq!(status.burst_remaining_units, 10_000_000);
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
                60_000_000,
                "2026-07-21T12:00:01Z",
            ))
            .await
            .unwrap();
        let second = store
            .reserve_finite_private_usage(reserve(
                "req-postgres-epoch-2",
                60_000_000,
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
                40_000_000,
                "2026-07-21T12:01:10Z",
            ))
            .await
            .unwrap();
        store
            .settle_finite_private_reservation(settle(
                second.reservation_id.unwrap(),
                "req-postgres-epoch-2",
                110_000_000,
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
        assert_eq!(status.burst_used_units, 150_000_000);
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
