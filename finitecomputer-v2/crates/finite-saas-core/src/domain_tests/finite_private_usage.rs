use super::*;

#[tokio::test]
async fn finite_private_reserve_and_settle_keeps_core_as_usage_authority() {
    with_isolated_postgres(|db| async move {
        let grant = db
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "private@finite.vip".to_string(),
                workos_user_id: Some("user_workos_private".to_string()),
                limit_profile_id: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        let key = db
            .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id.clone(),
                raw_key: "fpk_live_secret".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        assert_ne!(key.key_hash, "fpk_live_secret");
        assert!(
            !serde_json::to_string(&db.all("finite_private_api_keys").await)
                .unwrap()
                .contains("fpk_live_secret")
        );

        let reserved = db
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: "req-private-1".to_string(),
                presented_api_key: "fpk_live_secret".to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "kimi-k2-6".to_string(),
                estimated_prompt_tokens: 120_000,
                estimated_completion_tokens: 4_096,
                estimated_usage_units: 250_000,
                usage_formula_version: "2026-05-26.v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: Some("2026-05-25T13:00:00Z".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(reserved.decision, "allow");
        assert_eq!(reserved.burst_limit_units, Some(100_000_000));
        assert_eq!(reserved.burst_remaining_units, Some(99_750_000));
        assert_eq!(reserved.weekly_limit_units, None);
        assert_eq!(reserved.weekly_remaining_units, None);
        let reservation_id = reserved.reservation_id.clone().unwrap();
        assert_eq!(
            db.finite_private_grant(&grant.id)
                .await
                .unwrap()
                .current_window_used_units,
            250_000
        );

        let settled = db
            .settle_finite_private_reservation(SettleFinitePrivateReservationInput {
                reservation_id: reservation_id.clone(),
                request_id: "req-private-1".to_string(),
                settlement: FinitePrivateSettlementKind::Actual,
                prompt_tokens: Some(120_000),
                completion_tokens: Some(1_200),
                usage_units: Some(160_000),
                usage_formula_version: "2026-05-26.v1".to_string(),
                upstream_status: Some(200),
                upstream_error_class: None,
                now: Some("2026-05-25T13:05:00Z".to_string()),
            })
            .await
            .unwrap();

        assert!(settled.settled);
        assert_eq!(
            db.finite_private_grant(&grant.id)
                .await
                .unwrap()
                .current_window_used_units,
            160_000
        );
        let reservation = &db
            .finite_private_reservation(&reservation_id)
            .await
            .unwrap();
        assert_eq!(reservation.status, FinitePrivateReservationStatus::Settled);
        assert_eq!(
            reservation.settlement_kind,
            Some(FinitePrivateSettlementKind::Actual)
        );
        assert_eq!(reservation.settled_usage_units, Some(160_000));
    })
    .await;
}

#[tokio::test]
async fn finite_private_reserve_denies_unknown_key_and_over_limit_without_upstream_work() {
    with_isolated_postgres(|db| async move {
        let unknown = db
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: "req-private-unknown".to_string(),
                presented_api_key: "fpk_live_unknown".to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "kimi-k2-6".to_string(),
                estimated_prompt_tokens: 100,
                estimated_completion_tokens: 100,
                estimated_usage_units: 200,
                usage_formula_version: "2026-05-26.v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: Some("2026-05-25T13:00:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(unknown.decision, "deny");
        assert_eq!(
            unknown.error.as_ref().map(|error| error.code.as_str()),
            Some("invalid_api_key")
        );
        assert!(db.all_finite_private_reservations().await.is_empty());

        let grant = db
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "private@finite.vip".to_string(),
                workos_user_id: Some("user_workos_private".to_string()),
                limit_profile_id: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        db.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
            grant_id: grant.id.clone(),
            raw_key: "fpk_live_secret".to_string(),
            project_id: None,
            agent_runtime_id: None,
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap();

        let denied = db
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: "req-private-over".to_string(),
                presented_api_key: "fpk_live_secret".to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "kimi-k2-6".to_string(),
                estimated_prompt_tokens: DEFAULT_FINITE_PRIVATE_BURST_LIMIT_UNITS + 1,
                estimated_completion_tokens: 0,
                estimated_usage_units: DEFAULT_FINITE_PRIVATE_BURST_LIMIT_UNITS + 1,
                usage_formula_version: "2026-05-26.v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: Some("2026-05-25T13:00:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(denied.decision, "deny");
        assert_eq!(
            denied.error.as_ref().map(|error| error.code.as_str()),
            Some("burst_window_limit_exceeded")
        );
        let denied_error = denied.error.as_ref().unwrap();
        assert!(denied_error.message.contains("2026-05-25T18:00:00Z"));
        assert!(denied_error.message.contains("(in 5h)"));
        assert_eq!(
            db.finite_private_grant(&grant.id)
                .await
                .unwrap()
                .current_window_used_units,
            0
        );
        assert!(db.all_finite_private_reservations().await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn finite_private_weekly_limit_denies_without_upstream_work() {
    with_isolated_postgres(|db| async move {
        db.exec(
            "INSERT INTO finite_private_limit_profiles \
             (id, burst_window_seconds, burst_limit_units, weekly_limit_units, \
              created_at, updated_at) \
             VALUES ('weekly-small', 3600, 10000000, 1000, \
             '2026-05-25T12:00:00Z', '2026-05-25T12:00:00Z')",
        )
        .await;
        let grant = db
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "private@finite.vip".to_string(),
                workos_user_id: Some("user_workos_private".to_string()),
                limit_profile_id: Some("weekly-small".to_string()),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        db.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
            grant_id: grant.id.clone(),
            raw_key: "fpk_live_secret".to_string(),
            project_id: None,
            agent_runtime_id: None,
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap();

        let allowed = db
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: "req-private-weekly-1".to_string(),
                presented_api_key: "fpk_live_secret".to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "glm-5.2".to_string(),
                estimated_prompt_tokens: 800,
                estimated_completion_tokens: 0,
                estimated_usage_units: 800,
                usage_formula_version: "2026-05-26.v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: Some("2026-05-25T13:00:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(allowed.decision, "allow");
        assert_eq!(allowed.weekly_remaining_units, Some(200));

        let denied = db
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: "req-private-weekly-2".to_string(),
                presented_api_key: "fpk_live_secret".to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "glm-5.2".to_string(),
                estimated_prompt_tokens: 300,
                estimated_completion_tokens: 0,
                estimated_usage_units: 300,
                usage_formula_version: "2026-05-26.v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: Some("2026-05-26T13:00:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(denied.decision, "deny");
        assert_eq!(
            denied.error.as_ref().map(|error| error.code.as_str()),
            Some("weekly_limit_exceeded")
        );
        let denied_error = denied.error.as_ref().unwrap();
        assert!(denied_error.message.contains("2026-06-01T13:00:00Z"));
        assert!(denied_error.message.contains("(in 6d)"));
        assert_eq!(db.table_len("finite_private_reservations").await, 1);
    })
    .await;
}

#[tokio::test]
async fn finite_private_status_does_not_start_or_roll_the_usage_window() {
    with_isolated_postgres(|db| async move {
        let grant = db
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "status-read@finite.vip".to_string(),
                workos_user_id: None,
                limit_profile_id: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        db.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
            grant_id: grant.id.clone(),
            raw_key: "fpk_live_status_read".to_string(),
            project_id: None,
            agent_runtime_id: None,
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap();

        let status = db
            .finite_private_usage_status_for_api_key(
                "fpk_live_status_read",
                false,
                Some("2026-05-26T13:00:00Z".to_string()),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.burst_used_units, 0);
        assert_eq!(status.burst_reset_at, "2026-05-26T18:00:00Z");
        let after_unstarted_read = &db.finite_private_grant(&grant.id).await.unwrap();
        assert!(after_unstarted_read.current_window_started_at.is_none());
        assert_eq!(
            after_unstarted_read.burst_window_epoch,
            grant.burst_window_epoch
        );

        db.reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
            request_id: "req-status-window".to_string(),
            presented_api_key: "fpk_live_status_read".to_string(),
            endpoint: "/v1/chat/completions".to_string(),
            model: "glm-5.2".to_string(),
            estimated_prompt_tokens: 10,
            estimated_completion_tokens: 0,
            estimated_usage_units: 10,
            usage_formula_version: "v1".to_string(),
            dashboard_url: "https://finite.computer/dashboard".to_string(),
            now: Some("2026-05-26T14:00:00Z".to_string()),
        })
        .await
        .unwrap();
        let started = db.finite_private_grant(&grant.id).await.unwrap().clone();
        assert_eq!(
            started.current_window_started_at.as_deref(),
            Some("2026-05-26T14:00:00Z")
        );

        let expired_status = db
            .finite_private_usage_status_for_api_key(
                "fpk_live_status_read",
                false,
                Some("2026-05-26T20:00:00Z".to_string()),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(expired_status.burst_used_units, 0);
        assert_eq!(expired_status.burst_reset_at, "2026-05-27T01:00:00Z");
        assert_eq!(db.finite_private_grant(&grant.id).await.unwrap(), started);
    })
    .await;
}

#[tokio::test]
async fn finite_private_daily_reset_is_once_per_utc_day_and_epoch_safe() {
    with_isolated_postgres(|db| async move {
        let grant = db
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "reset@finite.vip".to_string(),
                workos_user_id: Some("user_workos_reset".to_string()),
                limit_profile_id: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        db.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
            grant_id: grant.id.clone(),
            raw_key: "fpk_live_reset".to_string(),
            project_id: None,
            agent_runtime_id: None,
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap();
        let reserved = db
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: "req-before-reset".to_string(),
                presented_api_key: "fpk_live_reset".to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "glm-5.2".to_string(),
                estimated_prompt_tokens: 10,
                estimated_completion_tokens: 0,
                estimated_usage_units: 10,
                usage_formula_version: "v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: Some("2026-05-26T23:59:00Z".to_string()),
            })
            .await
            .unwrap();
        let old_epoch = db
            .finite_private_grant(&grant.id)
            .await
            .unwrap()
            .burst_window_epoch;

        let reset = db
            .claim_finite_private_daily_reset_for_api_key(
                "fpk_live_reset",
                Some("2026-05-26T23:59:30Z".to_string()),
            )
            .await
            .unwrap();
        assert!(reset.performed);
        assert_eq!(reset.status.burst_used_units, 0);
        assert_eq!(
            reset.status.free_daily_reset_available_again_at,
            "2026-05-27T00:00:00Z"
        );
        assert_eq!(
            db.finite_private_grant(&grant.id)
                .await
                .unwrap()
                .burst_window_epoch,
            old_epoch + 1
        );

        db.settle_finite_private_reservation(SettleFinitePrivateReservationInput {
            reservation_id: reserved.reservation_id.unwrap(),
            request_id: "req-before-reset".to_string(),
            settlement: FinitePrivateSettlementKind::Actual,
            prompt_tokens: Some(5),
            completion_tokens: Some(0),
            usage_units: Some(5),
            usage_formula_version: "v1".to_string(),
            upstream_status: Some(200),
            upstream_error_class: None,
            now: Some("2026-05-26T23:59:40Z".to_string()),
        })
        .await
        .unwrap();
        assert_eq!(
            db.finite_private_grant(&grant.id)
                .await
                .unwrap()
                .current_window_used_units,
            0
        );

        let repeated = db
            .claim_finite_private_daily_reset_for_api_key(
                "fpk_live_reset",
                Some("2026-05-26T23:59:50Z".to_string()),
            )
            .await
            .unwrap();
        assert!(!repeated.performed);
        let next_day = db
            .claim_finite_private_daily_reset_for_workos_user(
                "user_workos_reset",
                Some("2026-05-27T00:00:00Z".to_string()),
            )
            .await
            .unwrap()
            .unwrap();
        assert!(next_day.performed);
    })
    .await;
}

#[tokio::test]
async fn finite_private_threshold_notices_are_strongest_once_per_epoch() {
    with_isolated_postgres(|db| async move {
        let grant = db
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "notice@finite.vip".to_string(),
                workos_user_id: Some("user_workos_notice".to_string()),
                limit_profile_id: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        db.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
            grant_id: grant.id,
            raw_key: "fpk_live_notice".to_string(),
            project_id: None,
            agent_runtime_id: None,
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap();

        for (request_id, units, at) in [
            ("req-notice-25", 76_000_000, "2026-05-26T13:00:00Z"),
            ("req-notice-10", 16_000_000, "2026-05-26T13:10:00Z"),
        ] {
            let reserved = db
                .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                    request_id: request_id.to_string(),
                    presented_api_key: "fpk_live_notice".to_string(),
                    endpoint: "/v1/chat/completions".to_string(),
                    model: "glm-5.2".to_string(),
                    estimated_prompt_tokens: units,
                    estimated_completion_tokens: 0,
                    estimated_usage_units: units,
                    usage_formula_version: "v1".to_string(),
                    dashboard_url: "https://finite.computer/dashboard".to_string(),
                    now: Some(at.to_string()),
                })
                .await
                .unwrap();
            db.settle_finite_private_reservation(SettleFinitePrivateReservationInput {
                reservation_id: reserved.reservation_id.unwrap(),
                request_id: request_id.to_string(),
                settlement: FinitePrivateSettlementKind::Actual,
                prompt_tokens: Some(units),
                completion_tokens: Some(0),
                usage_units: Some(units),
                usage_formula_version: "v1".to_string(),
                upstream_status: Some(200),
                upstream_error_class: None,
                now: Some(at.to_string()),
            })
            .await
            .unwrap();
            let status = db
                .finite_private_usage_status_for_api_key(
                    "fpk_live_notice",
                    true,
                    Some(at.to_string()),
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                status
                    .notice
                    .as_ref()
                    .map(|notice| notice.threshold_remaining_percent),
                Some(if units == 76_000_000 { 25 } else { 10 })
            );
            assert!(
                status
                    .notice
                    .unwrap()
                    .message
                    .contains(&status.burst_reset_at)
            );
        }
        let repeated = db
            .finite_private_usage_status_for_api_key(
                "fpk_live_notice",
                true,
                Some("2026-05-26T13:20:00Z".to_string()),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(repeated.notice, None);
    })
    .await;
}

#[tokio::test]
async fn finite_private_settlement_retry_is_idempotent_but_mismatch_conflicts() {
    with_isolated_postgres(|db| async move {
        let grant = db
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "settle@finite.vip".to_string(),
                workos_user_id: None,
                limit_profile_id: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        db.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
            grant_id: grant.id,
            raw_key: "fpk_live_settle".to_string(),
            project_id: None,
            agent_runtime_id: None,
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap();
        let reserved = db
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: "req-settle-retry".to_string(),
                presented_api_key: "fpk_live_settle".to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "glm-5.2".to_string(),
                estimated_prompt_tokens: 100,
                estimated_completion_tokens: 0,
                estimated_usage_units: 100,
                usage_formula_version: "v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        let input = SettleFinitePrivateReservationInput {
            reservation_id: reserved.reservation_id.unwrap(),
            request_id: "req-settle-retry".to_string(),
            settlement: FinitePrivateSettlementKind::Actual,
            prompt_tokens: Some(80),
            completion_tokens: Some(0),
            usage_units: Some(80),
            usage_formula_version: "v1".to_string(),
            upstream_status: Some(200),
            upstream_error_class: None,
            now: Some(NOW.to_string()),
        };
        assert!(
            db.settle_finite_private_reservation(input.clone())
                .await
                .unwrap()
                .settled
        );
        assert!(
            db.settle_finite_private_reservation(input.clone())
                .await
                .unwrap()
                .settled
        );
        let mut mismatch = input;
        mismatch.usage_units = Some(81);
        assert!(matches!(
            db.settle_finite_private_reservation(mismatch).await,
            Err(CoreError::FinitePrivateReservationAlreadySettled)
        ));
    })
    .await;
}
