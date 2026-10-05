use super::*;
use crate::{RuntimeRetirementSnapshotReceipt, TrialArchiveSnapshot};

fn archive_capacity(full: bool) -> crate::RunnerLeaseCapacity {
    let RuntimeCapabilitiesEnvelope::V1(mut caps) = capabilities();
    caps.trial_archive = true;
    caps.runtime_retirement = true;
    crate::RunnerLeaseCapacity {
        runner_classes: vec![crate::RunnerClass::Kata],
        runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(caps)),
        supports_relocation_credentials: true,
        max_sandbox_count: Some(1),
        active_sandbox_count: Some(u32::from(full)),
        ..Default::default()
    }
}
async fn archive_control(store: &TestDb, kind: RuntimeControlKind) -> crate::RuntimeControlLease {
    let lease = store
        .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
            runner_id: "archive-runner".into(),
            lease_token: "archive-lease".into(),
            lease_seconds: Some(300),
            source_host_id: Some("rchost".into()),
            runner_capacity: Some(archive_capacity(false)),
            now: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lease.request.kind, kind);
    lease
}
fn completion(
    lease: &crate::RuntimeControlLease,
    snapshot: Option<TrialArchiveSnapshot>,
) -> CompleteRuntimeControlRequestInput {
    CompleteRuntimeControlRequestInput {
        request_id: lease.request.id.clone(),
        runner_id: "archive-runner".into(),
        lease_token: "archive-lease".into(),
        runtime_artifact_id: None,
        state_schema_version: None,
        runtime_capabilities: None,
        runtime_host: None,
        published_app_urls: None,
        retirement_snapshot: None,
        trial_archive: snapshot,
        now: None,
    }
}
async fn archived(
    store: &TestDb,
) -> (
    String,
    String,
    String,
    crate::RuntimeControlLease,
    TrialArchiveSnapshot,
) {
    let (org, project, runtime, _) = setup(store).await;
    store
        .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::PastDue, 20))
        .await
        .unwrap();
    complete_next(store, RuntimeControlKind::Stop).await;
    let mut conn = store.connection().await.unwrap();
    let tx = conn.transaction().await.unwrap();
    let caps = serde_json::to_value(archive_capacity(false).runtime_capabilities.unwrap()).unwrap();
    tx.execute("UPDATE agent_runtimes SET runtime_capabilities=$2,health_reporting_npub='npub1sameagent' WHERE id=$1", &[&runtime,&caps]).await.unwrap();
    let p = select_project(&*tx, &project).await.unwrap().unwrap();
    postgres_enqueue_runtime_control_request_bound(
        &*tx,
        &p,
        &p.owner_user_id,
        RuntimeControlKind::ArchiveTrial,
        None,
        &current_time_iso().unwrap(),
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let lease = archive_control(store, RuntimeControlKind::ArchiveTrial).await;
    let spec = runtime_spec_v1(lease.runtime_spec.as_ref().unwrap());
    let now = current_time_iso().unwrap();
    let snapshot = TrialArchiveSnapshot {
        receipt: RuntimeRetirementSnapshotReceipt {
            schema: crate::RUNTIME_RETIREMENT_SNAPSHOT_SCHEMA.into(),
            request_id: lease.request.id.clone(),
            project_id: project.clone(),
            agent_runtime_id: runtime.clone(),
            durable_state_id: spec.durable_state_id.clone(),
            runtime_artifact_id: spec.runtime_artifact_id.clone(),
            backend: crate::RUNTIME_RETIREMENT_BACKEND_BORG.into(),
            locator: crate::runtime_retirement_archive_locator(&lease.request.id),
            zip_bytes: 100,
            zip_sha256: "a".repeat(64),
            manifest_sha256: "b".repeat(64),
            created_at: now.clone(),
            verified_at: now,
            recovery_authority_id: "test-archive".into(),
            retention_policy: crate::RUNTIME_RETIREMENT_RETENTION_INDEFINITE.into(),
        },
        durable_state_manifest_sha256: "c".repeat(64),
        agent_principal: "npub1sameagent".into(),
    };
    (org, project, runtime, lease, snapshot)
}
async fn creation(store: &TestDb, host: &str, full: bool) -> Option<AgentCreationLease> {
    store
        .lease_agent_creation_request(LeaseAgentCreationRequestInput {
            runner_id: format!("runner-{host}"),
            source_host_id: Some(host.into()),
            lease_token: format!("lease-{host}"),
            lease_seconds: Some(300),
            runner_capacity: Some(archive_capacity(full)),
            now: None,
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn trial_archive_receipt_before_reclaim_and_exact_terminal_replay() {
    with_isolated_postgres(|store|async move {
        let (_,_,runtime,lease,snapshot)=archived(&store).await;
        let mut wrong=snapshot.clone();wrong.agent_principal="npub1wrong".into();
        assert!(store.complete_runtime_control_request(completion(&lease,Some(wrong))).await.is_err());
        assert!(store.query_json("SELECT to_jsonb(a) FROM trial_runtime_archives a",&[]).await.is_empty());
        let input=completion(&lease,Some(snapshot.clone()));
        store.complete_runtime_control_request(input.clone()).await.unwrap();
        store.complete_runtime_control_request(input.clone()).await.unwrap();
        let mut stale=input;stale.lease_token="other-worker".into();
        assert!(store.complete_runtime_control_request(stale).await.is_err());
        store.reconcile_trial_runtimes(None).await.unwrap();
        let reclaim=archive_control(&store,RuntimeControlKind::ReclaimTrial).await;
        assert_eq!(reclaim.trial_archive,Some(snapshot));
        let input=completion(&reclaim,None);
        store.complete_runtime_control_request(input.clone()).await.unwrap();
        store.complete_runtime_control_request(input).await.unwrap();
        assert!(store.row("agent_runtimes",&runtime).await.is_some());
        assert!(store.query_json("SELECT to_jsonb(l) FROM project_runtime_links l WHERE agent_runtime_id=$1 AND active",&[&runtime]).await.len()==1);
        assert!(store.all("runtime_retirement_snapshots").await.is_empty());
    }).await;
}

#[tokio::test]
async fn trial_archive_payment_before_reclaim_uses_existing_runtime() {
    with_isolated_postgres(|store|async move {
        let (org,_,_,lease,snapshot)=archived(&store).await;
        store.complete_runtime_control_request(completion(&lease,Some(snapshot))).await.unwrap();
        store.sync_stripe_subscription(subscription(&org,BillingSubscriptionStatus::Active,30)).await.unwrap();
        complete_next(&store,RuntimeControlKind::Restart).await;
        assert!(store.query_json("SELECT to_jsonb(a) FROM trial_runtime_archives a WHERE restored_at IS NOT NULL AND reclaim_request_id IS NULL",&[]).await.len()==1);
        assert!(store.query_json("SELECT to_jsonb(q) FROM agent_creation_requests q WHERE relocation_spec IS NOT NULL",&[]).await.is_empty());
    }).await;
}

#[tokio::test]
async fn trial_archive_restore_capacity_billing_fencing_retry_and_same_identity() {
    with_isolated_postgres(|store|async move {
        let (org,project,runtime,archive,snapshot)=archived(&store).await;
        let memberships=store.all("project_room_memberships").await;
        store.complete_runtime_control_request(completion(&archive,Some(snapshot))).await.unwrap();
        store.reconcile_trial_runtimes(None).await.unwrap();
        let reclaim=archive_control(&store,RuntimeControlKind::ReclaimTrial).await;
        // Payment during reclaim never permits a parallel source restart.
        store.sync_stripe_subscription(subscription(&org,BillingSubscriptionStatus::Active,30)).await.unwrap();
        assert!(creation(&store,"restore-host",false).await.is_none());
        store.complete_runtime_control_request(completion(&reclaim,None)).await.unwrap();
        store.reconcile_trial_runtimes(None).await.unwrap();
        assert!(creation(&store,"restore-host",true).await.is_none());
        let lease=creation(&store,"restore-host",false).await.unwrap();
        assert_eq!(lease.trial_restore_allowed,Some(true));
        assert_eq!(lease.request.agent_runtime_id.as_deref(),Some(runtime.as_str()));
        assert_eq!(lease.project.id,project);
        let renew=crate::RenewRuntimeControlRequestInput {request_id:lease.request.id.clone(),runner_id:"runner-restore-host".into(),lease_token:"lease-restore-host".into(),lease_seconds:Some(300),now:None};
        store.renew_trial_restore(renew.clone()).await.unwrap();
        store.sync_stripe_subscription(subscription(&org,BillingSubscriptionStatus::PastDue,40)).await.unwrap();
        assert!(matches!(store.renew_trial_restore(renew).await,Err(CoreError::BillingRequired)));
        store.fail_agent_creation_request(FailAgentCreationRequestInput {request_id:lease.request.id.clone(),runner_id:"runner-restore-host".into(),lease_token:"lease-restore-host".into(),failure_message:"retry".into(),provisioned_finite_private_api_key_id:None,now:None}).await.unwrap();
        store.exec("UPDATE agent_creation_requests SET updated_at=clock_timestamp()-interval '31 seconds' WHERE status='requested'").await;
        assert!(creation(&store,"another-host",false).await.is_none());
        let paused=creation(&store,"restore-host",true).await.unwrap();
        assert_eq!(paused.trial_restore_allowed,Some(false));
        assert_eq!(paused.request.id,lease.request.id);
        store.sync_stripe_subscription(subscription(&org,BillingSubscriptionStatus::Active,50)).await.unwrap();
        let completed=store.complete_agent_creation_request(CompleteAgentCreationRequestInput {request_id:lease.request.id.clone(),runner_id:"runner-restore-host".into(),lease_token:"lease-restore-host".into(),source_host_id:"restore-host".into(),source_machine_id:"rc-agent-001".into(),runtime_artifact_id:Some("artifact-rc-v1".into()),state_schema_version:Some("state-v1".into()),provider_runtime_handle:None,contact_endpoint:Some("http://127.0.0.1:42000/contact".into()),runtime_capabilities:archive_capacity(false).runtime_capabilities,display_name:None,hostname:None,runtime_host:Some("http://127.0.0.1:42000".into()),runtime_status:Some(RuntimeSummaryStatus::Online),active_inference_profile:None,hermes_available:Some(true),published_app_urls:vec![],agent_npub:Some("npub1sameagent".into()),now:None}).await.unwrap();
        let stale_pause=crate::RenewRuntimeControlRequestInput {request_id:lease.request.id.clone(),runner_id:"runner-restore-host".into(),lease_token:"lease-restore-host".into(),lease_seconds:Some(300),now:None};
        assert!(store.renew_trial_restore_pause(stale_pause).await.is_err(), "a late worker cannot stop an already completed successor");
        assert_eq!(completed.request.agent_runtime_id.as_deref(),Some(runtime.as_str()));
        assert_eq!(store.all("project_room_memberships").await,memberships);
        assert!(store.query_json("SELECT to_jsonb(s) FROM trial_runtime_suspensions s WHERE agent_runtime_id=$1",&[&runtime]).await.is_empty());
        assert_eq!(store.row("agent_runtimes",&runtime).await.unwrap()["source_host_id"],"restore-host");
    }).await;
}

async fn queued_restore(store: &TestDb) -> String {
    let (org, _, _, archive, snapshot) = archived(store).await;
    store
        .complete_runtime_control_request(completion(&archive, Some(snapshot)))
        .await
        .unwrap();
    store.reconcile_trial_runtimes(None).await.unwrap();
    let reclaim = archive_control(store, RuntimeControlKind::ReclaimTrial).await;
    store
        .complete_runtime_control_request(completion(&reclaim, None))
        .await
        .unwrap();
    store
        .sync_stripe_subscription(subscription(&org, BillingSubscriptionStatus::Active, 30))
        .await
        .unwrap();
    store.reconcile_trial_runtimes(None).await.unwrap();
    store
        .query_json("SELECT to_jsonb(a) FROM trial_runtime_archives a", &[])
        .await[0]["restore_request_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn restore_key_input(lease: &AgentCreationLease) -> ProvisionFinitePrivateRuntimeKeyInput {
    ProvisionFinitePrivateRuntimeKeyInput {
        request_id: lease.request.id.clone(),
        runner_id: "runner-restore-host".into(),
        lease_token: "lease-restore-host".into(),
        source_host_id: Some("restore-host".into()),
        source_machine_id: Some("rc-agent-001".into()),
        trial_restore_key: Some(format!("fpk_live_{}", "a".repeat(64))),
        now: None,
    }
}
async fn retry_restore(store: &TestDb, id: &str, key: Option<String>, message: &str) {
    store
        .fail_agent_creation_request(FailAgentCreationRequestInput {
            request_id: id.into(),
            runner_id: "runner-restore-host".into(),
            lease_token: "lease-restore-host".into(),
            failure_message: message.into(),
            provisioned_finite_private_api_key_id: key,
            now: None,
        })
        .await
        .unwrap();
    store.exec("UPDATE agent_creation_requests SET updated_at=clock_timestamp()-interval '31 seconds' WHERE status='requested'").await;
}

#[tokio::test]
async fn trial_restore_cancellation_preserves_pending_and_uncertain_recovery() {
    with_isolated_postgres(|store| async move {
        let id = queued_restore(&store).await;
        let cancel = crate::CancelAgentCreationRequestInput {
            request_id: id.clone(),
            now: None,
        };
        assert!(matches!(
            store.cancel_agent_creation_request(cancel.clone()).await,
            Err(CoreError::AgentCreationRequestNotCancellable)
        ));
        let lease = creation(&store, "restore-host", false).await.unwrap();
        assert_eq!(lease.request.id, id);
        assert!(matches!(
            store.cancel_agent_creation_request(cancel.clone()).await,
            Err(CoreError::AgentCreationRequestNotCancellable)
        ));
        retry_restore(&store, &id, None, "target booted; completion response lost").await;
        assert!(matches!(
            store.cancel_agent_creation_request(cancel).await,
            Err(CoreError::AgentCreationRequestNotCancellable)
        ));
        store.reconcile_trial_runtimes(None).await.unwrap();
        assert!(creation(&store, "another-host", false).await.is_none());
        let retry = creation(&store, "restore-host", true).await.unwrap();
        assert_eq!(retry.request.id, id);
        assert_eq!(retry.request.relocation, lease.request.relocation);
        assert_eq!(
            store
                .query_json(
                    "SELECT to_jsonb(a) FROM trial_runtime_archives a WHERE restored_at IS NULL",
                    &[]
                )
                .await
                .len(),
            1
        );
    })
    .await;
}

#[tokio::test]
async fn trial_restore_key_retries_are_bounded_before_and_after_uncertain_boot() {
    with_isolated_postgres(|store| async move {
        let id = queued_restore(&store).await;
        let mut lease = creation(&store, "restore-host", false).await.unwrap();
        let before = store.all("finite_private_api_keys").await.len();
        let input = restore_key_input(&lease);
        // Concurrent retries, including a lost issuance response, reuse exactly one key.
        let (first, replay) = tokio::join!(
            store.provision_finite_private_runtime_key(input.clone()),
            store.provision_finite_private_runtime_key(input.clone())
        );
        let first = first.unwrap();
        let replay = replay.unwrap();
        assert_eq!(first.api_key.id, replay.api_key.id);
        assert!(first.raw_api_key == replay.raw_api_key);
        for failure in [
            "archive readback failed",
            "no local capacity",
            "target booted; completion uncertain",
        ] {
            retry_restore(&store, &id, Some(first.api_key.id.clone()), failure).await;
            lease = creation(&store, "restore-host", false).await.unwrap();
            let next = store
                .provision_finite_private_runtime_key(restore_key_input(&lease))
                .await
                .unwrap();
            assert_eq!(next.api_key.id, first.api_key.id);
            assert!(next.raw_api_key == first.raw_api_key);
            assert_eq!(
                next.api_key.status,
                crate::FinitePrivateApiKeyStatus::Active
            );
            assert_eq!(store.all("finite_private_api_keys").await.len(), before + 1);
        }
        let mut mismatch = input.clone();
        mismatch.trial_restore_key = Some(format!("fpk_live_{}", "b".repeat(64)));
        assert!(
            store
                .provision_finite_private_runtime_key(mismatch)
                .await
                .is_err()
        );
        let mut missing = input.clone();
        missing.trial_restore_key = None;
        assert!(
            store
                .provision_finite_private_runtime_key(missing)
                .await
                .is_err()
        );
        let mut stale = input.clone();
        stale.lease_token = "superseded-worker".into();
        assert!(
            store
                .provision_finite_private_runtime_key(stale)
                .await
                .is_err()
        );
        let mut wrong_host = input.clone();
        wrong_host.source_host_id = Some("other-host".into());
        assert!(
            store
                .provision_finite_private_runtime_key(wrong_host)
                .await
                .is_err()
        );
        store
            .exec("UPDATE finite_private_grants SET status='revoked'")
            .await;
        assert!(matches!(
            store
                .provision_finite_private_runtime_key(input.clone())
                .await,
            Err(CoreError::FinitePrivateGrantNotActive)
        ));
        store
            .exec("UPDATE finite_private_grants SET status='active'")
            .await;
        store
            .revoke_finite_private_api_key(crate::RevokeFinitePrivateApiKeyInput {
                key_id: first.api_key.id.clone(),
                now: None,
            })
            .await
            .unwrap();
        assert!(
            store
                .provision_finite_private_runtime_key(input)
                .await
                .is_err()
        );
        assert_eq!(
            store
                .row("finite_private_api_keys", &first.api_key.id)
                .await
                .unwrap()["status"],
            "revoked"
        );
        assert_eq!(store.all("finite_private_api_keys").await.len(), before + 1);
    })
    .await;
}

#[tokio::test]
async fn trial_restore_proposal_cannot_take_over_an_existing_key() {
    with_isolated_postgres(|store| async move {
        queued_restore(&store).await;
        let lease = creation(&store, "restore-host", false).await.unwrap();
        let input = restore_key_input(&lease);
        let grant = store.all("finite_private_grants").await[0]["id"].as_str().unwrap().to_string();
        let existing = store.issue_finite_private_api_key(crate::IssueFinitePrivateApiKeyInput {
            grant_id: grant, raw_key: input.trial_restore_key.clone().unwrap(), project_id: None, agent_runtime_id: None, now: None,
        }).await.unwrap();
        store.revoke_finite_private_api_key(crate::RevokeFinitePrivateApiKeyInput {key_id:existing.id.clone(),now:None}).await.unwrap();
        let before = store.row("finite_private_api_keys", &existing.id).await.unwrap();
        assert!(store.provision_finite_private_runtime_key(input).await.is_err());
        assert_eq!(store.row("finite_private_api_keys", &existing.id).await.unwrap(), before);
        assert!(store.query_json("SELECT to_jsonb(a) FROM trial_runtime_archives a WHERE restore_private_key_id IS NOT NULL", &[]).await.is_empty());
    }).await;
}
