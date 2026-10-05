mod cancellation;
mod failures;
mod lifecycle;
mod recovery;

use super::*;
use crate::store::runtime_credentials::ProvisionRuntimeCredential;
use crate::test_support::{TestDb, with_isolated_postgres};
use crate::{
    AdminRuntimeRelocateExactInput, AgentCreationConfiguration, CancelAgentCreationRequestInput,
    CompleteAgentCreationRequestInput, FailAgentCreationRequestInput, HostingTier,
    LeaseAgentCreationRequestInput, RegisterAgentCreationRuntimeInput, RequestAgentCreationInput,
    RunnerClass, RunnerLeaseCapacity, RuntimeCapabilitiesEnvelope, RuntimeCapabilitiesV1,
    RuntimeSummaryStatus,
};

struct PreparedRelocation {
    origin_request_id: String,
    request_id: String,
    project_id: String,
    runtime_id: String,
    source_host: String,
    target_host: String,
    source_machine: String,
    artifact_id: String,
    run: String,
    predecessor_secret: Option<String>,
}

fn relocation_capacity() -> RunnerLeaseCapacity {
    RunnerLeaseCapacity {
        runner_classes: vec![RunnerClass::Kata],
        runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
            runtime_upgrade: true,
            stop: true,
            ..Default::default()
        })),
        supports_relocation_credentials: true,
        ..Default::default()
    }
}

async fn artifact(db: &TestDb, id: &str, digest: char) {
    db.upsert_runtime_artifact(crate::UpsertRuntimeArtifactInput {
        id: id.into(),
        kind: crate::RuntimeArtifactKind::OciImage,
        reference: format!(
            "ghcr.io/finitecomputer/agent-runtime:{id}@sha256:{}",
            digest.to_string().repeat(64)
        ),
        version_label: id.into(),
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
}

async fn create_runtime(db: &TestDb, run: &str, enroll_initial: bool) -> PreparedRelocation {
    let artifact_id = format!("api-relocation-{run}-v1");
    artifact(db, &artifact_id, 'a').await;

    let owner_email = format!("{run}@finite.vip");
    let owner_workos = format!("api-relocation-{run}-owner");
    let created = db
        .request_agent_creation_configured(
            RequestAgentCreationInput {
                verified_email: owner_email.clone(),
                workos_user_id: owner_workos.clone(),
                display_name: format!("API relocation {run}"),
                launch_code: issue_test_launch_code(db).await,
                idempotency_key: format!("api-relocation-{run}-create"),
                now: None,
            },
            AgentCreationConfiguration {
                placement: Some(crate::RuntimePlacement::for_hosting_tier(
                    HostingTier::Standard,
                )),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let runner = "runner-oslo-1".to_string();
    let source_host = "oslo-host-1".to_string();
    let source_machine = format!("finite-kata-api-relocation-{run}");
    let source_machine_for_fixture = source_machine.clone();
    let artifact_id_for_fixture = artifact_id.clone();
    let creation_lease_token = format!("api-relocation-{run}-creation-lease");
    let creation = db
        .lease_agent_creation_request(LeaseAgentCreationRequestInput {
            runner_id: runner.clone(),
            source_host_id: Some(source_host.clone()),
            lease_token: creation_lease_token.clone(),
            lease_seconds: Some(300),
            runner_capacity: Some(relocation_capacity()),
            now: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(creation.request.id, created.request.id);

    let predecessor_secret = if enroll_initial {
        Some(
            db.provision_runtime_credential(ProvisionRuntimeCredential {
                creation_request_id: creation.request.id.clone(),
                runner_id: runner.clone(),
                lease_token: creation_lease_token.clone(),
                source_host_id: source_host.clone(),
                prepare_hosted_access: false,
            })
            .await
            .unwrap()
            .secret,
        )
    } else {
        None
    };

    let agent_npub = format!("npub1{}", "a".repeat(58));
    db.register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
        request_id: creation.request.id.clone(),
        runner_id: runner.clone(),
        lease_token: creation_lease_token.clone(),
        source_host_id: source_host.clone(),
        source_machine_id: source_machine.clone(),
        runtime_artifact_id: Some(artifact_id.clone()),
        state_schema_version: Some("state-v1".to_string()),
        provider_runtime_handle: None,
        contact_endpoint: Some("http://127.0.0.1:4200/contact".to_string()),
        runtime_capabilities: relocation_capacity().runtime_capabilities,
        display_name: None,
        hostname: None,
        runtime_host: Some(source_host.clone()),
        runtime_status: Some(RuntimeSummaryStatus::Online),
        active_inference_profile: None,
        hermes_available: Some(true),
        published_app_urls: vec![],
        now: None,
    })
    .await
    .unwrap();
    let completed = db
        .complete_agent_creation_request(CompleteAgentCreationRequestInput {
            request_id: creation.request.id.clone(),
            runner_id: runner.clone(),
            lease_token: creation_lease_token,
            source_host_id: source_host.clone(),
            source_machine_id: source_machine.clone(),
            runtime_artifact_id: Some(artifact_id),
            state_schema_version: Some("state-v1".to_string()),
            provider_runtime_handle: None,
            contact_endpoint: Some("http://127.0.0.1:4200/contact".to_string()),
            runtime_capabilities: relocation_capacity().runtime_capabilities,
            display_name: None,
            hostname: None,
            runtime_host: Some(source_host.clone()),
            runtime_status: Some(RuntimeSummaryStatus::Online),
            active_inference_profile: None,
            hermes_available: Some(true),
            published_app_urls: vec![],
            agent_npub: Some(agent_npub.clone()),
            now: None,
        })
        .await
        .unwrap();

    let runtime_id = completed.request.agent_runtime_id.unwrap();
    let project_id = completed.project.id;
    PreparedRelocation {
        origin_request_id: created.request.id.clone(),
        request_id: created.request.id,
        project_id,
        runtime_id,
        target_host: source_host.clone(),
        source_host,
        source_machine: source_machine_for_fixture,
        artifact_id: artifact_id_for_fixture,
        run: run.to_string(),
        predecessor_secret,
    }
}

async fn prepare_relocation(db: &TestDb, run: &str, enroll_initial: bool) -> PreparedRelocation {
    let runtime = create_runtime(db, run, enroll_initial).await;
    let fixture = enqueue_relocation(db, &runtime).await;
    lease_relocation(
        db,
        &fixture,
        &format!("api-relocation-{run}-relocation-lease"),
    )
    .await;
    fixture
}

async fn lease_relocation(db: &TestDb, fixture: &PreparedRelocation, token: &str) {
    let lease = db
        .lease_agent_creation_request(LeaseAgentCreationRequestInput {
            runner_id: "runner-oslo-1".to_string(),
            source_host_id: Some(fixture.target_host.clone()),
            lease_token: token.to_string(),
            lease_seconds: Some(300),
            runner_capacity: Some(relocation_capacity()),
            now: None,
        })
        .await
        .unwrap()
        .expect("relocation request should lease");
    assert_eq!(lease.request.id, fixture.request_id);
}

async fn enqueue_relocation(db: &TestDb, fixture: &PreparedRelocation) -> PreparedRelocation {
    let relocation = db
        .admin_request_runtime_relocate_exact(AdminRuntimeRelocateExactInput {
            admin_verified_email: format!("{}-retry-admin@finite.vip", fixture.run),
            admin_workos_user_id: format!("api-relocation-{}-retry-admin", fixture.run),
            project_id: fixture.project_id.clone(),
            expected_agent_runtime_id: fixture.runtime_id.clone(),
            expected_source_host_id: fixture.source_host.clone(),
            expected_source_machine_id: fixture.source_machine.clone(),
            target_source_host_id: fixture.target_host.clone(),
            expected_agent_npub: format!("npub1{}", "a".repeat(58)),
            durable_state_manifest_sha256: "c".repeat(64),
            operator_observed_compute_absent: fixture.source_host == fixture.target_host,
            now: None,
        })
        .await
        .unwrap();
    PreparedRelocation {
        origin_request_id: fixture.origin_request_id.clone(),
        request_id: relocation.id,
        project_id: fixture.project_id.clone(),
        runtime_id: fixture.runtime_id.clone(),
        source_host: fixture.source_host.clone(),
        target_host: fixture.target_host.clone(),
        source_machine: fixture.source_machine.clone(),
        artifact_id: fixture.artifact_id.clone(),
        run: fixture.run.clone(),
        predecessor_secret: fixture.predecessor_secret.clone(),
    }
}

async fn provision_relocation_over_http(
    app: &Router,
    request_id: &str,
    lease_token: &str,
) -> (StatusCode, serde_json::Value) {
    send_json(
        app,
        "POST",
        &format!("/api/core/v1/agent-creation-requests/{request_id}/relocation-credential"),
        &[("authorization".to_string(), runner_authorization())],
        Some(serde_json::json!({
            "runnerId": "runner-oslo-1",
            "leaseToken": lease_token
        })),
    )
    .await
}

async fn expire_creation_lease(db: &TestDb, request_id: &str) {
    execute_test_sql(db,
        "UPDATE agent_creation_requests SET lease_expires_at = CURRENT_TIMESTAMP - INTERVAL '1 second' WHERE id = $1",
        &[&request_id],
    ).await;
}

async fn execute_test_sql(
    db: &TestDb,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) {
    let (client, connection) = tokio_postgres::connect(&db.url, tokio_postgres::NoTls)
        .await
        .unwrap();
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });
    client.execute(sql, params).await.unwrap();
    drop(client);
    connection.abort();
}

async fn credential_state_for_creation(
    db: &TestDb,
    creation_request_id: &str,
) -> (bool, bool, Option<String>) {
    let (client, connection) = tokio_postgres::connect(&db.url, tokio_postgres::NoTls)
        .await
        .unwrap();
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });
    let row = client
        .query_one(
            "SELECT revoked, activated, agent_runtime_id
             FROM runtime_core_credentials
             WHERE creation_request_id = $1",
            &[&creation_request_id],
        )
        .await
        .unwrap();
    let state = (row.get(0), row.get(1), row.get(2));
    drop(client);
    connection.abort();
    state
}

async fn creation_status(db: &TestDb, request_id: &str) -> String {
    let (client, connection) = tokio_postgres::connect(&db.url, tokio_postgres::NoTls)
        .await
        .unwrap();
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });
    let status = client
        .query_one(
            "SELECT status FROM agent_creation_requests WHERE id = $1",
            &[&request_id],
        )
        .await
        .unwrap()
        .get(0);
    drop(client);
    connection.abort();
    status
}

async fn current_credential_state(db: &TestDb, runtime_id: &str) -> (bool, bool, Option<String>) {
    let (client, connection) = tokio_postgres::connect(&db.url, tokio_postgres::NoTls)
        .await
        .unwrap();
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });
    let row = client
        .query_one(
            "SELECT revoked, activated, agent_runtime_id
             FROM runtime_core_credentials
             WHERE agent_runtime_id = $1",
            &[&runtime_id],
        )
        .await
        .unwrap();
    let state = (row.get(0), row.get(1), row.get(2));
    drop(client);
    connection.abort();
    state
}

async fn register_relocation(db: &TestDb, fixture: &PreparedRelocation, lease_token: &str) {
    db.register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
        request_id: fixture.request_id.clone(),
        runner_id: "runner-oslo-1".to_string(),
        lease_token: lease_token.to_string(),
        source_host_id: fixture.target_host.clone(),
        source_machine_id: fixture.source_machine.clone(),
        runtime_artifact_id: Some(fixture.artifact_id.clone()),
        state_schema_version: Some("state-v1".to_string()),
        provider_runtime_handle: None,
        contact_endpoint: Some("http://127.0.0.1:4200/contact".to_string()),
        runtime_capabilities: relocation_capacity().runtime_capabilities,
        display_name: None,
        hostname: None,
        runtime_host: Some(fixture.target_host.clone()),
        runtime_status: Some(RuntimeSummaryStatus::Online),
        active_inference_profile: None,
        hermes_available: Some(true),
        published_app_urls: vec![],
        now: None,
    })
    .await
    .unwrap();
}

fn relocation_completion(
    fixture: &PreparedRelocation,
    lease_token: &str,
) -> CompleteAgentCreationRequestInput {
    CompleteAgentCreationRequestInput {
        request_id: fixture.request_id.clone(),
        runner_id: "runner-oslo-1".to_string(),
        lease_token: lease_token.to_string(),
        source_host_id: fixture.target_host.clone(),
        source_machine_id: fixture.source_machine.clone(),
        runtime_artifact_id: Some(fixture.artifact_id.clone()),
        state_schema_version: Some("state-v1".to_string()),
        provider_runtime_handle: None,
        contact_endpoint: Some("http://127.0.0.1:4200/contact".to_string()),
        runtime_capabilities: relocation_capacity().runtime_capabilities,
        display_name: None,
        hostname: None,
        runtime_host: Some(fixture.target_host.clone()),
        runtime_status: Some(RuntimeSummaryStatus::Online),
        active_inference_profile: None,
        hermes_available: Some(true),
        published_app_urls: vec![],
        agent_npub: None,
        now: None,
    }
}

/// Posts the same body a Runner sends, so the status is what a Runner sees.
async fn complete_relocation_over_http(
    app: &Router,
    fixture: &PreparedRelocation,
    lease_token: &str,
) -> StatusCode {
    let body = serde_json::to_value(relocation_completion(fixture, lease_token)).unwrap();
    send_json(
        app,
        "POST",
        &format!(
            "/api/core/v1/agent-creation-requests/{}/complete",
            fixture.request_id
        ),
        &[("authorization".to_string(), runner_authorization())],
        Some(body),
    )
    .await
    .0
}

async fn fail_relocation_over_http(
    app: &Router,
    fixture: &PreparedRelocation,
    lease_token: &str,
) -> StatusCode {
    let body = serde_json::to_value(FailAgentCreationRequestInput {
        request_id: fixture.request_id.clone(),
        runner_id: "runner-oslo-1".to_string(),
        lease_token: lease_token.to_string(),
        failure_message: "completion response was lost".to_string(),
        provisioned_finite_private_api_key_id: None,
        now: None,
    })
    .unwrap();
    send_json(
        app,
        "POST",
        &format!(
            "/api/core/v1/agent-creation-requests/{}/fail",
            fixture.request_id
        ),
        &[("authorization".to_string(), runner_authorization())],
        Some(body),
    )
    .await
    .0
}

async fn complete_relocation(db: &TestDb, fixture: &PreparedRelocation, lease_token: &str) {
    db.complete_agent_creation_request(relocation_completion(fixture, lease_token))
        .await
        .unwrap();
}

#[tokio::test]
async fn relocation_credential_endpoint_returns_secret_only_to_bound_runner() {
    with_isolated_postgres(|db| async move {
        let request_id = prepare_relocation(&db, "enrolled", true).await.request_id;
        let app = router(db.store.clone(), scoped_test_auth());
        let body = serde_json::json!({
            "runnerId": "runner-oslo-1",
            "leaseToken": "api-relocation-enrolled-relocation-lease"
        });
        let (status, response) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/agent-creation-requests/{request_id}/relocation-credential"),
            &[("authorization".to_string(), runner_authorization())],
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["secret"].as_str().unwrap().len(), 64);

        let response_headers = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/agent-creation-requests/{request_id}/relocation-credential"
                    ))
                    .header("authorization", runner_authorization())
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response_headers.status(), StatusCode::OK);
        assert_eq!(
            response_headers
                .headers()
                .get("cache-control")
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );

        let (status, _) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/agent-creation-requests/{request_id}/relocation-credential"),
            &[("authorization".to_string(), runner_authorization())],
            Some(serde_json::json!({
                "runnerId": "wrong-runner",
                "leaseToken": "api-relocation-enrolled-relocation-lease"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/agent-creation-requests/{request_id}/relocation-credential"),
            &[(
                "authorization".to_string(),
                "Bearer invalid-runner-token".to_string(),
            )],
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    })
    .await;
}

#[tokio::test]
async fn relocation_credential_endpoint_returns_null_for_unenrolled_relocation() {
    with_isolated_postgres(|db| async move {
        let request_id = prepare_relocation(&db, "unenrolled", false)
            .await
            .request_id;
        let app = router(db.store.clone(), scoped_test_auth());
        let (status, response) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/agent-creation-requests/{request_id}/relocation-credential"),
            &[("authorization".to_string(), runner_authorization())],
            Some(serde_json::json!({
                "runnerId": "runner-oslo-1",
                "leaseToken": "api-relocation-unenrolled-relocation-lease"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(response.is_null());
    })
    .await;
}

#[tokio::test]
async fn relocation_credential_retries_are_idempotent_and_recover_expired_leases() {
    with_isolated_postgres(|db| async move {
        let fixture = prepare_relocation(&db, "retry", true).await;
        let app = router(db.store.clone(), scoped_test_auth());
        let lease_token = "api-relocation-retry-relocation-lease";

        let (left, right) = tokio::join!(
            provision_relocation_over_http(&app, &fixture.request_id, lease_token),
            provision_relocation_over_http(&app, &fixture.request_id, lease_token),
        );
        assert_eq!(left.0, StatusCode::OK);
        assert_eq!(right.0, StatusCode::OK);
        let left_secret = left.1["secret"].as_str().unwrap();
        let right_secret = right.1["secret"].as_str().unwrap();
        assert!(
            left_secret == right_secret,
            "concurrent provisioning diverged"
        );
        assert_eq!(
            credential_state_for_creation(&db, &fixture.request_id).await,
            (false, false, None)
        );

        expire_creation_lease(&db, &fixture.request_id).await;
        let replacement_lease = "api-relocation-retry-replacement-lease";
        lease_relocation(&db, &fixture, replacement_lease).await;
        let (status, replacement) =
            provision_relocation_over_http(&app, &fixture.request_id, replacement_lease).await;
        assert_eq!(status, StatusCode::OK);
        let replacement_secret = replacement["secret"].as_str().unwrap();
        assert!(
            replacement_secret == left_secret,
            "lease takeover changed the pending credential"
        );

        let (status, _) =
            provision_relocation_over_http(&app, &fixture.request_id, lease_token).await;
        assert_eq!(status, StatusCode::CONFLICT);
    })
    .await;
}

async fn assert_legacy_cannot_lease(db: &TestDb, fixture: &PreparedRelocation) {
    let legacy_capacity: RunnerLeaseCapacity = serde_json::from_value(serde_json::json!({
        "runnerClasses": ["kata"], "runtimeCapabilities": relocation_capacity().runtime_capabilities.unwrap()
    })).unwrap();
    assert!(!legacy_capacity.supports_relocation_credentials);
    for runner_capacity in [None, Some(legacy_capacity)] {
        let lease = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "legacy-runner".into(),
                source_host_id: Some(fixture.target_host.clone()),
                lease_token: "legacy-relocation-lease".into(),
                lease_seconds: Some(300),
                runner_capacity,
                now: None,
            })
            .await
            .unwrap();
        assert!(lease.is_none());
    }
}
