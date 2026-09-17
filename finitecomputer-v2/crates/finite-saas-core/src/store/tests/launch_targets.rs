use super::*;

#[tokio::test]
async fn postgres_targeted_launch_code_is_atomic_and_isolates_the_host() {
    with_isolated_postgres(|store| async move {
            store
                .link_verified_user(LinkVerifiedUserInput {
                    verified_email: "operator@finite.vip".into(),
                    workos_user_id: "operator".into(),
                    now: None,
                })
                .await
                .unwrap();
            let issued = store
                .issue_launch_code_batch(IssueLaunchCodeBatchInput {
                    name: "targeted canary".into(),
                    code_count: 1,
                    expires_in_hours: Some(1),
                    hosting_tier: Some(HostingTier::Standard),
                    created_by_workos_user_id: "operator".into(),
                    now: None,
                })
                .await
                .unwrap();
            let code = &issued.codes[0];
            // Fail closed on a mismatched exact batch or issuer, without creating policy.
            assert!(
                store
                    .target_launch_code_exact(
                        &code.id,
                        "wrong-batch",
                        "target-host",
                        "operator@finite.vip",
                        "operator"
                    )
                    .await
                    .is_err()
            );
            assert!(
                store
                    .target_launch_code_exact(
                        &code.id,
                        &issued.batch.id,
                        "target-host",
                        "operator@finite.vip",
                        "someone-else"
                    )
                    .await
                    .is_err()
            );
            store
                .target_launch_code_exact(
                    &code.id,
                    &issued.batch.id,
                    "target-host",
                    "operator@finite.vip",
                    "operator",
                )
                .await
                .unwrap();
            store
                .target_launch_code_exact(
                    &code.id,
                    &issued.batch.id,
                    "target-host",
                    "operator@finite.vip",
                    "operator",
                )
                .await
                .unwrap();
            assert!(
                store
                    .target_launch_code_exact(
                        &code.id,
                        &issued.batch.id,
                        "another-host",
                        "operator@finite.vip",
                        "operator"
                    )
                    .await
                    .is_err()
            );
            // A different code cannot compete for the same one-slot host.
            let competing = store.issue_launch_code_batch(IssueLaunchCodeBatchInput {
                name: "competing".into(), code_count: 1, expires_in_hours: Some(1),
                hosting_tier: Some(HostingTier::Standard), created_by_workos_user_id: "operator".into(), now: None,
            }).await.unwrap();
            assert!(store.target_launch_code_exact(&competing.codes[0].id, &competing.batch.id, "target-host", "operator@finite.vip", "operator").await.is_err());
            let multiple = store.issue_launch_code_batch(IssueLaunchCodeBatchInput {
                name: "multiple".into(), code_count: 2, expires_in_hours: Some(1),
                hosting_tier: Some(HostingTier::Standard), created_by_workos_user_id: "operator".into(), now: None,
            }).await.unwrap();
            assert!(store.target_launch_code_exact(&multiple.codes[0].id, &multiple.batch.id, "other-host", "operator@finite.vip", "operator").await.is_err());
            let client = store.connection().await.unwrap();
            let count: i64 = client.query_one("SELECT count(*) FROM launch_code_host_targets", &[]).await.unwrap().get(0);
            let audits: i64 = client.query_one("SELECT count(*) FROM finite_private_admin_audit_events WHERE action='launch_code.target_host'", &[]).await.unwrap().get(0);
            assert_eq!((count, audits), (1, 1));
            drop(client);
            let input = |suffix: &str, launch_code: &str| RequestAgentCreationInput {
                verified_email: format!("{suffix}@finite.vip"),
                workos_user_id: suffix.into(),
                display_name: suffix.into(),
                launch_code: launch_code.into(),
                idempotency_key: suffix.into(),
                now: None,
            };
            // The older ordinary request must not be picked by the reserved host.
            let ordinary_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
            let ordinary = store
                .request_agent_creation(input("ordinary", &ordinary_code))
                .await
                .unwrap();
            let targeted = store
                .request_agent_creation_configured(
                    input("canary", &code.code),
                    AgentCreationConfiguration {
                        owner_chat_account_id: Some("a".repeat(64)),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                targeted.request.target_source_host_id.as_deref(),
                Some("target-host")
            );
            assert_eq!(
                targeted.request.owner_chat_account_id.as_deref(),
                Some("a".repeat(64).as_str())
            );
            let retry = store
                .request_agent_creation(input("canary", &code.code))
                .await
                .unwrap();
            assert!(retry.reused);
            assert_eq!(retry.request.id, targeted.request.id);
            assert_eq!(
                retry.request.target_source_host_id,
                targeted.request.target_source_host_id
            );
            assert!(
                store
                    .target_launch_code_exact(
                        &code.id,
                        &issued.batch.id,
                        "target-host",
                        "operator@finite.vip",
                        "operator"
                    )
                    .await
                    .is_err()
            );
            let lease = |host: Option<&str>| LeaseAgentCreationRequestInput {
                runner_id: host.unwrap_or("no-host").into(),
                lease_token: "lease".into(),
                source_host_id: host.map(String::from),
                runner_capacity: None,
                lease_seconds: Some(300),
                now: None,
            };
            // Wrong/missing host cannot claim the still-pending target. The
            // old ordinary request remains available to an ordinary runner.
            let normal_lease = store
                .lease_agent_creation_request(lease(Some("ordinary-host")))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(normal_lease.request.id, ordinary.request.id);
            assert!(
                store
                    .lease_agent_creation_request(lease(Some("ordinary-host")))
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                store
                    .lease_agent_creation_request(lease(None))
                    .await
                    .unwrap()
                    .is_none()
            );
            // Existing, unmodified Runner request shape: no new wire fields required.
            let canary_lease = store
                .lease_agent_creation_request(lease(Some("target-host")))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(canary_lease.request.id, targeted.request.id);
            assert!(
                store
                    .lease_agent_creation_request(lease(Some("target-host")))
                    .await
                    .unwrap()
                    .is_none()
            );
            // A newly queued ordinary request still cannot enter the reserved host.
            let another_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
            store
                .request_agent_creation(input("ordinary-later", &another_code))
                .await
                .unwrap();
            assert!(
                store
                    .lease_agent_creation_request(lease(Some("target-host")))
                    .await
                    .unwrap()
                    .is_none()
            );
        })
        .await;
}

#[tokio::test]
async fn postgres_target_binding_serializes_with_redemption() {
    for binding_first in [true, false] {
        with_isolated_postgres(|store| async move {
                store.link_verified_user(LinkVerifiedUserInput {
                    verified_email: "race@finite.vip".into(), workos_user_id: "race".into(), now: None,
                }).await.unwrap();
                let issued = store.issue_launch_code_batch(IssueLaunchCodeBatchInput {
                    name: "race".into(), code_count: 1, expires_in_hours: Some(1),
                    hosting_tier: Some(HostingTier::Standard), created_by_workos_user_id: "race".into(), now: None,
                }).await.unwrap();
                let mut client = store.connection().await.unwrap();
                let tx = client.transaction().await.unwrap();
                tx.query_one("SELECT id FROM launch_codes WHERE id=$1 FOR UPDATE", &[&issued.codes[0].id]).await.unwrap();
                let bind_store = store.clone();
                let code_id = issued.codes[0].id.clone();
                let batch_id = issued.batch.id.clone();
                let bind = async move { bind_store.target_launch_code_exact(&code_id, &batch_id, "race-host", "race@finite.vip", "race").await };
                let create_store = store.clone();
                let plaintext = issued.codes[0].code.clone();
                let create = async move { create_store.request_agent_creation(RequestAgentCreationInput {
                    verified_email: "race-user@finite.vip".into(), workos_user_id: "race-user".into(),
                    display_name: "race agent".into(), launch_code: plaintext, idempotency_key: "race".into(), now: None,
                }).await };
                let (binding, creation) = if binding_first {
                    let binding = tokio::spawn(bind);
                    wait_for_targeting_lock(&tx, "%WHERE code.id = $1 AND code.batch_id = $2%").await;
                    let creation = tokio::spawn(create);
                    wait_for_targeting_lock(&tx, "%WHERE code.code_hash = $1%").await;
                    (binding, creation)
                } else {
                    let creation = tokio::spawn(create);
                    wait_for_targeting_lock(&tx, "%WHERE code.code_hash = $1%").await;
                    let binding = tokio::spawn(bind);
                    wait_for_targeting_lock(&tx, "%WHERE code.id = $1 AND code.batch_id = $2%").await;
                    (binding, creation)
                };
                tx.commit().await.unwrap();
                let bound = binding.await.unwrap();
                let created = creation.await.unwrap().unwrap();
                assert_eq!(bound.is_ok(), binding_first);
                assert_eq!(created.request.target_source_host_id.as_deref(), binding_first.then_some("race-host"));
                let count: i64 = client.query_one("SELECT count(*) FROM launch_code_host_targets", &[]).await.unwrap().get(0);
                let audits: i64 = client.query_one("SELECT count(*) FROM finite_private_admin_audit_events WHERE action='launch_code.target_host'", &[]).await.unwrap().get(0);
                assert_eq!(count, i64::from(binding_first));
                assert_eq!(audits, count);
            }).await;
    }
}
