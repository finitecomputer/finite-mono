use super::*;
use crate::BillingSubscriptionStatus;
use crate::test_support::{TestDb, with_isolated_postgres};
use crate::trials::*;

#[tokio::test]
async fn trial_campaign_dry_run_does_not_persist_a_usable_code() {
    with_isolated_postgres(|db| async move {
        let preview = CoreStore::connect_dry_run(&db.url).await.unwrap();
        let campaign = preview
            .create_trial_campaign(
                CreateTrialCampaign {
                    name: "Preview".into(),
                    seat_limit: 10,
                    trial_days: 7,
                },
                "admin",
            )
            .await
            .unwrap();
        assert!(db.list_trial_campaigns().await.unwrap().is_empty());
        assert!(db.trial_offer(&campaign.code).await.is_err());
    })
    .await;
}

pub(super) async fn customer(db: &TestDb, suffix: &str) -> String {
    db.link_stripe_customer(LinkStripeCustomerInput {
        verified_email: format!("{suffix}@example.com"),
        workos_user_id: suffix.into(),
        stripe_customer_id: format!("cus_{suffix}"),
        now: None,
    })
    .await
    .unwrap()
    .customer_org_id
}
pub(super) fn reservation(code: &str, org: &str, user: &str, attempt: &str) -> ReserveTrial {
    ReserveTrial {
        code: code.into(),
        customer_org_id: org.into(),
        stripe_customer_id: format!("cus_{user}"),
        stripe_session_id: format!("cs_{attempt}"),
        attempt_id: attempt.into(),
        trial_days: 7,
        checkout_expires_at: time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
    }
}
pub(super) fn subscription(
    org: &str,
    user: &str,
    status: BillingSubscriptionStatus,
    time: i64,
) -> SyncStripeSubscriptionInput {
    SyncStripeSubscriptionInput {
        customer_org_id: Some(org.into()),
        stripe_customer_id: format!("cus_{user}"),
        stripe_subscription_id: format!("sub_{user}"),
        stripe_price_id: Some("price_standard".into()),
        expected_stripe_price_id: Some("price_standard".into()),
        subscription_status: status,
        current_period_end: Some(
            (time::OffsetDateTime::now_utc() + time::Duration::days(7))
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap(),
        ),
        cancel_at_period_end: false,
        stripe_event_id: Some(format!("evt_{user}_{time}")),
        stripe_event_created: Some(time),
        now: None,
    }
}
async fn overview(db: &TestDb, user: &str) -> BillingOverview {
    db.billing_overview(LinkVerifiedUserInput {
        verified_email: format!("{user}@example.com"),
        workos_user_id: user.into(),
        now: None,
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn trial_capacity_serializes_last_seat_and_releases_only_verified_expiry() {
    with_isolated_postgres(|db| async move {
        // Reapplying the migration must preserve existing campaigns and holds.
        let campaign = db
            .create_trial_campaign(
                CreateTrialCampaign {
                    name: "Summit".into(),
                    seat_limit: 1,
                    trial_days: 7,
                },
                "admin",
            )
            .await
            .unwrap();
        let a = customer(&db, "alice").await;
        let b = customer(&db, "bob").await;
        let (ra, rb) = tokio::join!(
            db.reserve_trial("alice", reservation(&campaign.code, &a, "alice", "a")),
            db.reserve_trial("bob", reservation(&campaign.code, &b, "bob", "b"))
        );
        assert_ne!(
            ra.is_ok(),
            rb.is_ok(),
            "exactly one customer gets the last seat"
        );
        let (winner, org, attempt, loser, loser_org) = if ra.is_ok() {
            ("alice", &a, "a", "bob", &b)
        } else {
            ("bob", &b, "b", "alice", &a)
        };
        let reused = db
            .reserve_trial(winner, reservation(&campaign.code, org, winner, "retry"))
            .await
            .unwrap();
        assert_eq!(reused.stripe_session_id, format!("cs_{attempt}"));
        assert!(
            db.reserve_trial(
                "intruder",
                reservation(&campaign.code, org, winner, "steal")
            )
            .await
            .is_err()
        );
        db.migrate().await.unwrap();
        let counts = db.list_trial_campaigns().await.unwrap();
        assert_eq!(counts[0].reserved_seats, 1);
        assert_eq!(counts[0].seats_remaining, 0);
        db.expire_trial(ExpireTrial {
            stripe_session_id: format!("cs_{attempt}"),
            stripe_customer_id: format!("cus_{winner}"),
        })
        .await
        .unwrap();
        db.expire_trial(ExpireTrial {
            stripe_session_id: format!("cs_{attempt}"),
            stripe_customer_id: format!("cus_{winner}"),
        })
        .await
        .unwrap();
        assert!(
            db.reserve_trial(winner, reservation(&campaign.code, org, winner, attempt))
                .await
                .is_err()
        );
        db.reserve_trial(loser, reservation(&campaign.code, loser_org, loser, "new"))
            .await
            .unwrap();
        assert_eq!(
            db.list_trial_campaigns().await.unwrap()[0].reserved_seats,
            1
        );
    })
    .await;
}

#[tokio::test]
async fn trial_expiry_before_reservation_cannot_strand_a_seat() {
    with_isolated_postgres(|db| async move {
        let c = db
            .create_trial_campaign(
                CreateTrialCampaign {
                    name: "Event".into(),
                    seat_limit: 1,
                    trial_days: 7,
                },
                "admin",
            )
            .await
            .unwrap();
        let org = customer(&db, "alice").await;
        db.expire_trial(ExpireTrial {
            stripe_session_id: "cs_early".into(),
            stripe_customer_id: "cus_alice".into(),
        })
        .await
        .unwrap();
        assert!(
            db.reserve_trial("alice", reservation(&c.code, &org, "alice", "early"))
                .await
                .is_err()
        );
        assert_eq!(
            db.list_trial_campaigns().await.unwrap()[0].seats_remaining,
            1
        );
        assert!(
            db.create_trial_campaign(
                CreateTrialCampaign {
                    name: "Bad".into(),
                    seat_limit: 0,
                    trial_days: 7
                },
                "admin"
            )
            .await
            .is_err()
        );
        assert!(db.trial_offer("wrong-code").await.is_err());
        assert_eq!(db.trial_offer(&c.code).await.unwrap().trial_days, 7);
    })
    .await;
}

#[tokio::test]
async fn trial_subscription_lifecycle_preserves_seat_and_blocks_only_trial_accounts() {
    with_isolated_postgres(|db| async move {
        let c = db
            .create_trial_campaign(
                CreateTrialCampaign {
                    name: "Event".into(),
                    seat_limit: 2,
                    trial_days: 7,
                },
                "admin",
            )
            .await
            .unwrap();
        let org = customer(&db, "alice").await;
        let before = overview(&db, "alice").await;
        assert!(before.trial_access.is_none());
        db.reserve_trial("alice", reservation(&c.code, &org, "alice", "checkout"))
            .await
            .unwrap();
        use crate::BillingSubscriptionStatus::*;
        // No matching reservation means billing writes roll back as well.
        assert!(
            db.sync_trial_stripe_subscription(
                subscription(&org, "alice", Trialing, 10),
                Some("wrong")
            )
            .await
            .is_err()
        );
        assert!(!overview(&db, "alice").await.can_create_agent);
        db.sync_trial_stripe_subscription(
            subscription(&org, "alice", Trialing, 10),
            Some("checkout"),
        )
        .await
        .unwrap();
        db.sync_trial_stripe_subscription(
            subscription(&org, "alice", Trialing, 10),
            Some("checkout"),
        )
        .await
        .unwrap();
        let active = overview(&db, "alice").await;
        assert!(active.can_create_agent);
        assert!(!active.trial_access.unwrap().blocked);
        let request = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "alice@example.com".into(),
                workos_user_id: "alice".into(),
                display_name: "Trial Agent".into(),
                launch_code: String::new(),
                idempotency_key: "launch".into(),
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(request.request.customer_org_id, org);
        db.sync_trial_stripe_subscription(
            subscription(&org, "alice", PastDue, 20),
            Some("checkout"),
        )
        .await
        .unwrap();
        assert!(overview(&db, "alice").await.trial_access.unwrap().blocked);
        // Delayed trial event cannot restore access.
        db.sync_trial_stripe_subscription(
            subscription(&org, "alice", Trialing, 11),
            Some("checkout"),
        )
        .await
        .unwrap();
        assert!(overview(&db, "alice").await.trial_access.unwrap().blocked);
        db.sync_trial_stripe_subscription(
            subscription(&org, "alice", Active, 30),
            Some("checkout"),
        )
        .await
        .unwrap();
        assert!(!overview(&db, "alice").await.trial_access.unwrap().blocked);
        db.sync_trial_stripe_subscription(
            subscription(&org, "alice", Canceled, 40),
            Some("checkout"),
        )
        .await
        .unwrap();
        db.expire_trial(ExpireTrial {
            stripe_session_id: "cs_checkout".into(),
            stripe_customer_id: "cus_alice".into(),
        })
        .await
        .unwrap();
        let counts = db.list_trial_campaigns().await.unwrap();
        assert_eq!(counts[0].redeemed_seats, 1);
        assert_eq!(counts[0].seats_remaining, 1);
        assert_eq!(
            counts[0].redemptions[0].owner_workos_user_id.as_deref(),
            Some("alice")
        );
        assert!(
            db.reserve_trial("alice", reservation(&c.code, &org, "alice", "again"))
                .await
                .is_err()
        );
        assert!(
            db.row("agent_creation_requests", &request.request.id)
                .await
                .is_some()
        );
        let other = customer(&db, "existing").await;
        db.sync_stripe_subscription(subscription(&other, "existing", Active, 10))
            .await
            .unwrap();
        db.sync_stripe_subscription(subscription(&other, "existing", PastDue, 20))
            .await
            .unwrap();
        assert!(overview(&db, "existing").await.trial_access.is_none());
    })
    .await;
}

#[tokio::test]
async fn trial_capacity_increase_is_absolute_guarded_and_preserves_attribution() {
    with_isolated_postgres(|db| async move {
        let campaign = db
            .create_trial_campaign(
                CreateTrialCampaign {
                    name: "Workshop".into(),
                    seat_limit: 10,
                    trial_days: 7,
                },
                "operator",
            )
            .await
            .unwrap();
        let org = customer(&db, "workshop").await;
        db.reserve_trial(
            "workshop",
            reservation(&campaign.code, &org, "workshop", "workshop"),
        )
        .await
        .unwrap();
        let before = db.all("trial_redemptions").await;
        let increase = |seat_limit, expected_seat_limit| IncreaseTrialCapacity {
            seat_limit,
            expected_seat_limit,
        };
        let preview = CoreStore::connect_dry_run(&db.url).await.unwrap();
        preview
            .increase_trial_capacity(&campaign.id, increase(15, 10), "operator")
            .await
            .unwrap();
        assert_eq!(db.list_trial_campaigns().await.unwrap()[0].seat_limit, 10);
        // Two operators starting from the same view cannot overwrite one another.
        let (a, b) = tokio::join!(
            db.increase_trial_capacity(&campaign.id, increase(15, 10), "operator"),
            db.increase_trial_capacity(&campaign.id, increase(20, 10), "other")
        );
        assert_ne!(a.is_ok(), b.is_ok());
        let current = db.list_trial_campaigns().await.unwrap()[0].seat_limit;
        for target in [0, current - 1, current, 10001] {
            assert!(
                db.increase_trial_capacity(&campaign.id, increase(target, current), "operator")
                    .await
                    .is_err()
            );
        }
        assert!(
            db.increase_trial_capacity("missing", increase(15, 10), "operator")
                .await
                .is_err()
        );
        assert!(
            db.increase_trial_capacity(&campaign.id, increase(25, current), "")
                .await
                .is_err()
        );
        db.increase_trial_capacity(&campaign.id, increase(25, current), "operator")
            .await
            .unwrap();
        let listed = db.list_trial_campaigns().await.unwrap();
        assert_eq!(listed[0].seat_limit, 25);
        assert_eq!(listed[0].reserved_seats, 1);
        assert_eq!(listed[0].seats_remaining, 24);
        assert_eq!(db.all("trial_redemptions").await, before);
        assert_eq!(db.trial_offer(&campaign.code).await.unwrap().trial_days, 7);
    })
    .await;
}

#[tokio::test]
async fn trial_admin_billing_projection_distinguishes_checkout_trial_and_paid() {
    with_isolated_postgres(|db| async move {
        let campaign = db
            .create_trial_campaign(
                CreateTrialCampaign {
                    name: "Workshop".into(),
                    seat_limit: 2,
                    trial_days: 7,
                },
                "operator",
            )
            .await
            .unwrap();
        let org = customer(&db, "projection").await;
        db.reserve_trial(
            "projection",
            reservation(&campaign.code, &org, "projection", "projection"),
        )
        .await
        .unwrap();
        assert!(
            db.list_trial_campaigns().await.unwrap()[0].redemptions[0]
                .trial_access
                .is_none()
        );
        let mut trial = subscription(&org, "projection", BillingSubscriptionStatus::Trialing, 10);
        trial.current_period_end = Some("2099-01-01T00:00:00Z".into());
        db.sync_trial_stripe_subscription(trial, Some("projection"))
            .await
            .unwrap();
        let listed = db.list_trial_campaigns().await.unwrap();
        let access = listed[0].redemptions[0].trial_access.as_ref().unwrap();
        assert!(!access.blocked);
        assert_eq!(access.subscription_status.as_deref(), Some("trialing"));
        assert_eq!(access.period_end.as_deref(), Some("2099-01-01T00:00:00Z"));
        let mut ended = subscription(&org, "projection", BillingSubscriptionStatus::Trialing, 11);
        ended.current_period_end = Some("2020-01-01T00:00:00Z".into());
        db.sync_trial_stripe_subscription(ended, None)
            .await
            .unwrap();
        assert!(
            db.list_trial_campaigns().await.unwrap()[0].redemptions[0]
                .trial_access
                .as_ref()
                .unwrap()
                .blocked
        );
        db.sync_trial_stripe_subscription(
            subscription(&org, "projection", BillingSubscriptionStatus::Active, 12),
            None,
        )
        .await
        .unwrap();
        let listed = db.list_trial_campaigns().await.unwrap();
        let access = listed[0].redemptions[0].trial_access.as_ref().unwrap();
        assert_eq!(access.subscription_status.as_deref(), Some("active"));
        assert!(!access.blocked);
        assert_eq!(listed[0].redeemed_seats, 1);
        assert_eq!(listed[0].seats_remaining, 1);
    })
    .await;
}

#[tokio::test]
async fn trial_workshop_codes_are_readable_normalized_and_legacy_compatible() {
    with_isolated_postgres(|db| async move {
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..128 {
            let code = generate_trial_code().unwrap();
            assert_eq!(code.len(), 19);
            assert_eq!(code.split('-').map(str::len).collect::<Vec<_>>(), [4, 4, 4, 4]);
            assert!(code.chars().all(|c| c == '-' || "23456789ABCDEFGHJKLMNPQRSTUVWXYZ".contains(c)));
            assert!(seen.insert(code));
        }
        let issued = db.create_trial_campaign(CreateTrialCampaign {
            name: "Readable".into(), seat_limit: 10, trial_days: 7,
        }, "operator").await.unwrap();
        // Displayed spelling must still work on an older Core during rollout/rollback.
        assert_eq!(hash_trial_code(&issued.code).unwrap(), hash_launch_code(&issued.code).unwrap());
        let compact = issued.code.replace('-', "");
        assert_eq!(db.trial_offer(&compact).await.unwrap().campaign_id, issued.id);
        let mut typo = issued.code.clone();
        typo.replace_range(0..1, if issued.code.starts_with('2') { "3" } else { "2" });
        assert!(db.trial_offer(&typo).await.is_err());
        let typed = issued.code.replace('-', " ").to_ascii_lowercase();
        assert_eq!(db.trial_offer(&typed).await.unwrap().campaign_id, issued.id);
        let org = customer(&db, "readable").await;
        db.reserve_trial("readable", reservation(&typed, &org, "readable", "readable")).await.unwrap();
        let reused = db.reserve_trial("readable", reservation(&compact, &org, "readable", "retry_spelling")).await.unwrap();
        assert_eq!(reused.stripe_session_id, "cs_readable");
        let stored = db.row("trial_campaigns", &issued.id).await.unwrap();
        assert_eq!(stored["code_hash"], hash_trial_code(&issued.code).unwrap());
        assert_eq!(stored["code"], issued.code);
        assert_eq!(db.list_trial_campaigns().await.unwrap()[0].code.as_deref(), Some(issued.code.as_str()));
        let legacy = "trial_0123456789abcdef0123";
        let legacy_hash = hash_launch_code(legacy).unwrap();
        db.query_json("UPDATE trial_campaigns SET code_hash=$2,code=NULL WHERE id=$1 RETURNING to_jsonb(trial_campaigns.*)", &[&issued.id, &legacy_hash]).await;
        assert_eq!(db.trial_offer(legacy).await.unwrap().campaign_id, issued.id);
        assert!(db.trial_offer(&legacy.to_ascii_uppercase()).await.is_err());
        assert_eq!(hash_trial_code(legacy).unwrap(), legacy_hash);
        let legacy_org = customer(&db, "legacy").await;
        db.reserve_trial("legacy", reservation(legacy, &legacy_org, "legacy", "legacy")).await.unwrap();
    }).await;
}
