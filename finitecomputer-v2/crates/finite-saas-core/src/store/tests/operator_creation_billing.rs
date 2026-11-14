//! What an operator-created Agent must never do to its owner's account:
//! use up their self-serve allowance across Stripe changes, change their
//! Finite Private grant, or meet an event trial that could stop it.

use super::operator_creation::{
    ADMIN_EMAIL, ADMIN_WORKOS, confirmed, input, link, org_of, owner_workos,
};
use super::*;
use crate::store::trials_tests::{customer, reservation};
use crate::trials::CreateTrialCampaign;

async fn set_subscription(
    db: &TestDb,
    email: &str,
    run: &str,
    status: BillingSubscriptionStatus,
    event: &str,
) {
    db.sync_stripe_subscription(SyncStripeSubscriptionInput {
        customer_org_id: Some(org_of(db, email).await),
        stripe_customer_id: format!("cus_{run}"),
        stripe_subscription_id: format!("sub_{run}"),
        stripe_price_id: Some("price_standard".to_string()),
        expected_stripe_price_id: Some("price_standard".to_string()),
        subscription_status: status,
        current_period_end: Some("2099-01-01T00:00:00Z".to_string()),
        cancel_at_period_end: false,
        stripe_event_id: Some(format!("evt_{run}_{event}")),
        stripe_event_created: None,
        now: None,
    })
    .await
    .unwrap();
}

async fn activate_billing(db: &TestDb, email: &str, workos_user_id: &str, run: &str) {
    db.link_stripe_customer(LinkStripeCustomerInput {
        verified_email: email.to_string(),
        workos_user_id: workos_user_id.to_string(),
        stripe_customer_id: format!("cus_{run}"),
        now: None,
    })
    .await
    .unwrap();
    set_subscription(db, email, run, BillingSubscriptionStatus::Active, "start").await;
}

async fn self_serve(db: &TestDb, email: &str, workos_user_id: &str, key: &str) -> CoreResult<()> {
    db.request_agent_creation(RequestAgentCreationInput {
        verified_email: email.to_string(),
        workos_user_id: workos_user_id.to_string(),
        display_name: "Own Agent".to_string(),
        launch_code: String::new(),
        idempotency_key: key.to_string(),
        now: None,
    })
    .await
    .map(|_| ())
}

/// The person can create exactly the one Agent their subscription allows.
async fn exactly_one_own_agent(db: &TestDb, email: &str, workos: &str, round: &str) {
    self_serve(db, email, workos, &format!("own-{round}-1"))
        .await
        .unwrap();
    let error = self_serve(db, email, workos, &format!("own-{round}-2"))
        .await
        .unwrap_err();
    assert!(
        matches!(error, CoreError::AgentCreationEntitlementExhausted),
        "{email}: {error:?}"
    );
}

/// Subscription start, lapse and recovery never trade the operator Agent
/// for the person's own paid slot: Core leaves it out of the count, and
/// the operator path never writes the entitlement Stripe sync owns.
#[tokio::test]
async fn operator_agents_never_use_the_owners_allowance_across_billing_changes() {
    with_isolated_postgres(|db| async move {
        link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        // Minted sponsored for a new person, who later signs in and pays.
        let minted = db
            .admin_create_agent_for_owner(input("newbie@example.com", "op-newbie"))
            .await
            .unwrap();
        assert!(minted.record.owner.customer_org_created);
        assert!(db.all("agent_creation_entitlements").await.is_empty());
        let newbie = owner_workos("newbie@example.com");
        link(&db, "newbie@example.com", &newbie).await;
        activate_billing(&db, "newbie@example.com", &newbie, "newbie").await;
        exactly_one_own_agent(&db, "newbie@example.com", &newbie, "paid").await;

        // Already paying when the operator adds an Agent; then the
        // subscription lapses (past due, then canceled) and recovers.
        activate_billing(&db, "lapse@example.com", "workos_lapse", "lapse").await;
        db.admin_create_agent_for_owner(confirmed("lapse@example.com", "workos_lapse", "op-lapse"))
            .await
            .unwrap();
        for (status, event) in [
            (BillingSubscriptionStatus::PastDue, "past-due"),
            (BillingSubscriptionStatus::Canceled, "canceled"),
            (BillingSubscriptionStatus::Active, "recovered"),
        ] {
            set_subscription(&db, "lapse@example.com", "lapse", status, event).await;
        }
        exactly_one_own_agent(&db, "lapse@example.com", "workos_lapse", "recovered").await;

        // Signed in, never paid, operator Agent first, pays afterwards.
        link(&db, "later@example.com", "workos_later").await;
        db.admin_create_agent_for_owner(confirmed("later@example.com", "workos_later", "op-later"))
            .await
            .unwrap();
        let org = db
            .row("customer_orgs", &org_of(&db, "later@example.com").await)
            .await
            .unwrap();
        assert_eq!(org["billing_class"], "standard", "existing class kept");
        activate_billing(&db, "later@example.com", "workos_later", "later").await;
        exactly_one_own_agent(&db, "later@example.com", "workos_later", "paid").await;
    })
    .await;
}

async fn lease(db: &TestDb, runner: &str) -> AgentCreationRequest {
    db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
        runner_id: runner.into(),
        lease_token: format!("lease-{runner}"),
        lease_seconds: Some(300),
        runner_capacity: Some(RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::Kata],
            ..RunnerLeaseCapacity::default()
        }),
        source_host_id: None,
        now: None,
    })
    .await
    .unwrap()
    .expect("a request leases")
    .request
}

async fn provision_key(
    db: &TestDb,
    request: &AgentCreationRequest,
    runner: &str,
) -> CoreResult<()> {
    db.provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
        trial_restore_key: None,
        request_id: request.id.clone(),
        runner_id: runner.into(),
        lease_token: format!("lease-{runner}"),
        source_host_id: Some(format!("host-{runner}")),
        source_machine_id: Some(format!("machine-{runner}")),
        now: None,
    })
    .await
    .map(|_| ())
}

fn grant_state(grant: &serde_json::Value) -> serde_json::Value {
    serde_json::json!([
        grant["limit_profile_id"],
        grant["status"],
        grant["current_window_used_units"],
        grant["burst_window_epoch"],
    ])
}

#[tokio::test]
async fn an_operator_agent_never_changes_the_owners_finite_private_grant() {
    with_isolated_postgres(|db| async move {
        link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        let grant = db
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "private@example.com".into(),
                workos_user_id: Some("workos_private".into()),
                limit_profile_id: Some(crate::FINITE_PRIVATE_5X_LIMIT_PROFILE.into()),
                now: None,
            })
            .await
            .unwrap();
        db.query_json(
            "UPDATE finite_private_grants SET current_window_used_units = 123,
               current_window_started_at = NOW() WHERE id = $1 RETURNING to_jsonb(id)",
            &[&grant.id],
        )
        .await;
        let before = grant_state(&db.row("finite_private_grants", &grant.id).await.unwrap());
        db.admin_create_agent_for_owner(confirmed("private@example.com", "workos_private", "private"))
            .await
            .unwrap();
        let request = lease(&db, "private").await;
        provision_key(&db, &request, "private").await.unwrap();
        let after = db.row("finite_private_grants", &grant.id).await.unwrap();
        assert_eq!(grant_state(&after), before, "profile, status and usage kept");
        assert_eq!(after["limit_profile_id"], crate::FINITE_PRIVATE_5X_LIMIT_PROFILE);

        // A revoked grant stays revoked: creation is refused up front, and a
        // grant revoked after creation fails the key step instead of
        // being reactivated.
        db.query_json(
            "UPDATE finite_private_grants SET status = 'revoked' WHERE id = $1 RETURNING to_jsonb(id)",
            &[&grant.id],
        )
        .await;
        let error = db
            .admin_create_agent_for_owner(confirmed("private@example.com", "workos_private", "again"))
            .await
            .unwrap_err();
        assert!(super::operator_creation::refusal(error).contains("Finite Private"));
        db.query_json(
            "UPDATE finite_private_grants SET status = 'active' WHERE id = $1 RETURNING to_jsonb(id)",
            &[&grant.id],
        )
        .await;
        db.admin_create_agent_for_owner(confirmed("private@example.com", "workos_private", "later"))
            .await
            .unwrap();
        db.query_json(
            "UPDATE finite_private_grants SET status = 'revoked' WHERE id = $1 RETURNING to_jsonb(id)",
            &[&grant.id],
        )
        .await;
        let request = lease(&db, "revoked").await;
        let error = provision_key(&db, &request, "revoked").await.unwrap_err();
        assert!(matches!(error, CoreError::FinitePrivateGrantNotActive), "{error:?}");
        let revoked = db.row("finite_private_grants", &grant.id).await.unwrap();
        assert_eq!(revoked["status"], "revoked");
        assert_eq!(revoked["current_window_used_units"], 123);

        // A never-signed-in owner gets a fresh default grant on first key.
        db.admin_create_agent_for_owner(input("fresh@example.com", "fresh"))
            .await
            .unwrap();
        let request = lease(&db, "fresh").await;
        provision_key(&db, &request, "fresh").await.unwrap();
        let fresh = db
            .query_json(
                "SELECT to_jsonb(g) FROM finite_private_grants g WHERE user_id = $1",
                &[&request.owner_user_id],
            )
            .await;
        assert_eq!(fresh[0]["status"], "active");
        assert_eq!(fresh[0]["limit_profile_id"], crate::DEFAULT_FINITE_PRIVATE_LIMIT_PROFILE);
    })
    .await;
}

#[tokio::test]
async fn an_org_holding_an_operator_agent_can_never_start_an_event_trial() {
    with_isolated_postgres(|db| async move {
        link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        let campaign = db
            .create_trial_campaign(
                CreateTrialCampaign {
                    name: "Workshop".into(),
                    seat_limit: 5,
                    trial_days: 7,
                },
                "admin",
            )
            .await
            .unwrap();
        // `customer` links barred@example.com as WorkOS user "barred" with
        // a Stripe customer and no subscription: trial-eligible.
        let org = customer(&db, "barred").await;
        db.admin_create_agent_for_owner(confirmed("barred@example.com", "barred", "barred"))
            .await
            .unwrap();
        let Err(error) = db
            .reserve_trial(
                "barred",
                reservation(&campaign.code, &org, "barred", "barred-checkout"),
            )
            .await
        else {
            panic!("an operator org reserved a trial seat");
        };
        assert!(
            matches!(&error, CoreError::TrialUnavailable(message) if message.contains("operator")),
            "{error:?}"
        );
        assert!(db.all("trial_redemptions").await.is_empty());

        // An eligible neighbour still can.
        let open = customer(&db, "open").await;
        db.reserve_trial(
            "open",
            reservation(&campaign.code, &open, "open", "open-checkout"),
        )
        .await
        .unwrap();
    })
    .await;
}

#[tokio::test]
async fn a_trial_never_stops_an_operator_agent() {
    with_isolated_postgres(|db| async move {
        link(&db, ADMIN_EMAIL, ADMIN_WORKOS).await;
        db.upsert_runtime_artifact(UpsertRuntimeArtifactInput {
            id: "artifact-trial".into(),
            kind: RuntimeArtifactKind::OciImage,
            reference: format!(
                "ghcr.io/finitecomputer/agent-runtime:t@sha256:{}",
                "c".repeat(64)
            ),
            version_label: "t".into(),
            source_git_sha: None,
            finitec_version: None,
            hermes_source_ref: None,
            finite_platform_plugin_ref: None,
            state_schema_version: "state-v1".into(),
            base_image: None,
            canary_runtime_id: None,
            recover_known_good_chat: false,
            promoted: true,
            now: None,
        })
        .await
        .unwrap();
        let org = customer(&db, "stopped").await;
        db.admin_create_agent_for_owner(confirmed("stopped@example.com", "stopped", "stopped"))
            .await
            .unwrap();
        let request = lease(&db, "trial").await;
        let capabilities = RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
            restart: true,
            recover_known_good_chat: false,
            runtime_upgrade: true,
            stop: true,
            runtime_retirement: false,
            trial_archive: false,
        });
        db.complete_agent_creation_request(CompleteAgentCreationRequestInput {
            request_id: request.id.clone(),
            runner_id: "trial".into(),
            lease_token: "lease-trial".into(),
            source_host_id: "host-trial".into(),
            source_machine_id: "machine-trial".into(),
            runtime_artifact_id: Some("artifact-trial".into()),
            state_schema_version: Some("state-v1".into()),
            provider_runtime_handle: None,
            contact_endpoint: None,
            runtime_capabilities: Some(capabilities),
            display_name: None,
            hostname: None,
            runtime_host: Some("host-trial".into()),
            runtime_status: Some(RuntimeSummaryStatus::Online),
            active_inference_profile: None,
            hermes_available: Some(true),
            published_app_urls: Vec::new(),
            agent_npub: None,
            now: None,
        })
        .await
        .unwrap();
        // A redeemed, unpaid, past-deadline trial written as older Core
        // could have written it: the org's access is blocked.
        db.query_json(
            "WITH campaign AS (
               INSERT INTO trial_campaigns (id, name, code_hash, seat_limit, trial_days, created_by_workos_user_id)
               VALUES ('campaign_old', 'Old', 'hash_old', 5, 1, 'w') RETURNING id)
             INSERT INTO trial_redemptions
               (id, campaign_id, customer_org_id, attempt_id, stripe_session_id, state, checkout_expires_at, redeemed_at)
             SELECT 'redemption_old', id, $1, 'attempt_old', 'cs_old', 'redeemed', NOW(), NOW() - INTERVAL '30 days'
             FROM campaign RETURNING to_jsonb(id)",
            &[&org],
        )
        .await;
        let stops = || async {
            db.query_json(
                "SELECT to_jsonb(kind) FROM runtime_control_requests WHERE kind = 'stop'",
                &[],
            )
            .await
            .len()
        };
        db.reconcile_trial_runtimes(None).await.unwrap();
        assert_eq!(stops().await, 0, "the operator Agent keeps running");
        assert!(
            db.query_json("SELECT to_jsonb(s) FROM trial_runtime_suspensions s", &[])
                .await
                .is_empty()
        );
        // Control: the same org without the operator record would be stopped.
        db.query_json(
            "DELETE FROM operator_agent_creations RETURNING to_jsonb(id)",
            &[],
        )
        .await;
        db.reconcile_trial_runtimes(None).await.unwrap();
        assert_eq!(stops().await, 1);
    })
    .await;
}
