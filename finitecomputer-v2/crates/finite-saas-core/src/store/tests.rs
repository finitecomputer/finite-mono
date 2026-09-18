use crate::BillingSubscriptionStatus;

use super::*;
use crate::test_support::{TestDb, with_isolated_postgres};
use crate::{
    FinitePrivateApiKeyStatus, RUNTIME_RELOCATION_SCHEMA, RunnerClass, RunnerLeaseCapacity,
    RuntimeArtifactKind, RuntimeCapabilitiesEnvelope, RuntimeCapabilitiesV1,
};
use futures_util::FutureExt;
use std::collections::{BTreeMap, BTreeSet};

fn kata_runtime_capabilities() -> RuntimeCapabilitiesEnvelope {
    RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
        restart: true,
        recover_known_good_chat: false,
        runtime_upgrade: true,
        stop: true,
        runtime_retirement: false,
    })
}

async fn issue_test_launch_code(store: &CoreStore, _now: &str) -> String {
    store
        .issue_launch_code_batch(IssueLaunchCodeBatchInput {
            name: "Postgres test batch".to_string(),
            code_count: 1,
            expires_in_hours: Some(crate::launch_codes::MAX_LAUNCH_CODE_BATCH_HOURS),
            hosting_tier: None,
            created_by_workos_user_id: "workos-test-operator".to_string(),
            now: None,
        })
        .await
        .unwrap()
        .codes[0]
        .code
        .clone()
}

async fn issue_confidential_test_launch_code(store: &CoreStore) -> String {
    store
        .issue_launch_code_batch(IssueLaunchCodeBatchInput {
            name: "Postgres confidential test batch".to_string(),
            code_count: 1,
            expires_in_hours: Some(crate::launch_codes::MAX_LAUNCH_CODE_BATCH_HOURS),
            hosting_tier: Some(HostingTier::Confidential),
            created_by_workos_user_id: "workos-test-operator".to_string(),
            now: None,
        })
        .await
        .unwrap()
        .codes[0]
        .code
        .clone()
}

fn phala_runner_capacity(provider_inventory_count: u32) -> RunnerLeaseCapacity {
    RunnerLeaseCapacity {
        runner_classes: vec![RunnerClass::Phala],
        max_sandbox_count: Some(1),
        active_sandbox_count: Some(provider_inventory_count),
        ..RunnerLeaseCapacity::default()
    }
}

/// The schema as production knew it before the lifecycle state machine:
/// every migration except 0021. The remap test below builds this shape,
/// seeds the legacy vocabulary, and proves 0021 maps it exactly.
const PRE_LIFECYCLE_SCHEMA_SQL: &str = concat!(
    include_str!("../../migrations/0001_core.sql"),
    "\n",
    include_str!("../../migrations/0002_runtime_upgrade.sql"),
    "\n",
    include_str!("../../migrations/0003_launch_codes.sql"),
    "\n",
    include_str!("../../migrations/0004_membership_archive.sql"),
    "\n",
    include_str!("../../migrations/0005_phala_expand.sql"),
    "\n",
    include_str!("../../migrations/0006_runtime_capabilities_expand.sql"),
    "\n",
    include_str!("../../migrations/0007_provider_creation_operations.sql"),
    "\n",
    include_str!("../../migrations/0008_agent_creation_provisional_runtime.sql"),
    "\n",
    include_str!("../../migrations/0009_artifact_recovery_support.sql"),
    "\n",
    include_str!("../../migrations/0010_align_finite_private_generous.sql"),
    "\n",
    include_str!("../../migrations/0011_agent_email.sql"),
    "\n",
    include_str!("../../migrations/0012_runtime_retirement_snapshots.sql"),
    "\n",
    include_str!("../../migrations/0013_double_finite_private_default.sql"),
    "\n",
    include_str!("../../migrations/0014_finite_private_user_controls.sql"),
    "\n",
    include_str!("../../migrations/0015_runner_capacity_fences.sql"),
    "\n",
    include_str!("../../migrations/0016_runtime_cold_relocation.sql"),
    "\n",
    include_str!("../../migrations/0017_rfc3339_reads.sql"),
    "\n",
    include_str!("../../migrations/0018_finite_private_5x_profile.sql"),
    "\n",
    include_str!("../../migrations/0019_brain_agent_departure_facts.sql")
);

/// A stageable retirement fixture: a launched, retirement-capable Runtime
/// with one provisioned runtime key and an enqueued exact destroy. Returns
/// the project, runtime, destroy request, and leased RuntimeSpec ids the
/// destroy completion must bind to.
async fn stage_retirement_in_flight(
    store: &TestDb,
    run: &str,
    host: &str,
) -> (String, String, crate::RuntimeControlRequest, String, String) {
    let owner_email = format!("{run}-owner@finite.vip");
    let admin_email = format!("{run}-admin@finite.vip");
    let machine_id = format!("{run}-agent-001");
    let launch_code = issue_test_launch_code(store, "2026-07-21T12:00:00Z").await;
    store
        .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
            id: format!("artifact-{run}-v1"),
            kind: RuntimeArtifactKind::OciImage,
            reference: format!(
                "ghcr.io/finitecomputer/finite-agent-runtime:{run}-v1@sha256:{}",
                "6".repeat(64)
            ),
            version_label: format!("{run}-v1"),
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
    store
        .request_agent_creation(RequestAgentCreationInput {
            verified_email: owner_email.clone(),
            workos_user_id: format!("workos_{run}_owner"),
            display_name: format!("{run} Agent"),
            launch_code,
            idempotency_key: format!("{run}-submit"),
            now: None,
        })
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
        .expect("creation request should lease");
    let completed = store
        .complete_agent_creation_request(CompleteAgentCreationRequestInput {
            request_id: lease.request.id.clone(),
            runner_id: format!("runner-{run}"),
            lease_token: format!("lease-{run}"),
            source_host_id: host.to_string(),
            source_machine_id: machine_id.clone(),
            runtime_artifact_id: Some(format!("artifact-{run}-v1")),
            state_schema_version: Some("state-v1".to_string()),
            provider_runtime_handle: None,
            contact_endpoint: Some("http://127.0.0.1:41004/contact".to_string()),
            runtime_capabilities: Some(kata_runtime_capabilities()),
            display_name: Some(format!("{run} Agent")),
            hostname: None,
            runtime_host: Some(host.to_string()),
            runtime_status: Some(RuntimeSummaryStatus::Online),
            active_inference_profile: Some("finite-private".to_string()),
            hermes_available: Some(true),
            published_app_urls: vec!["http://127.0.0.1:41004/contact".to_string()],
            agent_npub: None,
            now: None,
        })
        .await
        .unwrap();
    let runtime_id = completed.request.agent_runtime_id.clone().unwrap();
    let project_id = completed.project.id.clone();
    let retirement_capable =
        serde_json::to_string(&RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
            runtime_retirement: true,
            ..*kata_runtime_capabilities().v1()
        }))
        .unwrap();
    store
        .exec(&format!(
            "UPDATE agent_runtimes SET runtime_capabilities = '{retirement_capable}'::jsonb \
                 WHERE id = '{runtime_id}'"
        ))
        .await;
    let destroy = store
        .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
            admin_verified_email: admin_email.clone(),
            admin_workos_user_id: format!("workos_{run}_admin"),
            project_id: project_id.clone(),
            expected_agent_runtime_id: runtime_id.clone(),
            expected_source_host_id: host.to_string(),
            expected_source_machine_id: machine_id.clone(),
            now: Some("2026-07-21T12:01:00Z".to_string()),
        })
        .await
        .unwrap();
    let destroy_lease = store
        .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
            runner_id: format!("runner-{run}"),
            lease_token: format!("ctl-destroy-{run}"),
            lease_seconds: Some(600),
            source_host_id: Some(host.to_string()),
            runner_capacity: Some(crate::RunnerLeaseCapacity {
                runner_classes: vec![crate::RunnerClass::Kata],
                runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                    RuntimeCapabilitiesV1 {
                        runtime_retirement: true,
                        ..*kata_runtime_capabilities().v1()
                    },
                )),
                ..crate::RunnerLeaseCapacity::default()
            }),
            now: Some("2026-07-21T12:01:30Z".to_string()),
        })
        .await
        .unwrap()
        .expect("retirement should lease to a capable Kata runner");
    assert_eq!(destroy_lease.request.id, destroy.id);
    let destroy_spec = runtime_spec_v1(destroy_lease.runtime_spec.as_ref().unwrap());
    (
        project_id,
        runtime_id,
        destroy,
        destroy_spec.durable_state_id.clone(),
        destroy_spec.runtime_artifact_id.clone(),
    )
}

async fn offboarding_phase_of(store: &TestDb, runtime_id: &str) -> serde_json::Value {
    store
        .row("agent_runtimes", runtime_id)
        .await
        .expect("runtime row must read back")["offboarding_phase"]
        .clone()
}

mod admin_runtime;
mod billing_lifecycle;
mod canary_retry;
mod connection;
mod control_lifecycle;
mod creation_concurrency;
mod creation_lifecycle;
mod creation_placement;
mod creation_recovery;
mod health;
mod host_release;
mod launch_codes;
mod launch_targets;
mod lifecycle_migration;
mod offboarding;
mod private_usage;
mod provider_operations;
mod relocation_absence;
mod relocation_registration;
mod retirement;
mod upgrade_migration;

async fn retry_canary_fixture(
    db: &crate::test_support::TestDb,
) -> (crate::RetryTargetedLaunchCodeInput, String) {
    db.link_verified_user(LinkVerifiedUserInput {
        verified_email: "retry-operator@finite.vip".into(),
        workos_user_id: "retry-operator".into(),
        now: None,
    })
    .await
    .unwrap();
    let issue = |name: &str| IssueLaunchCodeBatchInput {
        name: name.into(),
        code_count: 1,
        expires_in_hours: Some(1),
        hosting_tier: Some(HostingTier::Standard),
        created_by_workos_user_id: "retry-operator".into(),
        now: None,
    };
    let old = db.issue_launch_code_batch(issue("original")).await.unwrap();
    db.target_launch_code_exact(
        &old.codes[0].id,
        &old.batch.id,
        "retry-target",
        "retry-operator@finite.vip",
        "retry-operator",
    )
    .await
    .unwrap();
    let created = db
        .request_agent_creation(RequestAgentCreationInput {
            verified_email: "retry-operator@finite.vip".into(),
            workos_user_id: "retry-operator".into(),
            display_name: "Misplaced canary".into(),
            launch_code: old.codes[0].code.clone(),
            idempotency_key: "original".into(),
            now: None,
        })
        .await
        .unwrap();
    // Reproduce the N-1 writer's durable failure, on synthetic state only:
    // the binding exists but the saved request has no host target.
    let client = db.connection().await.unwrap();
    client
        .execute(
            "UPDATE agent_creation_requests SET target_source_host_id=NULL WHERE id=$1",
            &[&created.request.id],
        )
        .await
        .unwrap();
    drop(client);
    let leased = db
        .lease_agent_creation_request(LeaseAgentCreationRequestInput {
            runner_id: "wrong-runner".into(),
            source_host_id: Some("wrong-host".into()),
            lease_token: "old-lease".into(),
            lease_seconds: Some(300),
            runner_capacity: None,
            now: None,
        })
        .await
        .unwrap()
        .unwrap();
    let completed = db
        .complete_agent_creation_request(CompleteAgentCreationRequestInput {
            request_id: leased.request.id,
            runner_id: "wrong-runner".into(),
            lease_token: "old-lease".into(),
            source_host_id: "wrong-host".into(),
            source_machine_id: "old-canary-machine".into(),
            runtime_artifact_id: Some("artifact-postgres-fixture".into()),
            state_schema_version: Some("state-v1".into()),
            provider_runtime_handle: None,
            contact_endpoint: None,
            runtime_capabilities: Some(kata_runtime_capabilities()),
            display_name: Some("Misplaced canary".into()),
            hostname: None,
            runtime_host: Some("wrong-host".into()),
            runtime_status: Some(RuntimeSummaryStatus::Online),
            active_inference_profile: None,
            hermes_available: Some(true),
            published_app_urls: vec![],
            agent_npub: None,
            now: None,
        })
        .await
        .unwrap();
    let fresh = db.issue_launch_code_batch(issue("retry")).await.unwrap();
    (
        crate::RetryTargetedLaunchCodeInput {
            code_id: fresh.codes[0].id.clone(),
            expected_batch_id: fresh.batch.id,
            previous_code_id: old.codes[0].id.clone(),
            expected_previous_request_id: completed.request.id,
            expected_previous_project_id: completed.project.id,
            expected_previous_runtime_id: completed.request.agent_runtime_id.unwrap(),
            expected_previous_source_host_id: "wrong-host".into(),
            target_source_host_id: "retry-target".into(),
            operator_email: "retry-operator@finite.vip".into(),
            operator_workos_user_id: "retry-operator".into(),
        },
        fresh.codes[0].code.clone(),
    )
}

async fn wait_for_targeting_lock(tx: &Transaction<'_>, pattern: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                tx.query_one("SELECT pg_stat_clear_snapshot()", &[]).await.unwrap();
                let waiting: bool = tx.query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid() AND wait_event_type='Lock' AND query LIKE $1)",
                    &[&pattern],
                ).await.unwrap().get(0);
                if waiting { break; }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }).await.expect("production writer must be waiting on code lock");
}
