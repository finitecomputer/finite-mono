use super::*;

#[test]
fn only_active_and_trialing_grant_agent_creation() {
    for status in [
        BillingSubscriptionStatus::Incomplete,
        BillingSubscriptionStatus::IncompleteExpired,
        BillingSubscriptionStatus::PastDue,
        BillingSubscriptionStatus::Canceled,
        BillingSubscriptionStatus::Unpaid,
        BillingSubscriptionStatus::Paused,
    ] {
        assert!(!status.can_create_agent(), "{status:?} must not entitle");
    }
    assert!(BillingSubscriptionStatus::Active.can_create_agent());
    assert!(BillingSubscriptionStatus::Trialing.can_create_agent());
}

#[test]
fn status_wire_strings_are_stripe_subscription_statuses() {
    // These strings are Stripe's `subscription.status` vocabulary, pinned
    // by the dashboard webhook mapper. Round-trip every variant through
    // serde and the parser so the DB column, JSON API, and parser can
    // never drift apart.
    for status in [
        BillingSubscriptionStatus::Incomplete,
        BillingSubscriptionStatus::IncompleteExpired,
        BillingSubscriptionStatus::Trialing,
        BillingSubscriptionStatus::Active,
        BillingSubscriptionStatus::PastDue,
        BillingSubscriptionStatus::Canceled,
        BillingSubscriptionStatus::Unpaid,
        BillingSubscriptionStatus::Paused,
    ] {
        let wire = status.as_str();
        assert_eq!(parse_billing_subscription_status(wire), Some(status));
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, format!("\"{wire}\""));
        assert_eq!(
            serde_json::from_str::<BillingSubscriptionStatus>(&json).unwrap(),
            status
        );
    }
    assert_eq!(parse_billing_subscription_status("not_a_status"), None);
}

#[test]
fn replacement_requires_terminal_current_and_entitling_incoming() {
    use BillingSubscriptionStatus::*;
    // No current subscription: any incoming status may establish one.
    for incoming in [
        Active, Trialing, PastDue, Canceled, Unpaid, Paused, Incomplete,
    ] {
        assert!(should_replace_stripe_subscription(None, incoming));
    }
    // Terminal current: only an entitling incoming status replaces it.
    for current in [Canceled, IncompleteExpired] {
        assert!(should_replace_stripe_subscription(Some(current), Active));
        assert!(should_replace_stripe_subscription(Some(current), Trialing));
        for incoming in [
            PastDue,
            Canceled,
            Unpaid,
            Paused,
            Incomplete,
            IncompleteExpired,
        ] {
            assert!(!should_replace_stripe_subscription(Some(current), incoming));
        }
    }
    // Anything else live (active, trialing, past_due, unpaid, paused,
    // incomplete): a different subscription id never replaces it.
    for current in [Active, Trialing, PastDue, Unpaid, Paused, Incomplete] {
        assert!(!should_replace_stripe_subscription(Some(current), Active));
        assert!(!should_replace_stripe_subscription(Some(current), Canceled));
    }
}

#[test]
fn stale_event_guard_requires_both_timestamps_and_strictly_older_event() {
    // Strictly older delivery for the same subscription is stale.
    assert!(stripe_event_is_stale(Some(100), Some(99)));
    // Equal or newer deliveries are applied.
    assert!(!stripe_event_is_stale(Some(100), Some(100)));
    assert!(!stripe_event_is_stale(Some(100), Some(101)));
    // Missing timestamps never drop the event: legacy rows and deliveries
    // without `event.created` must still sync.
    assert!(!stripe_event_is_stale(Some(100), None));
    assert!(!stripe_event_is_stale(None, Some(99)));
    assert!(!stripe_event_is_stale(None, None));
}

#[test]
fn sync_request_json_is_the_dashboard_contract() {
    // The dashboard webhook posts exactly these camelCase fields. Pin the
    // wire shape so a rename here cannot silently break the dashboard.
    let request: SyncStripeSubscriptionRequest = serde_json::from_str(
        r#"{
            "customerOrgId": "org_1",
            "stripeCustomerId": "cus_1",
            "stripeSubscriptionId": "sub_1",
            "stripePriceId": "price_standard",
            "subscriptionStatus": "active",
            "currentPeriodEnd": "2026-01-01T00:00:00Z",
            "cancelAtPeriodEnd": true,
            "stripeEventId": "evt_1",
            "stripeEventCreated": 1234,
            "now": null
        }"#,
    )
    .unwrap();
    assert_eq!(request.stripe_customer_id, "cus_1");
    assert_eq!(
        request.subscription_status,
        BillingSubscriptionStatus::Active
    );
    assert_eq!(request.stripe_event_created, Some(1234));
    let round_trip = serde_json::to_value(&request).unwrap();
    assert!(round_trip.get("stripeCustomerId").is_some());
    assert!(round_trip.get("stripeSubscriptionId").is_some());
    assert!(round_trip.get("stripeEventCreated").is_some());

    let link: LinkStripeCustomerRequest =
        serde_json::from_str(r#"{ "stripeCustomerId": "cus_2", "now": null }"#).unwrap();
    assert_eq!(link.stripe_customer_id, "cus_2");
}
