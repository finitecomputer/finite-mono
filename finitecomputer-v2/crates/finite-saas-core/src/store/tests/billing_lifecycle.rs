use super::*;

/// Centerpiece regression test: the STANDARD-billing (real paying) agent
/// creation path, end to end, against Postgres. This is the path that
/// shipped broken — `ensure_standard_agent_creation_entitlement_row` does
/// `INSERT ... ON CONFLICT (customer_org_id)`, which fails deterministically
/// unless the table carries a UNIQUE(customer_org_id) constraint. There was
/// no test on this path, which is the whole reason the bug reached prod.
///
/// It FAILS without the migration's UNIQUE(customer_org_id) constraint (the
/// create call errors with a 23P01/42P10-class DB error) and PASSES with it.
#[tokio::test]
async fn postgres_standard_billing_agent_creation_succeeds() {
    with_isolated_postgres(|store| async move {
        // The database is isolated per test, so fixed identifiers are safe.
        let run = "standard-billing";
        let email = format!("standard-billing-{run}@finite.vip");
        let workos_user_id = format!("workos_standard_billing_{run}");

        // A paid user: link the Stripe customer, then sync an ACTIVE standard
        // subscription. No launch code -> the standard-billing entitlement path.
        // Surrogate ids are minted at insert, so read the org id back from
        // the create call rather than deriving it from the email.
        let org_id = store
            .link_stripe_customer(LinkStripeCustomerInput {
                verified_email: email.clone(),
                workos_user_id: workos_user_id.clone(),
                stripe_customer_id: format!("cus_standard_{run}"),
                now: None,
            })
            .await
            .unwrap()
            .customer_org_id;
        store
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: format!("cus_standard_{run}"),
                stripe_subscription_id: format!("sub_standard_{run}"),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::Active,
                current_period_end: Some("2026-08-01T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some(format!("evt_standard_active_{run}")),
                stripe_event_created: None,
                now: None,
            })
            .await
            .unwrap();

        // Billing is recognized before any create attempt.
        let overview = store
            .billing_overview(LinkVerifiedUserInput {
                verified_email: email.clone(),
                workos_user_id: workos_user_id.clone(),
                now: None,
            })
            .await
            .unwrap();
        assert!(overview.can_create_agent);
        assert!(!overview.requires_billing);

        // The create that was broken: no launch code -> standard entitlement
        // upsert via ON CONFLICT (customer_org_id). This is the line under test.
        let created = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: email.clone(),
                workos_user_id: workos_user_id.clone(),
                display_name: "Standard Billing Agent".to_string(),
                launch_code: String::new(),
                idempotency_key: format!("standard-submit-{run}"),
                now: None,
            })
            .await
            .expect("standard-billing agent creation must succeed");
        assert!(!created.reused);
        assert_eq!(created.request.requested_launch_code, None);
        assert_eq!(created.request.customer_org_id, org_id);
        assert_eq!(
            created.request.status,
            AgentCreationRequestStatus::Requested
        );

        // Re-submitting the same idempotency key reuses the row (exercises the
        // ON CONFLICT upsert a second time, which is what originally exploded).
        let reused = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: email.clone(),
                workos_user_id: workos_user_id.clone(),
                display_name: "Standard Billing Agent".to_string(),
                launch_code: String::new(),
                idempotency_key: format!("standard-submit-{run}"),
                now: None,
            })
            .await
            .expect("idempotent re-submit must succeed");
        assert!(reused.reused);
        assert_eq!(reused.request.id, created.request.id);

        // The entitlement carries no launch code (it is the paid, standard one).
        let overview_after = store
            .billing_overview(LinkVerifiedUserInput {
                verified_email: email,
                workos_user_id,
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(
            overview_after
                .agent_creation_entitlement
                .as_ref()
                .and_then(|entitlement| entitlement.launch_code.as_deref()),
            None
        );
    })
    .await;
}

/// GOLDEN-PATH E2E (per-PR gate). Drives the real STANDARD-billing product
/// path end to end against real Postgres with a FAKE runner (no Docker /
/// Phala): link Stripe customer -> sync an ACTIVE standard subscription ->
/// request_agent_creation (no launch code) -> lease the request (the
/// runner's claim) -> provision the finite-private key -> register the
/// runtime -> complete. Then assert the runtime is visible/online and the
/// creation request is terminal (Running).
///
/// This is the hop-by-hop test that would have caught the 2026-07-04
/// incident: the standard-billing entitlement upsert, the lease queue, and
/// the runtime registration all execute against real SQL and constraints.
/// Phase 2 (surrogate IDs, ordering guard) extends this without rewriting:
/// the shape is a linear sequence of store calls with assertions between.
#[tokio::test]
async fn postgres_golden_path_standard_billing_create_lifecycle() {
    with_isolated_postgres(|store| async move {
        let email = "golden@finite.vip".to_string();
        let workos_user_id = "workos_golden".to_string();
        let stripe_customer_id = "cus_golden".to_string();
        let runner_id = "runner-golden-1".to_string();
        let lease_token = "lease-golden-1".to_string();
        let source_host_id = "golden-host".to_string();
        let source_machine_id = "golden-agent-001".to_string();

        // The runtime image the fake runner will register.
        store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: "artifact-golden-v1".to_string(),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/finite-agent-runtime:golden-v1@sha256:{}",
                    "4".repeat(64)
                ),
                version_label: "golden-v1".to_string(),
                source_git_sha: None,
                finitec_version: None,
                hermes_source_ref: None,
                finite_platform_plugin_ref: None,
                state_schema_version: "state-v1".to_string(),
                base_image: Some("python:3.11-trixie".to_string()),
                canary_runtime_id: None,
                recover_known_good_chat: false,
                promoted: true,
                now: None,
            })
            .await
            .unwrap();

        // 1. Link the Stripe customer and sync an ACTIVE standard sub. The
        // org id is a surrogate minted at insert, so read it back from the
        // create call instead of deriving it from the email.
        let org_id = store
            .link_stripe_customer(LinkStripeCustomerInput {
                verified_email: email.clone(),
                workos_user_id: workos_user_id.clone(),
                stripe_customer_id: stripe_customer_id.clone(),
                now: None,
            })
            .await
            .unwrap()
            .customer_org_id;
        store
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: stripe_customer_id.clone(),
                stripe_subscription_id: "sub_golden".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::Active,
                current_period_end: Some("2026-08-01T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_golden_active".to_string()),
                stripe_event_created: None,
                now: None,
            })
            .await
            .unwrap();

        // Billing recognizes the paid user before any create attempt.
        let overview = store
            .billing_overview(LinkVerifiedUserInput {
                verified_email: email.clone(),
                workos_user_id: workos_user_id.clone(),
                now: None,
            })
            .await
            .unwrap();
        assert!(overview.can_create_agent, "active standard sub can create");
        assert!(!overview.requires_billing);

        // 2. request_agent_creation with NO launch code (the paid path).
        let created = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: email.clone(),
                workos_user_id: workos_user_id.clone(),
                display_name: "Golden Agent".to_string(),
                launch_code: String::new(),
                idempotency_key: "golden-submit".to_string(),
                now: None,
            })
            .await
            .expect("standard-billing create must succeed");
        assert_eq!(
            created.request.status,
            AgentCreationRequestStatus::Requested
        );
        assert_eq!(created.request.customer_org_id, org_id);

        // 3. The runner leases the pending creation request.
        let lease = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: runner_id.clone(),
                source_host_id: None,
                lease_token: lease_token.clone(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: None,
            })
            .await
            .unwrap()
            .expect("the pending request must be leasable");
        assert_eq!(lease.request.id, created.request.id);
        assert_eq!(lease.request.status, AgentCreationRequestStatus::Launching);

        // The project is visible but has no runtime yet.
        let visible_before = store
            .visible_projects_for_workos_user(&workos_user_id)
            .await
            .unwrap();
        assert_eq!(visible_before.len(), 1);
        assert!(visible_before[0].runtime.is_none());

        // 4. Provision the finite-private key + register the runtime.
        store
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id: lease.request.id.clone(),
                runner_id: runner_id.clone(),
                lease_token: lease_token.clone(),
                source_host_id: Some(source_host_id.clone()),
                source_machine_id: Some(source_machine_id.clone()),
                now: None,
            })
            .await
            .unwrap();
        store
            .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                request_id: lease.request.id.clone(),
                runner_id: runner_id.clone(),
                lease_token: lease_token.clone(),
                source_host_id: source_host_id.clone(),
                source_machine_id: source_machine_id.clone(),
                runtime_artifact_id: Some("artifact-golden-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: None,
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Golden Agent".to_string()),
                hostname: None,
                runtime_host: Some(source_host_id.clone()),
                runtime_status: Some(RuntimeSummaryStatus::Unknown),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                now: None,
            })
            .await
            .unwrap();

        // 5. Complete the creation.
        let completed = store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: lease.request.id.clone(),
                runner_id: runner_id.clone(),
                lease_token: lease_token.clone(),
                source_host_id: source_host_id.clone(),
                source_machine_id: source_machine_id.clone(),
                runtime_artifact_id: Some("artifact-golden-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: None,
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Golden Agent".to_string()),
                hostname: None,
                runtime_host: Some(source_host_id.clone()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                agent_npub: None,
                now: None,
            })
            .await
            .unwrap();

        // The request is terminal (Running) ...
        assert_eq!(
            completed.request.status,
            AgentCreationRequestStatus::Running,
            "completed creation request must be terminal"
        );
        let requests = store
            .agent_creation_requests_for_workos_user(&workos_user_id)
            .await
            .unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].status, AgentCreationRequestStatus::Running);

        // ... and the runtime is visible and online.
        let visible_after = store
            .visible_projects_for_workos_user(&workos_user_id)
            .await
            .unwrap();
        assert_eq!(visible_after.len(), 1);
        let runtime = visible_after[0]
            .runtime
            .as_ref()
            .expect("completed project must expose a runtime");
        assert_eq!(runtime.source_machine_id, source_machine_id);

        // A second lease call finds nothing else pending: the queue drained.
        let empty = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: runner_id.clone(),
                source_host_id: None,
                lease_token: "lease-golden-2".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: None,
            })
            .await
            .unwrap();
        assert!(empty.is_none(), "no further pending requests to lease");
    })
    .await;
}

/// SURROGATE-ID REGRESSION (Phase 2a): wipe an account, then re-signup with
/// the SAME email. Primary keys are now opaque surrogates minted at insert
/// (`user_id`/`org_id`/`request_id` are random, resolved by natural key),
/// so a clean full wipe followed by re-signup yields entirely FRESH ids that
/// cannot collide with the previous account's orphaned rows. This is the
/// flipped version of the old deterministic-id baseline: the point of the
/// incident fix is that re-created identities do NOT reconstruct old keys.
#[tokio::test]
async fn postgres_wipe_then_recreate_same_email_gets_fresh_surrogate_ids() {
    with_isolated_postgres(|store| async move {
        let launch_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
        let email = "wipe-recreate@finite.vip".to_string();
        let workos_user_id = "workos_wipe_recreate".to_string();

        let first = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: email.clone(),
                workos_user_id: workos_user_id.clone(),
                display_name: "Wipe Recreate Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: "wipe-recreate-1".to_string(),
                now: None,
            })
            .await
            .unwrap();
        // Read the minted surrogate ids back from the store — they are not
        // derivable from the email any more.
        let first_user_id = first.request.owner_user_id.clone();
        let first_org_id = first.request.customer_org_id.clone();
        let first_request_id = first.request.id.clone();

        // Full wipe via raw SQL. `TRUNCATE ... CASCADE` on the account root
        // tables removes every FK-dependent row (projects, requests,
        // entitlements, chat identities, memberships, ...) in one clean
        // sweep — this is the "clean" wipe; the incident was the *partial*
        // version that left orphans behind the same deterministic ids.
        let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        raw.batch_execute("TRUNCATE TABLE users CASCADE")
            .await
            .expect("clean full wipe should not violate FKs");
        drop(raw);
        connection.abort();

        let replacement_launch_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;

        // Re-signup with the same email. A clean wipe means this succeeds,
        // and — because ids are now surrogate — mints a genuinely fresh
        // user/org/request that share NOTHING with the wiped account.
        let second = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: email.clone(),
                workos_user_id: workos_user_id.clone(),
                display_name: "Wipe Recreate Agent".to_string(),
                launch_code: replacement_launch_code,
                idempotency_key: "wipe-recreate-2".to_string(),
                now: None,
            })
            .await
            .expect("re-signup after a clean wipe must succeed");
        assert_ne!(
            second.request.owner_user_id, first_user_id,
            "surrogate ids: the re-created user must get a fresh id"
        );
        assert_ne!(
            second.request.customer_org_id, first_org_id,
            "surrogate ids: the re-created org must get a fresh id"
        );
        assert_ne!(
            second.request.id, first_request_id,
            "surrogate ids: the re-created request must get a fresh id"
        );
    })
    .await;
}

/// Phase 2b event-ordering guard (audit finding #5): out-of-order Stripe
/// webhooks for the SAME subscription. `sync_stripe_subscription` now compares
/// the incoming `event.created` against the last applied one and IGNORES a
/// stale event, so an `active` delivered AFTER a `canceled` can no longer
/// resurrect billing. This is the flipped former baseline.
#[tokio::test]
async fn postgres_out_of_order_webhook_is_ignored() {
    with_isolated_postgres(|store| async move {
        let email = "webhook-order@finite.vip".to_string();
        let workos_user_id = "workos_webhook_order".to_string();
        let stripe_customer_id = "cus_webhook_order".to_string();
        let stripe_subscription_id = "sub_webhook_order".to_string();

        let org_id = store
            .link_stripe_customer(LinkStripeCustomerInput {
                verified_email: email.clone(),
                workos_user_id: workos_user_id.clone(),
                stripe_customer_id: stripe_customer_id.clone(),
                now: None,
            })
            .await
            .unwrap()
            .customer_org_id;

        let sync = |status: BillingSubscriptionStatus, event: &str, created: i64| {
            store.sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: stripe_customer_id.clone(),
                stripe_subscription_id: stripe_subscription_id.clone(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: status,
                current_period_end: Some("2026-08-01T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some(event.to_string()),
                stripe_event_created: Some(created),
                now: None,
            })
        };

        // Real order: active (created t0), then canceled (created t1 > t0).
        sync(BillingSubscriptionStatus::Active, "evt_active", 1_000)
            .await
            .unwrap();
        let canceled = sync(BillingSubscriptionStatus::Canceled, "evt_canceled", 2_000)
            .await
            .unwrap();
        assert_eq!(
            canceled.subscription_status,
            Some(BillingSubscriptionStatus::Canceled)
        );

        // A STALE `active` event (created BEFORE the canceled event) arrives LAST.
        let stale = sync(BillingSubscriptionStatus::Active, "evt_active_stale", 1_500)
            .await
            .unwrap();

        // The guard drops the stale event; billing stays canceled.
        assert_eq!(
            stale.subscription_status,
            Some(BillingSubscriptionStatus::Canceled),
            "stale out-of-order webhook must be ignored; billing stays canceled"
        );
        assert_eq!(stale.last_stripe_event_id.as_deref(), Some("evt_canceled"));
        let overview = store
            .billing_overview(LinkVerifiedUserInput {
                verified_email: email.clone(),
                workos_user_id: workos_user_id.clone(),
                now: None,
            })
            .await
            .unwrap();
        assert!(
            !overview.can_create_agent,
            "canceled subscription must not re-grant create after a stale webhook"
        );
    })
    .await;
}

/// `billing_overview` is a READ: it must perform NO writes. We run it inside a
/// genuinely read-only transaction and additionally assert the billing row's
/// `updated_at` is byte-for-byte unchanged across the call.
#[tokio::test]
async fn postgres_billing_overview_performs_no_writes() {
    with_isolated_postgres(|store| async move {
            let email = "read-only@finite.vip".to_string();
            let workos_user_id = "workos_read_only".to_string();
            let stripe_customer_id = "cus_read_only".to_string();

            let org_id = store
                .link_stripe_customer(LinkStripeCustomerInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    stripe_customer_id: stripe_customer_id.clone(),
                    now: None,
                })
                .await
                .unwrap()
                .customer_org_id;
            store
                .sync_stripe_subscription(SyncStripeSubscriptionInput {
                    customer_org_id: Some(org_id.clone()),
                    stripe_customer_id: stripe_customer_id.clone(),
                    stripe_subscription_id: "sub_read_only".to_string(),
                    stripe_price_id: Some("price_standard".to_string()),
                    expected_stripe_price_id: Some("price_standard".to_string()),
                    subscription_status: BillingSubscriptionStatus::Active,
                    current_period_end: Some("2026-08-01T12:00:00Z".to_string()),
                    cancel_at_period_end: false,
                    stripe_event_id: Some("evt_read_only_active".to_string()),
                    stripe_event_created: Some(1_000),
                    now: None,
                })
                .await
                .unwrap();

            // Snapshot every row's updated_at (as text) across all billing-related
            // tables the overview touches.
            let (raw, raw_conn) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let raw_conn = tokio::spawn(async move {
                let _ = raw_conn.await;
            });
            async fn snapshot(raw: &tokio_postgres::Client) -> Vec<(String, String)> {
                let mut out: Vec<(String, String)> = Vec::new();
                for table in [
                    "customer_orgs",
                    "customer_billing_accounts",
                    "agent_creation_entitlements",
                    "users",
                ] {
                    let key = if table == "customer_billing_accounts" {
                        "customer_org_id"
                    } else {
                        "id"
                    };
                    for row in raw
                        .query(
                            &format!(
                                "SELECT {key}::text, core_rfc3339(updated_at) AS updated_at FROM {table} ORDER BY 1"
                            ),
                            &[],
                        )
                        .await
                        .unwrap()
                    {
                        out.push((format!("{table}:{}", row.get::<_, String>(0)), row.get(1)));
                    }
                }
                out
            }

            let before = snapshot(&raw).await;

            // Read-only op: if it tried to write, the READ ONLY transaction would
            // error; assert it succeeds AND leaves every updated_at unchanged.
            let overview = store
                .billing_overview(LinkVerifiedUserInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    now: None,
                })
                .await
                .unwrap();
            assert!(overview.can_create_agent);
            assert!(!overview.requires_billing);

            let after = snapshot(&raw).await;
            assert_eq!(
                before, after,
                "billing_overview must not mutate any row (a read that writes is banned)"
            );

            drop(raw);
            raw_conn.abort();
        })
        .await;
}
