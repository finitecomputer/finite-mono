use super::*;
use std::collections::BTreeSet;

#[tokio::test]
async fn finite_private_grant_can_start_as_pending_email_and_later_link_workos() {
    with_isolated_postgres(|db| async move {
        let pending_grant = db
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "friend@finite.vip".to_string(),
                workos_user_id: None,
                limit_profile_id: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        let pending_user = db.user(&pending_grant.user_id).await.unwrap();
        assert_eq!(pending_user.email, "friend@finite.vip");
        assert_eq!(pending_user.status, UserLinkStatus::Pending);
        assert_eq!(pending_user.workos_user_id, None);

        let linked_grant = db
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "friend@finite.vip".to_string(),
                workos_user_id: Some("user_workos_friend".to_string()),
                limit_profile_id: None,
                now: Some("2026-05-26T13:00:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(linked_grant.id, pending_grant.id);
        let linked_user = db.user(&linked_grant.user_id).await.unwrap();
        assert_eq!(linked_user.status, UserLinkStatus::Linked);
        assert_eq!(
            linked_user.workos_user_id.as_deref(),
            Some("user_workos_friend")
        );
    })
    .await;
}

#[tokio::test]
async fn finite_private_admin_operations_write_audit_events_without_raw_keys() {
    with_isolated_postgres(|db| async move {
        let grant = db
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "friend@finite.vip".to_string(),
                workos_user_id: None,
                limit_profile_id: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        let key = db
            .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id.clone(),
                raw_key: "fpk_live_first_secret".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: Some("2026-05-26T12:01:00Z".to_string()),
            })
            .await
            .unwrap();
        db.reset_finite_private_usage_window(ResetFinitePrivateUsageWindowInput {
            grant_id: grant.id.clone(),
            now: Some("2026-05-26T12:02:00Z".to_string()),
        })
        .await
        .unwrap();
        let rotated = db
            .rotate_finite_private_api_key(RotateFinitePrivateApiKeyInput {
                key_id: key.id.clone(),
                raw_key: "fpk_live_second_secret".to_string(),
                now: Some("2026-05-26T12:03:00Z".to_string()),
            })
            .await
            .unwrap();
        db.revoke_finite_private_grant(RevokeFinitePrivateGrantInput {
            grant_id: grant.id.clone(),
            now: Some("2026-05-26T12:04:00Z".to_string()),
        })
        .await
        .unwrap();

        let events = db.finite_private_admin_audit_events().await.unwrap();
        let actions = events
            .iter()
            .map(|event| event.action.as_str())
            .collect::<BTreeSet<_>>();
        for expected in [
            "finite_private.grant.approve",
            "finite_private.api_key.issue",
            "finite_private.grant.reset_window",
            "finite_private.api_key.rotate",
            "finite_private.grant.revoke",
        ] {
            assert!(actions.contains(expected));
        }
        assert_eq!(
            db.finite_private_admin_audit_events()
                .await
                .unwrap()
                .iter()
                .filter(|event| event.grant_id.as_deref() == Some(grant.id.as_str()))
                .count(),
            db.table_len("finite_private_admin_audit_events").await
        );
        assert_eq!(
            db.finite_private_api_key(&rotated.id).await.unwrap().status,
            FinitePrivateApiKeyStatus::Revoked
        );
        let audit_json =
            serde_json::to_string(&db.finite_private_admin_audit_events().await.unwrap()).unwrap();
        assert!(!audit_json.contains("fpk_live_first_secret"));
        assert!(!audit_json.contains("fpk_live_second_secret"));
    })
    .await;
}

#[tokio::test]
async fn admin_friend_key_issue_mirrors_cli_and_records_admin_audit() {
    with_isolated_postgres(|db| async move {
        let raw_key = "fpk_live_test_friend_key_material_0001";
        let issued = db
            .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                friend_email: "Friend@Finite.VIP".to_string(),
                limit_profile_id: None,
                raw_key: raw_key.to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();

        assert_eq!(issued.grant.status, FinitePrivateGrantStatus::Active);
        assert_eq!(issued.grant.limit_profile_id, "finite-private-generous-v2");
        assert_eq!(issued.api_key.status, FinitePrivateApiKeyStatus::Active);
        assert_ne!(issued.api_key.key_hash, raw_key);
        assert!(issued.api_key.project_id.is_none());
        assert!(issued.api_key.agent_runtime_id.is_none());

        let resolved = db
            .finite_private_key_and_grant(raw_key)
            .await
            .expect("issued raw key should validate");
        assert_eq!(resolved.0.id, issued.api_key.id);
        assert_eq!(resolved.1.id, issued.grant.id);

        let events = db.finite_private_admin_audit_events().await.unwrap();
        let admin_event = events
            .iter()
            .find(|event| event.action == "finite_private.friend_key.admin_issue")
            .expect("friend key issue should record an admin audit event");
        assert_eq!(admin_event.actor, "admin@finite.vip");
        assert_eq!(
            admin_event.api_key_id.as_deref(),
            Some(issued.api_key.id.as_str())
        );
    })
    .await;
}

#[tokio::test]
async fn admin_rotate_invalidates_old_raw_key_and_revoke_disables_key() {
    with_isolated_postgres(|db| async move {
        let old_raw = "fpk_live_old_raw_key_material_000000001";
        let issued = db
            .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                friend_email: "friend@finite.vip".to_string(),
                limit_profile_id: None,
                raw_key: old_raw.to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();

        let new_raw = "fpk_live_new_raw_key_material_000000002";
        let rotated = db
            .admin_rotate_finite_private_api_key(AdminRotateFinitePrivateApiKeyInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                key_id: issued.api_key.id.clone(),
                raw_key: new_raw.to_string(),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap();
        assert_ne!(rotated.id, issued.api_key.id);
        assert_eq!(rotated.status, FinitePrivateApiKeyStatus::Active);

        assert!(
            db.finite_private_key_and_grant(old_raw).await.is_none(),
            "old raw key must stop validating after rotate"
        );
        let resolved = db
            .finite_private_key_and_grant(new_raw)
            .await
            .expect("new raw key should validate");
        assert_eq!(resolved.0.id, rotated.id);
        assert_eq!(
            db.finite_private_api_key(&issued.api_key.id)
                .await
                .unwrap()
                .status,
            FinitePrivateApiKeyStatus::Revoked
        );

        let revoked = db
            .admin_revoke_finite_private_api_key(AdminRevokeFinitePrivateApiKeyInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                key_id: rotated.id.clone(),
                now: Some("2026-05-25T14:00:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(revoked.status, FinitePrivateApiKeyStatus::Revoked);
        assert!(db.finite_private_key_and_grant(new_raw).await.is_none());

        let actions = db
            .finite_private_admin_audit_events()
            .await
            .unwrap()
            .iter()
            .filter(|event| event.actor == "admin@finite.vip")
            .map(|event| event.action.clone())
            .collect::<Vec<_>>();
        assert!(actions.contains(&"finite_private.api_key.admin_rotate".to_string()));
        assert!(actions.contains(&"finite_private.api_key.admin_revoke".to_string()));
    })
    .await;
}

#[tokio::test]
async fn admin_window_reset_clears_burst_window_but_not_weekly_reservations() {
    with_isolated_postgres(|db| async move {
        let raw_key = "fpk_live_reset_raw_key_material_00000003";
        let issued = db
            .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                friend_email: "friend@finite.vip".to_string(),
                limit_profile_id: None,
                raw_key: raw_key.to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();

        let decision = db
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: "req-1".to_string(),
                presented_api_key: raw_key.to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "kimi-k2-6".to_string(),
                estimated_prompt_tokens: 10,
                estimated_completion_tokens: 10,
                estimated_usage_units: 1_000,
                usage_formula_version: "2026-05-26.v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap();
        assert_eq!(decision.decision, "allow");
        assert_eq!(
            db.finite_private_grant(&issued.grant.id)
                .await
                .unwrap()
                .current_window_used_units,
            1_000
        );

        let reset = db
            .admin_reset_finite_private_usage_window(AdminResetFinitePrivateUsageWindowInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                grant_id: issued.grant.id.clone(),
                now: Some("2026-05-25T14:00:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(reset.current_window_used_units, 0);
        assert_eq!(
            reset.current_window_started_at.as_deref(),
            Some("2026-05-25T14:00:00Z")
        );

        // Weekly usage is a rolling reservation window; reset must not touch it.
        let (weekly_used, _) = db
            .finite_private_weekly_usage(
                &issued.grant.id,
                parse_time("2026-05-25T14:00:00Z").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(weekly_used, 1_000);

        let events = db.finite_private_admin_audit_events().await.unwrap();
        let admin_event = events
            .iter()
            .find(|event| event.action == "finite_private.grant.admin_window_reset")
            .expect("window reset should record an admin audit event");
        assert_eq!(admin_event.actor, "admin@finite.vip");
    })
    .await;
}
