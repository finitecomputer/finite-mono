use super::trials_tests::{customer, reservation, subscription};
use super::*;
use crate::test_support::with_isolated_postgres;
use crate::trials::*;

fn update(code: &str, revision: i32) -> UpdateTrialCode {
    UpdateTrialCode {
        code: code.into(),
        expected_code_revision: revision,
    }
}

#[tokio::test]
async fn trial_code_edits_persist_without_losing_seats_or_account_attribution() {
    with_isolated_postgres(|db| async move {
        let issued = db.create_trial_campaign(CreateTrialCampaign {
            name: "Workshop".into(), seat_limit: 2, trial_days: 7,
        }, "admin").await.unwrap();
        let org = customer(&db, "attendee").await;
        db.reserve_trial("attendee", reservation(&issued.code, &org, "attendee", "checkout")).await.unwrap();
        db.query_json("INSERT INTO projects(id,customer_org_id,owner_user_id,display_name,created_at,updated_at) SELECT 'first',$1,owner_user_id,'First bot',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP FROM customer_orgs WHERE id=$1 RETURNING to_jsonb(projects.*)", &[&org]).await;
        db.query_json("INSERT INTO projects(id,customer_org_id,owner_user_id,display_name,created_at,updated_at) SELECT 'second',$1,owner_user_id,'Second bot',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP FROM customer_orgs WHERE id=$1 RETURNING to_jsonb(projects.*)", &[&org]).await;
        let before = db.all("trial_redemptions").await;
        db.update_trial_code(&issued.id, update("workshop-2026", 0), "admin").await.unwrap();
        assert_eq!(before, db.all("trial_redemptions").await);
        assert!(db.trial_offer(&issued.code).await.is_err());
        assert_eq!(db.trial_offer("work SHOP 2026").await.unwrap().campaign_id, issued.id);
        let reopened = CoreStore::connect(&db.url).await.unwrap();
        reopened.migrate().await.unwrap();
        let listed = reopened.list_trial_campaigns().await.unwrap();
        assert_eq!(listed[0].code.as_deref(), Some("WORKSHOP2026"));
        assert_eq!(listed[0].code_revision, 1);
        assert_eq!(listed[0].reserved_seats, 1);
        assert_eq!(listed[0].seats_remaining, 1);
        assert_eq!(listed[0].redemptions[0].owner_email, "attendee@example.com");
        assert_eq!(listed[0].redemptions[0].agent_names, ["First bot", "Second bot"]);
        // An already-open Checkout completes after rotation using its attempt,
        // not the now-invalid old code. Its seat remains attached to this campaign.
        db.sync_trial_stripe_subscription(subscription(&org, "attendee", crate::BillingSubscriptionStatus::Trialing, 10), Some("checkout")).await.unwrap();
        let listed = db.list_trial_campaigns().await.unwrap();
        assert_eq!(listed[0].redeemed_seats, 1);
        assert_eq!(listed[0].reserved_seats, 0);
        assert_eq!(listed[0].seat_limit, 2);
        assert_eq!(listed[0].redemptions[0].customer_org_id, org);
    }).await;
}

#[tokio::test]
async fn trial_code_edits_reject_duplicate_invalid_and_stale_writes() {
    with_isolated_postgres(|db| async move {
        let a = db
            .create_trial_campaign(
                CreateTrialCampaign {
                    name: "A".into(),
                    seat_limit: 10,
                    trial_days: 7,
                },
                "admin",
            )
            .await
            .unwrap();
        let b = db
            .create_trial_campaign(
                CreateTrialCampaign {
                    name: "B".into(),
                    seat_limit: 10,
                    trial_days: 7,
                },
                "admin",
            )
            .await
            .unwrap();
        let (left, right) = tokio::join!(
            db.update_trial_code(&a.id, update("WORKSHOP2026", 0), "admin"),
            db.update_trial_code(&a.id, update("WORKSHOP2027", 0), "admin"),
        );
        assert_ne!(left.is_ok(), right.is_ok());
        let row = db.row("trial_campaigns", &a.id).await.unwrap();
        let code = row["code"].as_str().unwrap();
        let result = db
            .update_trial_code(&b.id, update(&code.to_lowercase(), 0), "admin")
            .await;
        assert!(matches!(
            result,
            Err(CoreError::TrialUnavailable(
                "Another campaign already uses this code."
            ))
        ));
        assert_eq!(db.trial_offer(&b.code).await.unwrap().campaign_id, b.id);
        for invalid in [
            "",
            "short",
            "WORKSHOP_2026",
            "WORKSHOP💥2026",
            "ａｂｃｄｅｆｇｈ",
        ] {
            assert!(
                db.update_trial_code(&a.id, update(invalid, 1), "admin")
                    .await
                    .is_err()
            );
        }
        assert!(
            db.update_trial_code(&a.id, update("WORKSHOP2028", 1), "")
                .await
                .is_err()
        );
        assert!(
            db.update_trial_code("missing", update("WORKSHOP2028", 0), "admin")
                .await
                .is_err()
        );
        assert_eq!(db.row("trial_campaigns", &a.id).await.unwrap(), row);
        let preview = CoreStore::connect_dry_run(&db.url).await.unwrap();
        preview
            .update_trial_code(&a.id, update("WORKSHOP2028", 1), "admin")
            .await
            .unwrap();
        assert_eq!(db.row("trial_campaigns", &a.id).await.unwrap(), row);
    })
    .await;
}

#[tokio::test]
async fn trial_code_migration_keeps_hash_only_codes_valid_until_explicit_replacement() {
    with_isolated_postgres(|db| async move {
        let issued = db.create_trial_campaign(CreateTrialCampaign {
            name: "Legacy".into(), seat_limit: 2, trial_days: 7,
        }, "admin").await.unwrap();
        let legacy = "trial_0123456789abcdef0123";
        let hash = hash_trial_code(legacy).unwrap();
        db.query_json("UPDATE trial_campaigns SET code=NULL,code_hash=$2 WHERE id=$1 RETURNING to_jsonb(trial_campaigns.*)", &[&issued.id, &hash]).await;
        // Reproduce an actual pre-migration row, then apply and reapply.
        db.query_json("ALTER TABLE trial_campaigns DROP COLUMN code, DROP COLUMN code_revision", &[]).await;
        db.migrate().await.unwrap();
        db.migrate().await.unwrap();
        let listed = db.list_trial_campaigns().await.unwrap();
        assert!(listed[0].code.is_none());
        assert_eq!(db.trial_offer(legacy).await.unwrap().campaign_id, issued.id);
        assert!(db.trial_offer(&legacy.to_uppercase()).await.is_err());
        db.update_trial_code(&issued.id, update("replacement2026", 0), "admin").await.unwrap();
        assert!(db.trial_offer(legacy).await.is_err());
        assert_eq!(db.trial_offer("replacement 2026").await.unwrap().campaign_id, issued.id);
    }).await;
}

#[tokio::test]
async fn trial_code_edits_serialize_with_capacity_and_reservations() {
    with_isolated_postgres(|db| async move {
        let issued = db
            .create_trial_campaign(
                CreateTrialCampaign {
                    name: "Concurrent".into(),
                    seat_limit: 1,
                    trial_days: 7,
                },
                "admin",
            )
            .await
            .unwrap();
        let org = customer(&db, "concurrent").await;
        let (edit, capacity, reserve) = tokio::join!(
            db.update_trial_code(&issued.id, update("CONCURRENT2026", 0), "admin"),
            db.increase_trial_capacity(
                &issued.id,
                IncreaseTrialCapacity {
                    seat_limit: 2,
                    expected_seat_limit: 1,
                },
                "admin"
            ),
            db.reserve_trial(
                "concurrent",
                reservation(&issued.code, &org, "concurrent", "race")
            ),
        );
        edit.unwrap();
        capacity.unwrap();
        let listed = db.list_trial_campaigns().await.unwrap();
        assert_eq!(listed[0].code.as_deref(), Some("CONCURRENT2026"));
        assert_eq!(listed[0].seat_limit, 2);
        assert_eq!(listed[0].reserved_seats, i64::from(reserve.is_ok()));
        assert_eq!(listed[0].seats_remaining, 2 - listed[0].reserved_seats);
        assert!(db.trial_offer(&issued.code).await.is_err());
        // Retrying via the new spelling either reuses the pre-edit hold or
        // creates it after the edit; it must never consume a second seat.
        db.reserve_trial(
            "concurrent",
            reservation("concurrent-2026", &org, "concurrent", "after"),
        )
        .await
        .unwrap();
        assert_eq!(
            db.list_trial_campaigns().await.unwrap()[0].reserved_seats,
            1
        );
    })
    .await;
}
