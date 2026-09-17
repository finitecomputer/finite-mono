use super::*;

#[tokio::test]
async fn paid_self_serve_agent_creation_requires_active_stripe_billing() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;

        let unpaid = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "paid@finite.vip".to_string(),
                workos_user_id: "user_workos_paid".to_string(),
                display_name: "Paid Agent".to_string(),
                launch_code: String::new(),
                idempotency_key: "paid-submit-before-billing".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(unpaid, CoreError::BillingRequired));
        assert!(db.all_users().await.is_empty());
        assert!(db.all_customer_orgs().await.is_empty());

        db.link_stripe_customer(LinkStripeCustomerInput {
            verified_email: "paid@finite.vip".to_string(),
            workos_user_id: "user_workos_paid".to_string(),
            stripe_customer_id: "cus_paid".to_string(),
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap();
        let org_id = db
            .personal_org_by_owner(&db.user_by_email("paid@finite.vip").await.unwrap().id)
            .await
            .unwrap()
            .id;
        db.sync_stripe_subscription(SyncStripeSubscriptionInput {
            customer_org_id: Some(org_id.clone()),
            stripe_customer_id: "cus_paid".to_string(),
            stripe_subscription_id: "sub_paid".to_string(),
            stripe_price_id: Some("price_standard".to_string()),
            expected_stripe_price_id: Some("price_standard".to_string()),
            subscription_status: BillingSubscriptionStatus::Active,
            current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
            cancel_at_period_end: false,
            stripe_event_id: Some("evt_paid_active".to_string()),
            stripe_event_created: None,
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap();

        let overview = db
            .billing_overview(LinkVerifiedUserInput {
                verified_email: "paid@finite.vip".to_string(),
                workos_user_id: "user_workos_paid".to_string(),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap();
        assert!(overview.can_create_agent);
        assert!(!overview.requires_billing);
        assert_eq!(
            overview
                .agent_creation_entitlement
                .as_ref()
                .and_then(|entitlement| entitlement.launch_code.as_deref()),
            None
        );

        let created = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "paid@finite.vip".to_string(),
                workos_user_id: "user_workos_paid".to_string(),
                display_name: "Paid Agent".to_string(),
                launch_code: String::new(),
                idempotency_key: "paid-submit".to_string(),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap();
        assert_eq!(created.request.requested_launch_code, None);
        assert_eq!(
            db.customer_org(&org_id).await.unwrap().billing_class,
            BillingClass::Standard
        );
        let lease = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-paid-1".to_string(),
                source_host_id: None,
                lease_token: "paid-lease-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some("2026-05-25T13:01:00Z".to_string()),
            })
            .await
            .unwrap()
            .expect("paid request should be leased");
        let provisioned = db
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-paid-1".to_string(),
                lease_token: "paid-lease-1".to_string(),
                source_host_id: Some("paid-host-1".to_string()),
                source_machine_id: Some("paid-agent-001".to_string()),
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .await
            .unwrap();
        let completed = db
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-paid-1".to_string(),
                lease_token: "paid-lease-1".to_string(),
                source_host_id: "paid-host-1".to_string(),
                source_machine_id: "paid-agent-001".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
                provider_runtime_handle: None,
                contact_endpoint: None,
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: None,
                hostname: None,
                runtime_host: Some("paid-host-1".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: vec!["https://paid-agent.example.com/contact".to_string()],
                agent_npub: None,
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .await
            .unwrap();
        let runtime_id = completed.request.agent_runtime_id.unwrap();
        assert!(db.agent_runtime(&runtime_id).await.is_some());

        db.sync_stripe_subscription(SyncStripeSubscriptionInput {
            customer_org_id: Some(org_id),
            stripe_customer_id: "cus_paid".to_string(),
            stripe_subscription_id: "sub_paid".to_string(),
            stripe_price_id: Some("price_standard".to_string()),
            expected_stripe_price_id: Some("price_standard".to_string()),
            subscription_status: BillingSubscriptionStatus::PastDue,
            current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
            cancel_at_period_end: false,
            stripe_event_id: Some("evt_paid_past_due".to_string()),
            stripe_event_created: None,
            now: Some("2026-05-25T14:00:00Z".to_string()),
        })
        .await
        .unwrap();
        let blocked_after_past_due = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "paid@finite.vip".to_string(),
                workos_user_id: "user_workos_paid".to_string(),
                display_name: "Second Paid Agent".to_string(),
                launch_code: String::new(),
                idempotency_key: "paid-submit-2".to_string(),
                now: Some("2026-05-25T14:01:00Z".to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(blocked_after_past_due, CoreError::BillingRequired));
        assert!(db.agent_runtime(&runtime_id).await.is_some());
        assert!(
            db.all("project_runtime_links")
                .await
                .iter()
                .any(|link| link["agent_runtime_id"] == runtime_id.as_str()
                    && link["active"] == true)
        );
        assert_eq!(
            db.finite_private_api_key(&provisioned.api_key.id)
                .await
                .unwrap()
                .status,
            FinitePrivateApiKeyStatus::Active
        );
    })
    .await;
}

#[tokio::test]
async fn stripe_subscription_sync_ignores_non_current_subscription_events() {
    with_isolated_postgres(|db| async move {
        db.link_stripe_customer(LinkStripeCustomerInput {
            verified_email: "paid@finite.vip".to_string(),
            workos_user_id: "user_workos_paid".to_string(),
            stripe_customer_id: "cus_paid".to_string(),
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap();
        let org_id = db
            .personal_org_by_owner(&db.user_by_email("paid@finite.vip").await.unwrap().id)
            .await
            .unwrap()
            .id;
        let current = db
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: "cus_paid".to_string(),
                stripe_subscription_id: "sub_current".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::Active,
                current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_current_active".to_string()),
                stripe_event_created: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        assert_eq!(
            current.stripe_subscription_id.as_deref(),
            Some("sub_current")
        );

        let ignored = db
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: "cus_paid".to_string(),
                stripe_subscription_id: "sub_second".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::Active,
                current_period_end: Some("2026-07-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_second_active".to_string()),
                stripe_event_created: None,
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap();
        assert_eq!(
            ignored.stripe_subscription_id.as_deref(),
            Some("sub_current")
        );
        assert_eq!(
            db.customer_billing_account(&org_id)
                .await
                .unwrap()
                .last_stripe_event_id
                .as_deref(),
            Some("evt_current_active")
        );

        db.sync_stripe_subscription(SyncStripeSubscriptionInput {
            customer_org_id: Some(org_id.clone()),
            stripe_customer_id: "cus_paid".to_string(),
            stripe_subscription_id: "sub_current".to_string(),
            stripe_price_id: Some("price_standard".to_string()),
            expected_stripe_price_id: Some("price_standard".to_string()),
            subscription_status: BillingSubscriptionStatus::Canceled,
            current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
            cancel_at_period_end: false,
            stripe_event_id: Some("evt_current_canceled".to_string()),
            stripe_event_created: None,
            now: Some("2026-05-25T14:00:00Z".to_string()),
        })
        .await
        .unwrap();

        let replacement = db
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: "cus_paid".to_string(),
                stripe_subscription_id: "sub_replacement".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::Active,
                current_period_end: Some("2026-08-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_replacement_active".to_string()),
                stripe_event_created: None,
                now: Some("2026-05-25T15:00:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(
            replacement.stripe_subscription_id.as_deref(),
            Some("sub_replacement")
        );

        let old_event = db
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: "cus_paid".to_string(),
                stripe_subscription_id: "sub_current".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::PastDue,
                current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_current_late_past_due".to_string()),
                stripe_event_created: None,
                now: Some("2026-05-25T16:00:00Z".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(
            old_event.stripe_subscription_id.as_deref(),
            Some("sub_replacement")
        );
        assert_eq!(
            db.customer_billing_account(&org_id)
                .await
                .unwrap()
                .subscription_status
                .unwrap(),
            BillingSubscriptionStatus::Active
        );
    })
    .await;
}

#[tokio::test]
async fn stripe_subscription_sync_ignores_stale_out_of_order_event() {
    with_isolated_postgres(|db| async move {
        // Event-ordering guard: for the SAME subscription, a webhook whose Stripe
        // `event.created` predates the last applied event must be ignored, so a
        // stale `active` delivered after `canceled` cannot resurrect billing.
        db.link_stripe_customer(LinkStripeCustomerInput {
            verified_email: "order@finite.vip".to_string(),
            workos_user_id: "user_workos_order".to_string(),
            stripe_customer_id: "cus_order".to_string(),
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap();
        let org_id = db
            .personal_org_by_owner(&db.user_by_email("order@finite.vip").await.unwrap().id)
            .await
            .unwrap()
            .id;

        sync_order_subscription(
            &db,
            &org_id,
            BillingSubscriptionStatus::Active,
            "evt_active",
            1_000,
        )
        .await;
        let canceled = sync_order_subscription(
            &db,
            &org_id,
            BillingSubscriptionStatus::Canceled,
            "evt_canceled",
            2_000,
        )
        .await;
        assert_eq!(
            canceled.subscription_status,
            Some(BillingSubscriptionStatus::Canceled)
        );

        // Stale `active` (created BEFORE the canceled event) arrives last.
        let stale = sync_order_subscription(
            &db,
            &org_id,
            BillingSubscriptionStatus::Active,
            "evt_active_stale",
            1_500,
        )
        .await;
        assert_eq!(
            stale.subscription_status,
            Some(BillingSubscriptionStatus::Canceled),
            "stale out-of-order webhook must be ignored; billing stays canceled"
        );
        assert_eq!(stale.last_stripe_event_id.as_deref(), Some("evt_canceled"));
    })
    .await;
}

#[tokio::test]
async fn stripe_subscription_sync_requires_standard_price_before_entitlement() {
    with_isolated_postgres(|db| async move {
        db.link_stripe_customer(LinkStripeCustomerInput {
            verified_email: "paid@finite.vip".to_string(),
            workos_user_id: "user_workos_paid".to_string(),
            stripe_customer_id: "cus_paid".to_string(),
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap();
        let org_id = db
            .personal_org_by_owner(&db.user_by_email("paid@finite.vip").await.unwrap().id)
            .await
            .unwrap()
            .id;

        let wrong_price = db
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: "cus_paid".to_string(),
                stripe_subscription_id: "sub_wrong_price".to_string(),
                stripe_price_id: Some("price_other".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::Active,
                current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_wrong_price_active".to_string()),
                stripe_event_created: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            wrong_price,
            CoreError::StripeSubscriptionPriceMismatch
        ));
        assert!(db.all("agent_creation_entitlements").await.is_empty());

        let missing_expected_price = db
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id),
                stripe_customer_id: "cus_paid".to_string(),
                stripe_subscription_id: "sub_missing_expected".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: None,
                subscription_status: BillingSubscriptionStatus::Trialing,
                current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_missing_expected_trialing".to_string()),
                stripe_event_created: None,
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            missing_expected_price,
            CoreError::MissingStripeStandardPriceId
        ));
        assert!(db.all("agent_creation_entitlements").await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn stripe_subscription_lapse_preserves_launch_code_entitlement() {
    with_isolated_postgres(|db| async move {
        let launch_code = issue_test_launch_code(&db).await;
        let launch_code_id = issued_launch_code_id(&db, &launch_code).await;
        db.request_agent_creation(RequestAgentCreationInput {
            verified_email: "bridge@finite.vip".to_string(),
            workos_user_id: "user_workos_bridge".to_string(),
            display_name: "Bridge Agent".to_string(),
            launch_code: launch_code.clone(),
            idempotency_key: "bridge-submit".to_string(),
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap();
        let org_id = db
            .personal_org_by_owner(&db.user_by_email("bridge@finite.vip").await.unwrap().id)
            .await
            .unwrap()
            .id;
        assert_eq!(
            db.all("agent_creation_entitlements")
                .await
                .iter()
                .find(|entitlement| entitlement["customer_org_id"] == org_id.as_str())
                .and_then(|entitlement| entitlement["launch_code"].as_str()),
            Some(launch_code_id.as_str())
        );

        db.sync_stripe_subscription(SyncStripeSubscriptionInput {
            customer_org_id: Some(org_id.clone()),
            stripe_customer_id: "cus_bridge".to_string(),
            stripe_subscription_id: "sub_bridge".to_string(),
            stripe_price_id: Some("price_standard".to_string()),
            expected_stripe_price_id: Some("price_standard".to_string()),
            subscription_status: BillingSubscriptionStatus::Active,
            current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
            cancel_at_period_end: false,
            stripe_event_id: Some("evt_bridge_active".to_string()),
            stripe_event_created: None,
            now: Some(LATER.to_string()),
        })
        .await
        .unwrap();
        assert_eq!(
            db.agent_creation_entitlement(&org_id)
                .await
                .unwrap()
                .launch_code
                .as_deref(),
            Some(launch_code_id.as_str())
        );

        db.sync_stripe_subscription(SyncStripeSubscriptionInput {
            customer_org_id: Some(org_id.clone()),
            stripe_customer_id: "cus_bridge".to_string(),
            stripe_subscription_id: "sub_bridge".to_string(),
            stripe_price_id: Some("price_standard".to_string()),
            expected_stripe_price_id: Some("price_standard".to_string()),
            subscription_status: BillingSubscriptionStatus::PastDue,
            current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
            cancel_at_period_end: false,
            stripe_event_id: Some("evt_bridge_past_due".to_string()),
            stripe_event_created: None,
            now: Some("2026-05-25T14:00:00Z".to_string()),
        })
        .await
        .unwrap();
        let entitlement = &db.agent_creation_entitlement(&org_id).await.unwrap();
        assert_eq!(
            entitlement.launch_code.as_deref(),
            Some(launch_code_id.as_str())
        );
        assert_eq!(entitlement.allowed_new_agent_runtimes, 1);
    })
    .await;
}
