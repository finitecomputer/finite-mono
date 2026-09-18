//! Existing domain and Postgres contract suites, grouped by responsibility.

use crate::test_support::{TestDb, with_isolated_postgres};
use crate::*;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;

mod billing_entitlements;
mod creation_lifecycle;
mod finite_private_admin;
mod finite_private_usage;
mod launch_admission;
mod provider_operations;
mod runner_admission;
mod runtime_artifacts;
mod runtime_canary;
mod runtime_controls;
mod runtime_health;
mod runtime_lifecycle;
mod runtime_relocation;
mod runtime_retirement;
mod runtime_upgrades;
mod schema_compatibility;

const NOW: &str = "2026-05-25T12:00:00Z";

const LATER: &str = "2026-05-25T13:00:00Z";

fn phala_runner_capacity(provider_inventory_count: u32) -> RunnerLeaseCapacity {
    RunnerLeaseCapacity {
        runner_classes: vec![RunnerClass::Phala],
        max_sandbox_count: Some(1),
        active_sandbox_count: Some(provider_inventory_count),
        ..RunnerLeaseCapacity::default()
    }
}

/// A second handle on the same database with different runtime
/// configuration.
///
/// The in-memory store took environment/secret references as call
/// arguments; the real store carries them on the handle. Tests that prove a
/// persisted runtime spec is reused rather than recomputed lease twice
/// through differently-configured handles.
fn with_runtime_config(
    db: &TestDb,
    environment: &BTreeMap<String, String>,
    secret_references: &[String],
) -> crate::store::CoreStore {
    db.store
        .clone()
        .with_runtime_environment(environment.clone())
        .unwrap()
        .with_runtime_secret_references(secret_references.to_vec())
        .unwrap()
}

/// One Stripe subscription sync for the `cus_order` fixture.
///
/// A plain closure cannot hold `.await`, so the repeated call is a helper.
async fn sync_order_subscription(
    db: &TestDb,
    org_id: &str,
    status: BillingSubscriptionStatus,
    event: &str,
    created: i64,
) -> CustomerBillingAccount {
    db.sync_stripe_subscription(SyncStripeSubscriptionInput {
        customer_org_id: Some(org_id.to_string()),
        stripe_customer_id: "cus_order".to_string(),
        stripe_subscription_id: "sub_order".to_string(),
        stripe_price_id: Some("price_standard".to_string()),
        expected_stripe_price_id: Some("price_standard".to_string()),
        subscription_status: status,
        current_period_end: Some("2026-08-01T12:00:00Z".to_string()),
        cancel_at_period_end: false,
        stripe_event_id: Some(event.to_string()),
        stripe_event_created: Some(created),
        now: Some(NOW.to_string()),
    })
    .await
    .unwrap()
}

async fn issue_test_launch_code(db: &TestDb) -> String {
    issue_launch_code(db, None).await
}

/// Issue one real launch code batch and return its single plaintext code.
///
/// Staging rows directly is no longer possible (and was never how a code
/// reaches production), so tests redeem codes the store actually issued.
async fn issue_launch_code(db: &TestDb, hosting_tier: Option<HostingTier>) -> String {
    db.issue_launch_code_batch(launch_codes::IssueLaunchCodeBatchInput {
        name: "Test batch".to_string(),
        code_count: 1,
        expires_in_hours: Some(launch_codes::MAX_LAUNCH_CODE_BATCH_HOURS),
        hosting_tier,
        created_by_workos_user_id: "workos-test-operator".to_string(),
        now: Some(NOW.to_string()),
    })
    .await
    .unwrap()
    .codes[0]
        .code
        .clone()
}

async fn issued_launch_code_id(db: &TestDb, plaintext: &str) -> String {
    let hash = launch_codes::hash_launch_code(plaintext).unwrap();
    db.query_json(
        "SELECT to_jsonb(t) FROM launch_codes t WHERE t.code_hash = $1",
        &[&hash],
    )
    .await
    .first()
    .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn promote_runtime_artifact(db: &TestDb) {
    promote_runtime_artifact_version(
        db,
        "artifact-v1",
        &format!(
            "ghcr.io/finitecomputer/agent-runtime:v1@sha256:{}",
            "a".repeat(64)
        ),
        "v1",
        "db-v1",
        NOW,
    )
    .await;
}

async fn promote_runtime_artifact_version(
    db: &TestDb,
    id: &str,
    reference: &str,
    version_label: &str,
    state_schema_version: &str,
    now: &str,
) {
    db.upsert_runtime_artifact(UpsertRuntimeArtifactInput {
        id: id.to_string(),
        kind: RuntimeArtifactKind::OciImage,
        reference: reference.to_string(),
        version_label: version_label.to_string(),
        source_git_sha: Some("git-sha".to_string()),
        finitec_version: Some("finitec-test".to_string()),
        hermes_source_ref: Some("hermes-ref".to_string()),
        finite_platform_plugin_ref: Some("plugin-ref".to_string()),
        state_schema_version: state_schema_version.to_string(),
        base_image: Some("python:3.11-trixie".to_string()),
        canary_runtime_id: None,
        recover_known_good_chat: false,
        promoted: true,
        now: Some(now.to_string()),
    })
    .await
    .unwrap();
}

async fn complete_self_serve_agent(
    db: &TestDb,
    email: &str,
    workos_user_id: &str,
    idempotency_key: &str,
    source_machine_id: &str,
    artifact_id: &str,
    now: &str,
) -> String {
    let launch_code = issue_test_launch_code(db).await;
    let requested = db
        .request_agent_creation(RequestAgentCreationInput {
            verified_email: email.to_string(),
            workos_user_id: workos_user_id.to_string(),
            display_name: source_machine_id.to_string(),
            launch_code: launch_code.clone(),
            idempotency_key: idempotency_key.to_string(),
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap();
    let lease = db
        .lease_agent_creation_request(LeaseAgentCreationRequestInput {
            runner_id: "runner-oslo-1".to_string(),
            source_host_id: None,
            lease_token: format!("lease-{source_machine_id}"),
            lease_seconds: Some(300),
            runner_capacity: None,
            now: Some(LATER.to_string()),
        })
        .await
        .unwrap()
        .unwrap();
    let completed = db
        .complete_agent_creation_request(CompleteAgentCreationRequestInput {
            request_id: requested.request.id,
            runner_id: "runner-oslo-1".to_string(),
            lease_token: format!("lease-{source_machine_id}"),
            source_host_id: "oslo-host-1".to_string(),
            source_machine_id: source_machine_id.to_string(),
            runtime_artifact_id: Some(artifact_id.to_string()),
            state_schema_version: None,
            provider_runtime_handle: None,
            contact_endpoint: None,
            runtime_capabilities: Some(kata_runtime_capabilities()),
            display_name: None,
            hostname: None,
            runtime_host: Some("oslo-host-1".to_string()),
            runtime_status: Some(RuntimeSummaryStatus::Online),
            active_inference_profile: Some("finite-private".to_string()),
            hermes_available: Some(true),
            published_app_urls: Vec::new(),
            agent_npub: None,
            now: Some(now.to_string()),
        })
        .await
        .unwrap();
    assert_eq!(lease.project.id, completed.project.id);
    completed.request.agent_runtime_id.unwrap()
}

fn kata_runtime_capabilities() -> RuntimeCapabilitiesEnvelope {
    RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
        restart: true,
        recover_known_good_chat: false,
        runtime_upgrade: true,
        stop: true,
        runtime_retirement: false,
    })
}

/// Stage the production anomaly the retired-offboard repair exists for: a
/// destroy control that stored its verified retirement receipt while the
/// offboarding transaction never ran, leaving the runtime link active with
/// no compute behind it. Returns (project_id, agent_runtime_id, destroy
/// request_id).
async fn stage_retired_offboard_anomaly(
    db: &TestDb,
    email: &str,
    workos_user_id: &str,
    idempotency_key: &str,
    source_machine_id: &str,
) -> (String, String, String) {
    promote_runtime_artifact(db).await;
    let runtime_id = complete_self_serve_agent(
        db,
        email,
        workos_user_id,
        idempotency_key,
        source_machine_id,
        "artifact-v1",
        "2026-07-21T20:00:00Z",
    )
    .await;
    let project_id = db
        .agent_runtime(&runtime_id)
        .await
        .unwrap()
        .project_id
        .clone();
    let retirement_capable =
        serde_json::to_string(&RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
            runtime_retirement: true,
            ..*kata_runtime_capabilities().v1()
        }))
        .unwrap();
    db.exec(&format!(
        "UPDATE agent_runtimes SET runtime_capabilities = '{retirement_capable}'::jsonb \
         WHERE id = '{runtime_id}'"
    ))
    .await;
    let destroy = db
        .request_runtime_destroy(RequestRuntimeDestroyInput {
            verified_email: email.to_string(),
            workos_user_id: workos_user_id.to_string(),
            project_id: project_id.clone(),
            now: Some("2026-07-21T20:01:00Z".to_string()),
        })
        .await
        .unwrap();
    let lease = db
        .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
            runner_id: "runner-oslo-1".to_string(),
            lease_token: format!("destroy-lease-{source_machine_id}"),
            lease_seconds: Some(60),
            source_host_id: Some("oslo-host-1".to_string()),
            runner_capacity: Some(RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Kata],
                runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                    RuntimeCapabilitiesV1 {
                        runtime_retirement: true,
                        ..*kata_runtime_capabilities().v1()
                    },
                )),
                ..RunnerLeaseCapacity::default()
            }),
            now: Some("2026-07-21T20:01:30Z".to_string()),
        })
        .await
        .unwrap()
        .unwrap();
    let spec = runtime_spec_v1(lease.runtime_spec.as_ref().unwrap());
    db.exec(&format!(
        "UPDATE runtime_control_requests \
         SET status = 'stopped', lease_token = NULL, lease_expires_at = NULL, \
             completed_at = CURRENT_TIMESTAMP \
         WHERE id = '{}'",
        destroy.id
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO runtime_retirement_snapshots (
           request_id, project_id, agent_runtime_id, durable_state_id,
           runtime_artifact_id, schema_version, backend, locator,
           zip_bytes, zip_sha256, manifest_sha256, created_at,
           verified_at, recovery_authority_id, retention_policy, stored_at
         ) VALUES (
           '{}', '{}', '{}', '{}',
           '{}', 'runtime_retirement_snapshot.v1', 'borg', '{}',
           8192, '{}', '{}', '2026-07-21T20:02:00Z',
           '2026-07-21T20:03:00Z', 'finite-assisted-test',
           'indefinite_until_purge', CURRENT_TIMESTAMP
         )",
        destroy.id,
        project_id,
        runtime_id,
        spec.durable_state_id,
        spec.runtime_artifact_id,
        runtime_retirement_archive_locator(&destroy.id),
        "a".repeat(64),
        "b".repeat(64),
    ))
    .await;
    (project_id, runtime_id, destroy.id)
}
