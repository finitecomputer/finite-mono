use super::*;
use crate::test_support::{TestDb, with_isolated_postgres};
use crate::{BillingSubscriptionStatus, RuntimeArtifactKind, RuntimeCapabilitiesV1};

fn capabilities() -> RuntimeCapabilitiesEnvelope {
    RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
        restart: true,
        recover_known_good_chat: false,
        runtime_upgrade: true,
        stop: true,
        runtime_retirement: false,
    })
}

fn subscription(
    org: &str,
    status: BillingSubscriptionStatus,
    event: i64,
) -> SyncStripeSubscriptionInput {
    SyncStripeSubscriptionInput {
        customer_org_id: Some(org.into()),
        stripe_customer_id: "cus_trial".into(),
        stripe_subscription_id: "sub_trial".into(),
        stripe_price_id: Some("price_standard".into()),
        expected_stripe_price_id: Some("price_standard".into()),
        subscription_status: status,
        current_period_end: Some("2099-10-06T12:00:00Z".into()),
        cancel_at_period_end: false,
        stripe_event_id: Some(format!("evt_{event}")),
        stripe_event_created: Some(event),
        now: None,
    }
}

async fn setup(store: &TestDb) -> (String, String, String, String) {
    let run = "trial-lifecycle";
    let email = format!("{run}@finite.vip");
    let workos = format!("workos_{run}");
    let host = "rchost";
    let machine = "rc-agent-001";

    store
        .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
            id: "artifact-rc-v1".to_string(),
            kind: RuntimeArtifactKind::OciImage,
            reference: format!(
                "ghcr.io/finitecomputer/finite-agent-runtime:rc-v1@sha256:{}",
                "3".repeat(64)
            ),
            version_label: "rc-v1".to_string(),
            source_git_sha: None,
            finitec_version: None,
            hermes_source_ref: None,
            finite_platform_plugin_ref: None,
            state_schema_version: "state-v1".to_string(),
            base_image: None,
            canary_runtime_id: None,
            recover_known_good_chat: false,
            promoted: true,
            now: None,
        })
        .await
        .unwrap();
    let org = store
        .link_stripe_customer(LinkStripeCustomerInput {
            verified_email: email.clone(),
            workos_user_id: workos.clone(),
            stripe_customer_id: "cus_trial".into(),
            now: None,
        })
        .await
        .unwrap()
        .customer_org_id;
    let campaign = store
        .create_trial_campaign(
            crate::trials::CreateTrialCampaign {
                name: "Trial lifecycle".into(),
                seat_limit: 1,
                trial_days: 7,
            },
            "admin",
        )
        .await
        .unwrap();
    store
        .reserve_trial(
            &workos,
            crate::trials::ReserveTrial {
                code: campaign.code,
                customer_org_id: org.clone(),
                stripe_customer_id: "cus_trial".into(),
                stripe_session_id: "cs_trial".into(),
                attempt_id: "trial".into(),
                trial_days: 7,
                checkout_expires_at: time::OffsetDateTime::now_utc().unix_timestamp() + 3600,
            },
        )
        .await
        .unwrap();
    store
        .sync_trial_stripe_subscription(
            subscription(&org, crate::BillingSubscriptionStatus::Trialing, 10),
            Some("trial"),
        )
        .await
        .unwrap();
    store
        .request_agent_creation_configured(
            RequestAgentCreationInput {
                verified_email: email.clone(),
                workos_user_id: workos.clone(),
                display_name: "RC Agent".to_string(),
                launch_code: String::new(),
                idempotency_key: format!("{run}-submit"),
                now: None,
            },
            AgentCreationConfiguration {
                placement: Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
                requested_hosting_tier: None,
                profile_picture_url: None,
                owner_chat_account_id: None,
            },
        )
        .await
        .unwrap();
    let lease = store
        .lease_agent_creation_request(LeaseAgentCreationRequestInput {
            runner_id: format!("runner-{run}"),
            source_host_id: None,
            lease_token: format!("lease-{run}"),
            lease_seconds: Some(300),
            runner_capacity: None,
            now: None,
        })
        .await
        .unwrap()
        .expect("request should lease");
    // A Finite Private key bound to the runtime, to prove destroy revokes it.
    let provisioned = store
        .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
            request_id: lease.request.id.clone(),
            runner_id: format!("runner-{run}"),
            lease_token: format!("lease-{run}"),
            source_host_id: Some(host.to_string()),
            source_machine_id: Some(machine.to_string()),
            now: None,
        })
        .await
        .unwrap();
    let completed = store
        .complete_agent_creation_request(CompleteAgentCreationRequestInput {
            request_id: lease.request.id.clone(),
            runner_id: format!("runner-{run}"),
            lease_token: format!("lease-{run}"),
            source_host_id: host.to_string(),
            source_machine_id: machine.to_string(),
            runtime_artifact_id: Some("artifact-rc-v1".to_string()),
            state_schema_version: Some("state-v1".to_string()),
            provider_runtime_handle: None,
            contact_endpoint: Some("http://127.0.0.1:41001/contact".to_string()),
            runtime_capabilities: Some(capabilities()),
            display_name: Some("RC Agent".to_string()),
            hostname: None,
            runtime_host: Some(host.to_string()),
            runtime_status: Some(RuntimeSummaryStatus::Online),
            active_inference_profile: None,
            hermes_available: Some(true),
            published_app_urls: vec!["http://127.0.0.1:41001/contact".to_string()],
            agent_npub: None,
            now: None,
        })
        .await
        .unwrap();
    let project_id = completed.project.id.clone();
    let runtime_id = completed.request.agent_runtime_id.clone().unwrap();
    (org, project_id, runtime_id, provisioned.api_key.id)
}

async fn complete_next(store: &TestDb, kind: RuntimeControlKind) -> RuntimeControlRequest {
    let lease = store
        .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
            runner_id: "trial-runner".into(),
            lease_token: "trial-control".into(),
            lease_seconds: Some(60),
            source_host_id: Some("rchost".into()),
            runner_capacity: Some(crate::RunnerLeaseCapacity {
                runner_classes: vec![crate::RunnerClass::Kata],
                runtime_capabilities: Some(capabilities()),
                ..Default::default()
            }),
            now: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lease.request.kind, kind);
    store
        .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
            request_id: lease.request.id.clone(),
            runner_id: "trial-runner".into(),
            lease_token: "trial-control".into(),
            runtime_artifact_id: None,
            state_schema_version: None,
            runtime_capabilities: None,
            runtime_host: None,
            published_app_urls: None,
            retirement_snapshot: None,
            now: None,
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn trial_nonpayment_stops_once_and_payment_restores_same_agent() {
    with_isolated_postgres(|store| async move {
        let (org, project, runtime, key) = setup(&store).await;
        let runtime_before = store.row("agent_runtimes", &runtime).await.unwrap();
        let key_before = store.row("finite_private_api_keys", &key).await.unwrap();
        let memberships = store.all("project_room_memberships").await;
        let fail = subscription(&org, BillingSubscriptionStatus::PastDue, 20);
        store.sync_stripe_subscription(fail.clone()).await.unwrap();
        store.sync_stripe_subscription(fail).await.unwrap();
        store.reconcile_trial_runtimes(None).await.unwrap();
        assert_eq!(store.all("runtime_control_requests").await.len(), 1);
        assert!(matches!(
            store
                .request_runtime_restart(RequestRuntimeRestartInput {
                    verified_email: "trial-lifecycle@finite.vip".into(),
                    workos_user_id: "workos_trial-lifecycle".into(),
                    project_id: project.clone(),
                    now: None,
                })
                .await,
            Err(CoreError::BillingRequired)
        ));
        // Recovery before Stop is acknowledged must wait, not launch in parallel.
        store
            .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::Active, 30))
            .await
            .unwrap();
        assert_eq!(store.all("runtime_control_requests").await.len(), 1);
        complete_next(&store, RuntimeControlKind::Stop).await;
        store.reconcile_trial_runtimes(None).await.unwrap();
        store.reconcile_trial_runtimes(None).await.unwrap();
        assert_eq!(store.all("runtime_control_requests").await.len(), 2);
        complete_next(&store, RuntimeControlKind::Restart).await;
        store.reconcile_trial_runtimes(None).await.unwrap();
        // Out-of-order and duplicate delivery cannot re-suspend restored compute.
        store
            .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::PastDue, 20))
            .await
            .unwrap();
        store
            .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::Active, 30))
            .await
            .unwrap();
        assert_eq!(store.all("runtime_control_requests").await.len(), 2);
        assert_eq!(store.all("agent_runtimes").await.len(), 1);
        assert_eq!(store.all("agent_creation_requests").await.len(), 1);
        let runtime_after = store.row("agent_runtimes", &runtime).await.unwrap();
        for field in [
            "id",
            "project_id",
            "source_machine_id",
            "provider_runtime_handle",
            "runtime_artifact_id",
        ] {
            assert_eq!(runtime_before[field], runtime_after[field]);
        }
        assert_eq!(
            store.row("finite_private_api_keys", &key).await.unwrap(),
            key_before
        );
        assert_eq!(store.all("project_room_memberships").await, memberships);
        assert_eq!(
            store
                .query_json("SELECT to_jsonb(s) FROM trial_runtime_suspensions s", &[])
                .await
                .len(),
            0
        );
        store.migrate().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn trial_clock_expiry_stops_without_webhook() {
    with_isolated_postgres(|store| async move {
        let (org, project, runtime, _) = setup(&store).await;
        store
            .reconcile_trial_runtimes(Some("2099-10-06T11:59:59Z"))
            .await
            .unwrap();
        assert!(store.all("runtime_control_requests").await.is_empty());
        store
            .reconcile_trial_runtimes(Some("2099-10-06T12:00:00Z"))
            .await
            .unwrap();
        assert_eq!(store.all("runtime_control_requests").await.len(), 1);
        assert_eq!(
            store.all("runtime_control_requests").await[0]["kind"],
            "stop"
        );
        assert_eq!(
            store.all("runtime_control_requests").await[0]["agent_runtime_id"],
            runtime
        );
        assert_eq!(
            store.all("runtime_control_requests").await[0]["project_id"],
            project
        );
        // Expiry does not rewrite billing or consume a second seat.
        let account = billing::select_customer_billing_account(
            &**store.connection().await.unwrap(),
            &org,
            false,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            account.subscription_status,
            Some(BillingSubscriptionStatus::Trialing)
        );
        assert_eq!(
            store.list_trial_campaigns().await.unwrap()[0].redeemed_seats,
            1
        );
    })
    .await;
}

#[tokio::test]
async fn trial_recovery_leaves_owner_stopped_runtime_stopped() {
    with_isolated_postgres(|store| async move {
        let (org, project, runtime, _) = setup(&store).await;
        store
            .request_runtime_stop(RequestRuntimeStopInput {
                verified_email: "trial-lifecycle@finite.vip".into(),
                workos_user_id: "workos_trial-lifecycle".into(),
                project_id: project,
                now: None,
            })
            .await
            .unwrap();
        complete_next(&store, RuntimeControlKind::Stop).await;
        store
            .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::PastDue, 20))
            .await
            .unwrap();
        store
            .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::Active, 30))
            .await
            .unwrap();
        store.reconcile_trial_runtimes(None).await.unwrap();
        assert_eq!(store.all("runtime_control_requests").await.len(), 1);
        assert_eq!(
            store.row("agent_runtimes", &runtime).await.unwrap()["host_facts"]["runtime_status"],
            "offline"
        );
        assert_eq!(
            store
                .query_json("SELECT to_jsonb(s) FROM trial_runtime_suspensions s", &[])
                .await[0]["resume_allowed"],
            false
        );
    })
    .await;
}

#[tokio::test]
async fn trial_expiry_replaces_queued_restart_and_serializes_duplicate_reconciliation() {
    with_isolated_postgres(|store| async move {
        let (org, project, _, _) = setup(&store).await;
        let restart = store
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "trial-lifecycle@finite.vip".into(),
                workos_user_id: "workos_trial-lifecycle".into(),
                project_id: project,
                now: None,
            })
            .await
            .unwrap();
        let deadline = "2099-10-06T12:00:00Z";
        let (a, b) = tokio::join!(
            store.reconcile_trial_org(&org, deadline),
            store.reconcile_trial_org(&org, deadline)
        );
        a.unwrap();
        b.unwrap();
        assert_eq!(
            store
                .row("runtime_control_requests", &restart.id)
                .await
                .unwrap()["status"],
            "failed"
        );
        let controls = store.all("runtime_control_requests").await;
        assert_eq!(controls.len(), 2);
        assert_eq!(controls.iter().filter(|r| r["kind"] == "stop").count(), 1);
    })
    .await;
}

#[tokio::test]
async fn trial_expiry_waits_for_live_control_but_fences_an_expired_lease() {
    with_isolated_postgres(|store| async move {
        let (org, project, _, _) = setup(&store).await;
        let restart = store
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "trial-lifecycle@finite.vip".into(),
                workos_user_id: "workos_trial-lifecycle".into(),
                project_id: project,
                now: None,
            })
            .await
            .unwrap();
        let lease = store
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "slow-runner".into(),
                lease_token: "slow-token".into(),
                lease_seconds: Some(300),
                source_host_id: Some("rchost".into()),
                runner_capacity: Some(crate::RunnerLeaseCapacity {
                    runner_classes: vec![crate::RunnerClass::Kata],
                    runtime_capabilities: Some(capabilities()),
                    ..Default::default()
                }),
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        store
            .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::PastDue, 20))
            .await
            .unwrap();
        assert_eq!(store.all("runtime_control_requests").await.len(), 1);
        assert_eq!(
            store
                .row("runtime_control_requests", &restart.id)
                .await
                .unwrap()["status"],
            "launching"
        );
        store
            .reconcile_trial_org(&org, "2099-10-06T12:00:00Z")
            .await
            .unwrap();
        assert_eq!(store.all("runtime_control_requests").await.len(), 2);
        assert_eq!(
            store
                .row("runtime_control_requests", &restart.id)
                .await
                .unwrap()["status"],
            "failed"
        );
        assert!(
            store
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: lease.request.id,
                    runner_id: "slow-runner".into(),
                    lease_token: "slow-token".into(),
                    runtime_artifact_id: None,
                    state_schema_version: None,
                    runtime_capabilities: None,
                    runtime_host: None,
                    published_app_urls: None,
                    retirement_snapshot: None,
                    now: None,
                })
                .await
                .is_err()
        );
    })
    .await;
}

#[tokio::test]
async fn trial_billing_events_preserve_sponsorship() {
    with_isolated_postgres(|store| async move {
        let (org, _, _, _) = setup(&store).await;
        store
            .exec(&format!(
                "UPDATE customer_orgs SET billing_class = 'sponsored' WHERE id = '{org}'"
            ))
            .await;
        store
            .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::Active, 20))
            .await
            .unwrap();
        assert_eq!(
            store.row("customer_orgs", &org).await.unwrap()["billing_class"],
            "sponsored"
        );
        store
            .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::PastDue, 30))
            .await
            .unwrap();
        store
            .reconcile_trial_org(&org, "2099-10-06T12:00:00Z")
            .await
            .unwrap();
        assert!(store.all("runtime_control_requests").await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn trial_expiry_stops_uncertain_restart_even_when_core_still_records_offline() {
    with_isolated_postgres(|store| async move {
        let (org, project, runtime, _) = setup(&store).await;
        let control = RequestRuntimeStopInput {
            verified_email: "trial-lifecycle@finite.vip".into(),
            workos_user_id: "workos_trial-lifecycle".into(),
            project_id: project,
            now: None,
        };
        store.request_runtime_stop(control.clone()).await.unwrap();
        complete_next(&store, RuntimeControlKind::Stop).await;
        let restart = store.request_runtime_restart(control).await.unwrap();
        store
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "lost-runner".into(),
                lease_token: "lost-token".into(),
                lease_seconds: Some(60),
                source_host_id: Some("rchost".into()),
                runner_capacity: Some(crate::RunnerLeaseCapacity {
                    runner_classes: vec![crate::RunnerClass::Kata],
                    runtime_capabilities: Some(capabilities()),
                    ..Default::default()
                }),
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        // The provider has started compute but its completion never reached
        // Core. The offline latch is old evidence, not proof compute is off.
        assert_eq!(
            store.row("agent_runtimes", &runtime).await.unwrap()["host_facts"]["runtime_status"],
            "offline"
        );
        store
            .reconcile_trial_org(&org, "2099-10-06T12:00:00Z")
            .await
            .unwrap();
        assert_eq!(
            store
                .row("runtime_control_requests", &restart.id)
                .await
                .unwrap()["status"],
            "failed"
        );
        let controls = store.all("runtime_control_requests").await;
        assert_eq!(controls.len(), 3);
        assert_eq!(
            controls
                .iter()
                .filter(|r| r["kind"] == "stop" && r["status"] == "requested")
                .count(),
            1
        );
        assert_eq!(
            store
                .query_json("SELECT to_jsonb(s) FROM trial_runtime_suspensions s", &[])
                .await
                .len(),
            1
        );
    })
    .await;
}

async fn fail_next(store: &TestDb, kind: RuntimeControlKind) {
    let lease = store
        .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
            runner_id: "failing-runner".into(),
            lease_token: "failing-token".into(),
            lease_seconds: Some(60),
            source_host_id: Some("rchost".into()),
            runner_capacity: Some(crate::RunnerLeaseCapacity {
                runner_classes: vec![crate::RunnerClass::Kata],
                runtime_capabilities: Some(capabilities()),
                ..Default::default()
            }),
            now: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lease.request.kind, kind);
    store
        .fail_runtime_control_request(FailRuntimeControlRequestInput {
            request_id: lease.request.id,
            runner_id: "failing-runner".into(),
            lease_token: "failing-token".into(),
            failure_message: "Synthetic provider interruption".into(),
            failure_stage: Some(RuntimeLifecycleStage::Compute),
            now: None,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn trial_owner_stop_intent_survives_failed_stop_and_enforcement_retry() {
    with_isolated_postgres(|store| async move {
        let (org, project, _, _) = setup(&store).await;
        store
            .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::PastDue, 20))
            .await
            .unwrap();
        store
            .request_runtime_stop(RequestRuntimeStopInput {
                verified_email: "trial-lifecycle@finite.vip".into(),
                workos_user_id: "workos_trial-lifecycle".into(),
                project_id: project,
                now: None,
            })
            .await
            .unwrap();
        fail_next(&store, RuntimeControlKind::Stop).await;
        store.reconcile_trial_runtimes(None).await.unwrap();
        complete_next(&store, RuntimeControlKind::Stop).await;
        store
            .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::Active, 30))
            .await
            .unwrap();
        store.reconcile_trial_runtimes(None).await.unwrap();
        assert_eq!(
            store.all("runtime_control_requests").await.len(),
            2,
            "payment must not recreate automatic resume after an explicit owner Stop failed"
        );
    })
    .await;
}

#[tokio::test]
async fn trial_manual_recovery_satisfies_failed_automatic_resume_without_second_restart() {
    with_isolated_postgres(|store| async move {
        let (org, project, _, _) = setup(&store).await;
        store
            .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::PastDue, 20))
            .await
            .unwrap();
        complete_next(&store, RuntimeControlKind::Stop).await;
        store
            .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::Active, 30))
            .await
            .unwrap();
        fail_next(&store, RuntimeControlKind::Restart).await;
        store
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "trial-lifecycle@finite.vip".into(),
                workos_user_id: "workos_trial-lifecycle".into(),
                project_id: project,
                now: None,
            })
            .await
            .unwrap();
        complete_next(&store, RuntimeControlKind::Restart).await;
        store.reconcile_trial_runtimes(None).await.unwrap();
        assert_eq!(
            store.all("runtime_control_requests").await.len(),
            3,
            "successful explicit recovery must not be restarted a second time by billing"
        );
    })
    .await;
}

#[tokio::test]
async fn runtime_recovery_projection_respects_billing_and_stop_intent() {
    with_isolated_postgres(|store| async move {
        let (org, _, runtime, _) = setup(&store).await;
        let projects = store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap();
        assert!(projects[0].runtime_recovery.is_none());
        store.sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::PastDue, 20)).await.unwrap();
        assert!(store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap()[0].runtime_recovery.is_none());
        store.sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::Active, 30)).await.unwrap();
        assert_eq!(store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap()[0].runtime_recovery, Some(RuntimeRecoveryStatus::Restarting));
        complete_next(&store, RuntimeControlKind::Stop).await;
        store.reconcile_trial_runtimes(None).await.unwrap();
        assert_eq!(store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap()[0].runtime_recovery, Some(RuntimeRecoveryStatus::Restarting));
        let client = store.connection().await.unwrap();
        client.execute("UPDATE runtime_control_requests SET status = 'failed', failure_stage = 'launch', failure_message = 'private provider detail' WHERE agent_runtime_id = $1 AND kind = 'restart'", &[&runtime]).await.unwrap();
        drop(client);
        let projects = store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap();
        assert_eq!(projects[0].runtime_recovery, Some(RuntimeRecoveryStatus::Failed));
        assert!(projects[0].active_runtime_control.is_none());
        let public = crate::api::PublicVisibleProject::from(projects[0].clone());
        let json = serde_json::to_string(&public).unwrap();
        assert!(json.contains("\"runtime_recovery\":\"failed\""));
        assert!(!json.contains("private provider detail"));
        let client = store.connection().await.unwrap();
        client.execute("UPDATE trial_runtime_suspensions SET resume_allowed = FALSE WHERE agent_runtime_id = $1", &[&runtime]).await.unwrap();
        drop(client);
        assert!(store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap()[0].runtime_recovery.is_none());
    }).await;
}

#[tokio::test]
async fn runtime_recovery_waits_for_fresh_health_after_marker_clears() {
    with_isolated_postgres(|store| async move {
        let (org, _, runtime, _) = setup(&store).await;
        store.sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::PastDue, 20)).await.unwrap();
        complete_next(&store, RuntimeControlKind::Stop).await;
        store.sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::Active, 30)).await.unwrap();
        complete_next(&store, RuntimeControlKind::Restart).await;
        store.reconcile_trial_runtimes(None).await.unwrap();
        assert!(store.query_json("SELECT to_jsonb(s) FROM trial_runtime_suspensions s", &[]).await.is_empty());
        assert_eq!(store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap()[0].runtime_recovery, Some(RuntimeRecoveryStatus::RestartPending));
        let client = store.connection().await.unwrap();
        client.execute("UPDATE agent_runtimes SET health_ready = FALSE, health_reported_at = now(), health_observed_at = now(), health_report_interval_seconds = 30 WHERE id = $1", &[&runtime]).await.unwrap();
        drop(client);
        let not_ready = store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap().remove(0);
        assert_eq!(not_ready.runtime_recovery, Some(RuntimeRecoveryStatus::RestartPending));
        let public = crate::api::PublicVisibleProject::from(not_ready);
        assert_eq!(public.runtime.unwrap().runtime_status, RuntimeSummaryStatus::Offline);
        let client = store.connection().await.unwrap();
        client.execute("UPDATE agent_runtimes SET health_reported_at = now() - interval '5 minutes' WHERE id = $1", &[&runtime]).await.unwrap();
        drop(client);
        assert_eq!(store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap()[0].runtime_recovery, Some(RuntimeRecoveryStatus::RestartPending));
        let client = store.connection().await.unwrap();
        client.execute("UPDATE agent_runtimes SET health_ready = TRUE, health_reported_at = now(), health_observed_at = now(), health_report_interval_seconds = 30 WHERE id = $1", &[&runtime]).await.unwrap();
        drop(client);
        let project = store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap().remove(0);
        assert!(project.runtime_recovery.is_none());
        let public = crate::api::PublicVisibleProject::from(project);
        assert_eq!(public.runtime.unwrap().runtime_status, RuntimeSummaryStatus::Online);
    }).await;
}

#[tokio::test]
async fn ordinary_restart_has_generic_presentation_and_blocked_access_hides_recovery() {
    with_isolated_postgres(|store| async move {
        let (org, project, runtime, _) = setup(&store).await;
        store.request_runtime_restart(RequestRuntimeRestartInput {
            verified_email: "trial-lifecycle@finite.vip".into(),
            workos_user_id: "workos_trial-lifecycle".into(),
            project_id: project.clone(),
            now: None,
        }).await.unwrap();
        let visible = store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap();
        assert_eq!(visible[0].runtime_recovery, Some(RuntimeRecoveryStatus::RestartPending));
        let client = store.connection().await.unwrap();
        client.execute("UPDATE runtime_control_requests SET status = 'failed', failure_stage = 'launch' WHERE agent_runtime_id = $1 AND kind = 'restart'", &[&runtime]).await.unwrap();
        drop(client);
        let visible = store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap();
        assert_eq!(visible[0].runtime_recovery, Some(RuntimeRecoveryStatus::RestartFailed));
        // Access gating is independent of lifecycle reconciliation. A read must
        // not present payment recovery merely because an old restart exists.
        let client = store.connection().await.unwrap();
        client.execute("UPDATE customer_billing_accounts SET subscription_status = 'past_due' WHERE customer_org_id = $1", &[&org]).await.unwrap();
        drop(client);
        assert!(store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap()[0].runtime_recovery.is_none());
        let client = store.connection().await.unwrap();
        client.execute("UPDATE customer_billing_accounts SET subscription_status = 'active' WHERE customer_org_id = $1", &[&org]).await.unwrap();
        drop(client);
        store.request_runtime_restart(RequestRuntimeRestartInput {
            verified_email: "trial-lifecycle@finite.vip".into(),
            workos_user_id: "workos_trial-lifecycle".into(),
            project_id: project.clone(), now: None,
        }).await.unwrap();
        complete_next(&store, RuntimeControlKind::Restart).await;
        assert!(store.query_json("SELECT to_jsonb(s) FROM trial_runtime_suspensions s", &[]).await.is_empty());
        assert_eq!(store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap()[0].runtime_recovery, Some(RuntimeRecoveryStatus::RestartPending));
        store.request_runtime_stop(RequestRuntimeStopInput {
            verified_email: "trial-lifecycle@finite.vip".into(),
            workos_user_id: "workos_trial-lifecycle".into(),
            project_id: project, now: None,
        }).await.unwrap();
        assert!(store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap()[0].runtime_recovery.is_none());
        complete_next(&store, RuntimeControlKind::Stop).await;
        assert!(store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap()[0].runtime_recovery.is_none());

    }).await;
}

#[tokio::test]
async fn runtime_recovery_observation_ends_when_health_never_arrives() {
    with_isolated_postgres(|store| async move {
        let (org, _, _, _) = setup(&store).await;
        store.sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::PastDue, 20)).await.unwrap();
        complete_next(&store, RuntimeControlKind::Stop).await;
        store.sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::Active, 30)).await.unwrap();
        complete_next(&store, RuntimeControlKind::Restart).await;
        store.reconcile_trial_runtimes(None).await.unwrap();
        // No health report yet: the completed restart is still being observed.
        assert_eq!(store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap()[0].runtime_recovery, Some(RuntimeRecoveryStatus::RestartPending));
        // A runtime that never reports health must not show "waiting" forever.
        let client = store.connection().await.unwrap();
        client.execute("UPDATE runtime_control_requests SET completed_at = completed_at - interval '11 minutes' WHERE kind = 'restart'", &[]).await.unwrap();
        drop(client);
        assert!(store.visible_projects_for_workos_user("workos_trial-lifecycle").await.unwrap()[0].runtime_recovery.is_none());
    }).await;
}

#[tokio::test]
async fn trial_canceled_then_paid_checkout_restores_same_agent() {
    with_isolated_postgres(|store| async move {
        let (org, _, runtime, _) = setup(&store).await;
        let runtime_before = store.row("agent_runtimes", &runtime).await.unwrap();
        store
            .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::Canceled, 20))
            .await
            .unwrap();
        store.reconcile_trial_runtimes(None).await.unwrap();
        complete_next(&store, RuntimeControlKind::Stop).await;
        store.reconcile_trial_runtimes(None).await.unwrap();
        assert_eq!(store.all("runtime_control_requests").await.len(), 1);
        // Manage billing sends a canceled trial to paid Checkout, which creates
        // a replacement subscription without a trial attempt.
        let mut paid = subscription(&org, BillingSubscriptionStatus::Active, 30);
        paid.stripe_subscription_id = "sub_paid".into();
        paid.stripe_event_id = Some("evt_paid".into());
        let account = store.sync_stripe_subscription(paid).await.unwrap();
        assert_eq!(account.stripe_subscription_id.as_deref(), Some("sub_paid"));
        store.reconcile_trial_runtimes(None).await.unwrap();
        complete_next(&store, RuntimeControlKind::Restart).await;
        store.reconcile_trial_runtimes(None).await.unwrap();
        assert_eq!(store.all("runtime_control_requests").await.len(), 2);
        assert_eq!(store.all("agent_runtimes").await.len(), 1);
        assert_eq!(store.all("agent_creation_requests").await.len(), 1);
        let runtime_after = store.row("agent_runtimes", &runtime).await.unwrap();
        for field in [
            "id",
            "project_id",
            "source_machine_id",
            "runtime_artifact_id",
        ] {
            assert_eq!(runtime_before[field], runtime_after[field]);
        }
        let overview = store
            .billing_overview(LinkVerifiedUserInput {
                verified_email: "trial-lifecycle@finite.vip".into(),
                workos_user_id: "workos_trial-lifecycle".into(),
                now: None,
            })
            .await
            .unwrap();
        assert!(!overview.trial_access.unwrap().blocked);
        assert!(!overview.requires_billing);
    })
    .await;
}

#[tokio::test]
async fn trial_failed_stop_retries_back_off_instead_of_every_sweep() {
    with_isolated_postgres(|store| async move {
        let (org, _, _, _) = setup(&store).await;
        store
            .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::PastDue, 20))
            .await
            .unwrap();
        store.reconcile_trial_runtimes(None).await.unwrap();
        fail_next(&store, RuntimeControlKind::Stop).await;
        // The first retry is immediate.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        store.reconcile_trial_runtimes(None).await.unwrap();
        fail_next(&store, RuntimeControlKind::Stop).await;
        // A second failure waits instead of re-enqueueing on every sweep.
        for _ in 0..3 {
            store.reconcile_trial_runtimes(None).await.unwrap();
        }
        assert_eq!(store.all("runtime_control_requests").await.len(), 2);
        let later = (time::OffsetDateTime::now_utc() + time::Duration::seconds(20))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        store.reconcile_trial_runtimes(Some(&later)).await.unwrap();
        assert_eq!(store.all("runtime_control_requests").await.len(), 3);
    })
    .await;
}
