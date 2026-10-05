use super::trials_tests::{customer, reservation, subscription};
use super::*;
use crate::test_support::{TestDb, with_isolated_postgres};
use crate::trials::CreateTrialCampaign;
use crate::{BillingSubscriptionStatus, RunnerClass, RunnerLeaseCapacity};

const START: &str = "2030-01-01T00:00:00Z";
const BEFORE: &str = "2030-01-07T23:59:59Z";
const DEADLINE: &str = "2030-01-08T00:00:00Z";
const AFTER: &str = "2030-01-08T00:00:02Z";

async fn redeemed_trial(db: &TestDb) -> String {
    let campaign = db
        .create_trial_campaign(
            CreateTrialCampaign {
                name: "Access gates".into(),
                seat_limit: 1,
                trial_days: 7,
            },
            "admin",
        )
        .await
        .unwrap();
    let org = customer(db, "gates").await;
    db.reserve_trial(
        "gates",
        reservation(&campaign.code, &org, "gates", "gates-checkout"),
    )
    .await
    .unwrap();
    let mut input = subscription(&org, "gates", BillingSubscriptionStatus::Trialing, 10);
    input.current_period_end = Some(DEADLINE.into());
    db.sync_trial_stripe_subscription(input, Some("gates-checkout"))
        .await
        .unwrap();
    org
}

fn creation(now: &str) -> RequestAgentCreationInput {
    RequestAgentCreationInput {
        verified_email: "gates@example.com".into(),
        workos_user_id: "gates".into(),
        display_name: "Trial Agent".into(),
        launch_code: String::new(),
        idempotency_key: "trial-agent".into(),
        now: Some(now.into()),
    }
}

fn creation_lease(now: &str) -> LeaseAgentCreationRequestInput {
    LeaseAgentCreationRequestInput {
        runner_id: "gates-runner".into(),
        source_host_id: None,
        lease_token: "gates-creation-lease".into(),
        lease_seconds: Some(1),
        runner_capacity: None,
        now: Some(now.into()),
    }
}

fn capabilities() -> crate::RuntimeCapabilitiesEnvelope {
    crate::RuntimeCapabilitiesEnvelope::V1(crate::RuntimeCapabilitiesV1 {
        restart: true,
        recover_known_good_chat: true,
        runtime_upgrade: true,
        stop: true,
        runtime_retirement: true,
        trial_archive: false,
    })
}

fn control_lease(now: &str) -> LeaseRuntimeControlRequestInput {
    LeaseRuntimeControlRequestInput {
        runner_id: "gates-runner".into(),
        source_host_id: Some("gates-host".into()),
        lease_token: "gates-control-lease".into(),
        lease_seconds: Some(1),
        runner_capacity: Some(RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::Kata],
            runtime_capabilities: Some(capabilities()),
            ..RunnerLeaseCapacity::default()
        }),
        now: Some(now.into()),
    }
}

async fn running_trial(db: &TestDb) -> Project {
    redeemed_trial(db).await;
    db.request_agent_creation(creation(START)).await.unwrap();
    let lease = db
        .lease_agent_creation_request(creation_lease(START))
        .await
        .unwrap()
        .unwrap();
    db.complete_agent_creation_request(CompleteAgentCreationRequestInput {
        request_id: lease.request.id,
        runner_id: "gates-runner".into(),
        lease_token: "gates-creation-lease".into(),
        source_host_id: "gates-host".into(),
        source_machine_id: "gates-machine".into(),
        runtime_artifact_id: Some("artifact-postgres-fixture".into()),
        state_schema_version: Some("state-v1".into()),
        provider_runtime_handle: None,
        contact_endpoint: None,
        runtime_capabilities: Some(capabilities()),
        display_name: None,
        hostname: None,
        runtime_host: Some("gates-host".into()),
        runtime_status: Some(RuntimeSummaryStatus::Online),
        active_inference_profile: None,
        hermes_available: Some(true),
        published_app_urls: Vec::new(),
        agent_npub: None,
        now: Some(START.into()),
    })
    .await
    .unwrap()
    .project
}

async fn blocked(db: &TestDb, org: &str, now: &str) -> bool {
    let mut client = db.connection().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET TRANSACTION READ ONLY").await.unwrap();
    let access = trials_access::trial_access(&*tx, org, now)
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();
    access.blocked
}

#[tokio::test]
async fn trial_access_deadline_is_exclusive_read_only_and_has_a_redemption_fallback() {
    with_isolated_postgres(|db| async move {
        let org = redeemed_trial(&db).await;
        assert!(!blocked(&db, &org, BEFORE).await);
        assert!(blocked(&db, &org, DEADLINE).await);
        db.exec(&format!(
            "UPDATE customer_billing_accounts SET current_period_end = NULL WHERE customer_org_id = '{org}';
             UPDATE trial_redemptions SET redeemed_at = '{START}' WHERE customer_org_id = '{org}'"
        ))
        .await;
        assert!(!blocked(&db, &org, BEFORE).await);
        assert!(blocked(&db, &org, DEADLINE).await);
        // No billing row also fails closed for a redeemed standard trial.
        db.exec(&format!(
            "DELETE FROM customer_billing_accounts WHERE customer_org_id = '{org}'"
        ))
        .await;
        assert!(blocked(&db, &org, DEADLINE).await);
        let client = db.connection().await.unwrap();
        assert!(
            client
                .query_one(
                    "SELECT core_trial_access_blocked($1, $2::text::timestamptz)",
                    &[&org, &DEADLINE],
                )
                .await
                .unwrap()
                .get::<_, bool>(0)
        );
    })
    .await;
}

#[tokio::test]
async fn trial_access_exempts_paid_sponsored_grandfathered_and_unredeemed_accounts() {
    with_isolated_postgres(|db| async move {
        let org = redeemed_trial(&db).await;
        for status in ["past_due", "unpaid", "canceled", "paused"] {
            db.exec(&format!(
                "UPDATE customer_billing_accounts SET subscription_status = '{status}' WHERE customer_org_id = '{org}'"
            ))
            .await;
            assert!(blocked(&db, &org, BEFORE).await);
        }
        for class in ["sponsored", "grandfathered"] {
            db.exec(&format!(
                "UPDATE customer_orgs SET billing_class = '{class}' WHERE id = '{org}'"
            ))
            .await;
            assert!(!blocked(&db, &org, DEADLINE).await);
        }
        db.exec(&format!(
            "UPDATE customer_orgs SET billing_class = 'standard' WHERE id = '{org}';
             UPDATE customer_billing_accounts SET subscription_status = 'active' WHERE customer_org_id = '{org}'"
        ))
        .await;
        assert!(!blocked(&db, &org, DEADLINE).await);
        let other = customer(&db, "ordinary").await;
        db.sync_stripe_subscription(subscription(
            &other,
            "ordinary",
            BillingSubscriptionStatus::PastDue,
            10,
        ))
        .await
        .unwrap();
        let client = db.connection().await.unwrap();
        assert!(
            trials_access::trial_access(&**client, &other, DEADLINE)
                .await
                .unwrap()
                .is_none()
        );
    })
    .await;
}

#[tokio::test]
async fn trial_creation_request_and_queued_or_expired_lease_require_access() {
    with_isolated_postgres(|db| async move {
        let org = redeemed_trial(&db).await;
        assert!(matches!(
            db.request_agent_creation(creation(DEADLINE)).await,
            Err(CoreError::BillingRequired)
        ));
        assert_eq!(db.table_len("agent_creation_requests").await, 0);
        let created = db.request_agent_creation(creation(START)).await.unwrap();
        assert!(
            db.lease_agent_creation_request(creation_lease(DEADLINE))
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            db.lease_agent_creation_request(creation_lease(BEFORE))
                .await
                .unwrap()
                .unwrap()
                .request
                .id,
            created.request.id
        );
        assert!(
            db.lease_agent_creation_request(creation_lease(DEADLINE))
                .await
                .unwrap()
                .is_none()
        );
        db.exec(&format!(
            "UPDATE customer_billing_accounts SET subscription_status = 'active' WHERE customer_org_id = '{org}'"
        ))
        .await;
        let restored = db
            .lease_agent_creation_request(creation_lease(DEADLINE))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(restored.request.id, created.request.id);
        assert_eq!(restored.project.id, created.project.id);
    })
    .await;
}

#[tokio::test]
async fn trial_control_requests_block_upbound_but_preserve_stop_and_destroy() {
    with_isolated_postgres(|db| async move {
        let project = running_trial(&db).await;
        for kind in [
            RuntimeControlKind::Restart,
            RuntimeControlKind::RecoverKnownGoodChatRuntime,
            RuntimeControlKind::Upgrade,
        ] {
            let mut client = db.connection().await.unwrap();
            let tx = client.transaction().await.unwrap();
            assert!(matches!(
                postgres_enqueue_runtime_control_request_bound(
                    &*tx, &project, &project.owner_user_id, kind, None, DEADLINE, None
                )
                .await,
                Err(CoreError::BillingRequired)
            ));
        }
        for kind in [RuntimeControlKind::Stop, RuntimeControlKind::Destroy] {
            let mut client = db.connection().await.unwrap();
            let tx = client.transaction().await.unwrap();
            let request = postgres_enqueue_runtime_control_request_bound(
                &*tx, &project, &project.owner_user_id, kind, None, DEADLINE, None,
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            drop(client);
            let leased = db
                .lease_runtime_control_request(control_lease(DEADLINE))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(leased.request.id, request.id);
            assert_eq!(leased.request.kind, kind);
            db.exec(&format!(
                "UPDATE runtime_control_requests SET status = 'failed', lease_token = NULL, lease_expires_at = NULL WHERE id = '{}'",
                request.id
            ))
            .await;
        }
    })
    .await;
}

#[tokio::test]
async fn trial_control_lease_cannot_restart_queued_or_expired_work_after_deadline() {
    with_isolated_postgres(|db| async move {
        let project = running_trial(&db).await;
        let restart = db
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "gates@example.com".into(),
                workos_user_id: "gates".into(),
                project_id: project.id,
                now: Some(START.into()),
            })
            .await
            .unwrap();
        assert!(
            db.lease_runtime_control_request(control_lease(DEADLINE))
                .await
                .unwrap()
                .is_none()
        );
        let active = db
            .lease_runtime_control_request(control_lease(BEFORE))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(active.request.id, restart.id);
        assert!(
            db.lease_runtime_control_request(control_lease(DEADLINE))
                .await
                .unwrap()
                .is_none()
        );
    })
    .await;
}

async fn complete_stop(db: &TestDb, request: &RuntimeControlRequest, now: &str) {
    db.complete_runtime_control_request(CompleteRuntimeControlRequestInput {
        trial_archive: None,
        request_id: request.id.clone(),
        runner_id: "gates-runner".into(),
        lease_token: "gates-control-lease".into(),
        runtime_artifact_id: None,
        state_schema_version: None,
        runtime_capabilities: None,
        runtime_host: None,
        published_app_urls: None,
        retirement_snapshot: None,
        now: Some(now.into()),
    })
    .await
    .unwrap();
}

async fn owner_stop_supersedes_trial_resume(after_billing_stop: bool) {
    with_isolated_postgres(|db| async move {
        let project = running_trial(&db).await;
        db.reconcile_trial_org(&project.customer_org_id, DEADLINE)
            .await
            .unwrap();
        let billing_stop = db
            .lease_runtime_control_request(control_lease(DEADLINE))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(billing_stop.request.kind, RuntimeControlKind::Stop);
        if after_billing_stop {
            complete_stop(&db, &billing_stop.request, DEADLINE).await;
        }
        let now = if after_billing_stop { AFTER } else { DEADLINE };
        let owner_stop = db
            .request_runtime_stop(RequestRuntimeStopInput {
                verified_email: "gates@example.com".into(),
                workos_user_id: "gates".into(),
                project_id: project.id,
                now: Some(now.into()),
            })
            .await
            .unwrap();
        if after_billing_stop {
            assert_ne!(owner_stop.id, billing_stop.request.id);
            let leased = db
                .lease_runtime_control_request(control_lease(now))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(leased.request.id, owner_stop.id);
        } else {
            assert_eq!(owner_stop.id, billing_stop.request.id);
            assert_eq!(owner_stop.lease_token, billing_stop.request.lease_token);
        }
        let intent = db
            .query_json("SELECT to_jsonb(s) FROM trial_runtime_suspensions s", &[])
            .await;
        assert_eq!(intent.len(), 1);
        assert_eq!(intent[0]["resume_allowed"], false);
        assert_eq!(intent[0]["stop_request_id"], owner_stop.id);
        let mut paid = subscription(
            &project.customer_org_id,
            "gates",
            BillingSubscriptionStatus::Active,
            20,
        );
        paid.now = Some(now.into());
        db.sync_trial_stripe_subscription(paid, Some("gates-checkout"))
            .await
            .unwrap();
        complete_stop(&db, &owner_stop, now).await;
        db.reconcile_trial_org(&project.customer_org_id, AFTER)
            .await
            .unwrap();
        assert!(
            db.lease_runtime_control_request(control_lease(AFTER))
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            db.row("agent_runtimes", &owner_stop.agent_runtime_id)
                .await
                .unwrap()["host_facts"]["runtime_status"],
            "offline"
        );
        assert!(
            db.all("runtime_control_requests")
                .await
                .iter()
                .all(|request| request["kind"] == "stop")
        );
    })
    .await;
}

#[tokio::test]
async fn trial_owner_stop_of_in_flight_billing_stop_prevents_payment_resume() {
    owner_stop_supersedes_trial_resume(false).await;
}

#[tokio::test]
async fn trial_owner_stop_after_billing_stop_prevents_payment_resume() {
    owner_stop_supersedes_trial_resume(true).await;
}

#[tokio::test]
async fn trial_admin_stop_acknowledgement_and_destroy_clear_resume_ownership() {
    with_isolated_postgres(|db| async move {
        let project = running_trial(&db).await;
        db.reconcile_trial_org(&project.customer_org_id, DEADLINE)
            .await
            .unwrap();
        let billing_stop = db
            .lease_runtime_control_request(control_lease(DEADLINE))
            .await
            .unwrap()
            .unwrap();
        for kind in [RuntimeControlKind::Stop, RuntimeControlKind::Destroy] {
            let mut client = db.connection().await.unwrap();
            let tx = client.transaction().await.unwrap();
            let requested = postgres_admin_request_runtime_control_bound(
                &*tx,
                AdminRuntimeControlInput {
                    admin_verified_email: "gates-admin@example.com".into(),
                    admin_workos_user_id: "gates-admin".into(),
                    project_id: project.id.clone(),
                    now: Some(if kind == RuntimeControlKind::Stop {
                        DEADLINE.into()
                    } else {
                        AFTER.into()
                    }),
                },
                kind,
                None,
                None,
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            drop(client);
            let intent = db
                .query_json("SELECT to_jsonb(s) FROM trial_runtime_suspensions s", &[])
                .await;
            assert_eq!(intent.len(), 1);
            assert_eq!(intent[0]["resume_allowed"], false);
            assert_eq!(intent[0]["stop_request_id"], requested.id);
            if kind == RuntimeControlKind::Stop {
                assert_eq!(requested.id, billing_stop.request.id);
                complete_stop(&db, &billing_stop.request, DEADLINE).await;
                // Restore only the billing ownership fixture to prove Destroy
                // explicitly disables it too; the Runtime remains stopped.
                db.exec(&format!(
                    "UPDATE trial_runtime_suspensions SET resume_allowed = TRUE WHERE agent_runtime_id = '{}'",
                    billing_stop.runtime.id,
                ))
                .await;
            }
        }
    })
    .await;
}
