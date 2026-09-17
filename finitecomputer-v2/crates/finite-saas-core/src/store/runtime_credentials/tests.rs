use super::*;
use crate::test_support::{TestDb, with_isolated_postgres};
use crate::{
    RunnerClass, RunnerLeaseCapacity, RuntimeArtifactKind, RuntimeCapabilitiesEnvelope,
    RuntimeCapabilitiesV1,
};
pub(crate) async fn requested(db: &TestDb) -> String {
    let code = db
        .issue_launch_code_batch(IssueLaunchCodeBatchInput {
            name: "runtime bootstrap".into(),
            code_count: 1,
            expires_in_hours: Some(24),
            hosting_tier: None,
            created_by_workos_user_id: "operator".into(),
            now: None,
        })
        .await
        .unwrap()
        .codes[0]
        .code
        .clone();
    let request = db
        .request_agent_creation(RequestAgentCreationInput {
            verified_email: "runtime-auth@finite.test".into(),
            workos_user_id: "runtime-auth-user".into(),
            display_name: "Runtime auth".into(),
            launch_code: code,
            idempotency_key: "runtime-auth-create".into(),
            now: None,
        })
        .await
        .unwrap()
        .request;
    db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
        runner_id: "auth-runner".into(),
        source_host_id: Some("auth-host".into()),
        lease_token: "test-launch-lease".into(),
        lease_seconds: Some(300),
        runner_capacity: None,
        now: None,
    })
    .await
    .unwrap()
    .unwrap();
    request.id
}
pub(crate) fn provision(request: &str) -> ProvisionRuntimeCredential {
    ProvisionRuntimeCredential {
        creation_request_id: request.into(),
        runner_id: "auth-runner".into(),
        source_host_id: "auth-host".into(),
        lease_token: "test-launch-lease".into(),
    }
}
pub(crate) async fn register(db: &TestDb, request: &str) -> String {
    register_named(db, request, "auth-machine")
        .await
        .unwrap()
        .request
        .agent_runtime_id
        .unwrap()
}
async fn register_named(
    db: &TestDb,
    request: &str,
    machine: &str,
) -> CoreResult<AgentCreationLease> {
    db.register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
        request_id: request.into(),
        runner_id: "auth-runner".into(),
        lease_token: "test-launch-lease".into(),
        source_host_id: "auth-host".into(),
        source_machine_id: machine.into(),
        runtime_artifact_id: Some("artifact-postgres-fixture".into()),
        state_schema_version: Some("state-v1".into()),
        provider_runtime_handle: None,
        contact_endpoint: None,
        runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
            runtime_upgrade: true,
            restart: true,
            stop: true,
            ..Default::default()
        })),
        display_name: None,
        hostname: None,
        runtime_host: None,
        runtime_status: None,
        active_inference_profile: None,
        hermes_available: None,
        published_app_urls: vec![],
        now: None,
    })
    .await
}
pub(crate) async fn complete(db: &TestDb, request: &str) -> CoreResult<AgentCreationLease> {
    db.complete_agent_creation_request(CompleteAgentCreationRequestInput {
        request_id: request.into(),
        runner_id: "auth-runner".into(),
        lease_token: "test-launch-lease".into(),
        source_host_id: "auth-host".into(),
        source_machine_id: "auth-machine".into(),
        runtime_artifact_id: Some("artifact-postgres-fixture".into()),
        state_schema_version: Some("state-v1".into()),
        provider_runtime_handle: None,
        contact_endpoint: None,
        runtime_capabilities: None,
        display_name: None,
        hostname: None,
        runtime_host: None,
        runtime_status: Some(RuntimeSummaryStatus::Online),
        active_inference_profile: None,
        hermes_available: None,
        published_app_urls: vec![],
        agent_npub: None,
        now: None,
    })
    .await
}
async fn expire(db: &TestDb, request: &str) {
    db.connection().await.unwrap().execute("UPDATE agent_creation_requests SET lease_expires_at=clock_timestamp()-INTERVAL '1 second' WHERE id=$1", &[&request]).await.unwrap();
}
pub(crate) async fn upgrade(db: &TestDb, creation: &str) -> RuntimeControlLease {
    let project = db
        .agent_creation_request(creation)
        .await
        .unwrap()
        .project_id;
    db.upsert_runtime_artifact(UpsertRuntimeArtifactInput {
        id: "enrollment-v2".into(),
        kind: RuntimeArtifactKind::OciImage,
        reference: format!(
            "ghcr.io/finitecomputer/agent-runtime:enrollment@sha256:{}",
            "b".repeat(64)
        ),
        version_label: "enrollment".into(),
        source_git_sha: None,
        finitec_version: None,
        hermes_source_ref: None,
        finite_platform_plugin_ref: None,
        state_schema_version: "state-v1".into(),
        base_image: None,
        recover_known_good_chat: false,
        promoted: true,
        now: None,
    })
    .await
    .unwrap();
    db.admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
        admin_verified_email: "enrollment-operator@finite.test".into(),
        admin_workos_user_id: "enrollment-operator".into(),
        project_id: project,
        target_runtime_artifact_id: "enrollment-v2".into(),
        now: None,
    })
    .await
    .unwrap();
    db.lease_runtime_control_request(LeaseRuntimeControlRequestInput {
        runner_id: "auth-runner".into(),
        lease_token: "upgrade-lease".into(),
        lease_seconds: Some(300),
        source_host_id: Some("auth-host".into()),
        runner_capacity: Some(RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::Kata],
            runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                runtime_upgrade: true,
                restart: true,
                stop: true,
                ..Default::default()
            })),
            ..Default::default()
        }),
        now: None,
    })
    .await
    .unwrap()
    .unwrap()
}
pub(crate) fn upgrade_input(lease: &RuntimeControlLease) -> ProvisionUpgradeCredential {
    ProvisionUpgradeCredential {
        request_id: lease.request.id.clone(),
        runner_id: "auth-runner".into(),
        lease_token: "upgrade-lease".into(),
        source_host_id: "auth-host".into(),
    }
}

#[tokio::test]
async fn existing_enrollment_preserves_assignment_and_native_generation_on_replay() {
    for already_enrolled in [false, true] {
        with_isolated_postgres(move |db| async move {
                let creation = requested(&db).await;
                let initial = if already_enrolled {
                    Some(
                        db.provision_runtime_credential(provision(&creation))
                            .await
                            .unwrap()
                            .secret,
                    )
                } else {
                    None
                };
                let runtime = register(&db, &creation).await;
                complete(&db, &creation).await.unwrap();
                assert!(db.hosted_route_targets_for_host("auth-host").await.unwrap().is_empty());
                let lease = upgrade(&db, &creation).await;
                let secret = db
                    .provision_upgrade_credential(upgrade_input(&lease))
                    .await
                    .unwrap()
                    .secret;
                if let Some(initial) = initial {
                    assert!(initial == secret);
                }
                assert_eq!(
                    db.authenticate_runtime_credential(&secret)
                        .await
                        .unwrap()
                        .unwrap()
                        .agent_runtime_id,
                    runtime
                );
                let state = db
                    .hosted_access(&runtime, "runtime-auth-user")
                    .await
                    .unwrap();
                assert!(state.enrolled && !state.enabled);
                assert!(db.hosted_route_targets_for_host("auth-host").await.unwrap().is_empty());
                let enabled = db
                    .set_hosted_access(
                        &runtime,
                        "runtime-auth-user",
                        super::super::hosted_hermes::SetHostedAccess {
                            enabled: true,
                            expected_generation: state.generation,
                        },
                    )
                    .await
                    .unwrap();
                // FIN-90 enrollment must not bypass FIN-91's applied-generation gate.
                assert!(db.hosted_route_targets_for_host("auth-host").await.unwrap().is_empty());
                assert!(db.report_hosted(&secret, super::super::hosted_hermes::HostedReport {
                    generation: enabled.generation - 1,
                    status: super::super::hosted_hermes::ApplyStatus::Applied,
                }).await.is_err());
                assert!(db.hosted_route_targets_for_host("auth-host").await.unwrap().is_empty());
                db.report_hosted(&secret, super::super::hosted_hermes::HostedReport {
                    generation: enabled.generation,
                    status: super::super::hosted_hermes::ApplyStatus::Applied,
                }).await.unwrap();
                let routes = db.hosted_route_targets_for_host("auth-host").await.unwrap();
                assert_eq!(routes, vec![super::super::hosted_hermes::HostedRouteTarget {
                    runtime_id: runtime.clone(),
                    project_id: lease.request.project_id.clone(),
                    source_machine_id: "auth-machine".into(),
                    generation: enabled.generation,
                }]);
                assert!(db.hosted_route_targets_for_host("other-host").await.unwrap().is_empty());
                let origins = crate::hosted_hermes::HostedHermesOrigins::from_json(
                    r#"{"auth-host":"https://agent.example.test"}"#,
                )
                .unwrap();
                let before = db.hosted_desired(&secret, &origins).await.unwrap().unwrap();
                let retry = db
                    .provision_upgrade_credential(upgrade_input(&lease))
                    .await
                    .unwrap();
                assert!(retry.secret == secret);
                // A replacement worker's live lease can retry delivery without rotation.
                db.connection().await.unwrap().execute("UPDATE runtime_control_requests SET lease_token='replacement-lease' WHERE id=$1", &[&lease.request.id]).await.unwrap();
                assert!(db.provision_upgrade_credential(upgrade_input(&lease)).await.is_err());
                let mut replacement = upgrade_input(&lease);
                replacement.lease_token = "replacement-lease".into();
                assert!(db.provision_upgrade_credential(replacement).await.unwrap().secret == secret);
                db.connection().await.unwrap().execute("UPDATE runtime_control_requests SET lease_token='upgrade-lease' WHERE id=$1", &[&lease.request.id]).await.unwrap();

                assert_eq!(db.hosted_route_targets_for_host("auth-host").await.unwrap(), routes);
                let after = db.hosted_desired(&secret, &origins).await.unwrap().unwrap();
                assert_eq!(after.generation, enabled.generation);
                assert!(
                    before.password == after.password
                        && before.signing_secret == after.signing_secret
                );
                assert!(
                    db.revoke_runtime_credential(&runtime, &creation)
                        .await
                        .unwrap()
                );
                assert!(db.hosted_route_targets_for_host("auth-host").await.unwrap().is_empty());
                assert!(
                    db.provision_upgrade_credential(upgrade_input(&lease))
                        .await
                        .is_err()
                );
                assert!(
                    db.authenticate_runtime_credential(&secret)
                        .await
                        .unwrap()
                        .is_none()
                );
            })
            .await;
    }
}

#[tokio::test]
async fn existing_enrollment_rejects_stale_authority_without_creating_credentials() {
    for mutation in [
        "UPDATE runtime_control_requests SET lease_expires_at=clock_timestamp()-INTERVAL '1 second'",
        "UPDATE runtime_control_requests SET source_machine_id='stale-machine'",
        "UPDATE runtime_control_requests SET source_host_id='other-host'",
        "UPDATE runtime_control_requests SET kind='restart',target_runtime_artifact_id=NULL",
        "UPDATE project_runtime_links SET active=FALSE",
        "UPDATE projects SET owner_user_id=(SELECT id FROM users WHERE workos_user_id='enrollment-operator')",
        "UPDATE agent_creation_requests SET status='cancelled'",
        "UPDATE agent_creation_requests SET relocation_spec='{}'::jsonb",
    ] {
        with_isolated_postgres(move |db| async move {
            let creation = requested(&db).await;
            register(&db, &creation).await;
            complete(&db, &creation).await.unwrap();
            let lease = upgrade(&db, &creation).await;
            db.connection()
                .await
                .unwrap()
                .execute(mutation, &[])
                .await
                .unwrap();
            assert!(
                db.provision_upgrade_credential(upgrade_input(&lease))
                    .await
                    .is_err(),
                "{mutation}"
            );
            let count: i64 = db
                .connection()
                .await
                .unwrap()
                .query_one("SELECT count(*) FROM runtime_core_credentials", &[])
                .await
                .unwrap()
                .get(0);
            assert_eq!(count, 0);
        })
        .await;
    }
}

#[tokio::test]
async fn existing_enrollment_uses_primary_origin_not_relocation_history() {
    with_isolated_postgres(|db| async move {
            let creation = requested(&db).await;
            register(&db, &creation).await;
            complete(&db, &creation).await.unwrap();
            let lease = upgrade(&db, &creation).await;
            let relocation = serde_json::to_value(crate::RuntimeRelocationEnvelope::V1(crate::RuntimeRelocationV1 {
                source_host_id: "previous-host".into(), source_machine_id: "previous-machine".into(),
                target_source_host_id: "auth-host".into(), expected_agent_npub: "npub-history-fixture".into(),
                durable_state_manifest_sha256: "a".repeat(64), source_compute_absent: true,
            })).unwrap();
            db.connection().await.unwrap().execute(
                "INSERT INTO agent_creation_requests SELECT (jsonb_populate_record(NULL::agent_creation_requests,
                 to_jsonb(q)||jsonb_build_object('id',q.id||'-relocation','idempotency_key',q.idempotency_key||'-relocation','relocation_spec',$2::jsonb))).*
                 FROM agent_creation_requests q WHERE q.id=$1", &[&creation, &relocation],
            ).await.unwrap();
            let secret = db.provision_upgrade_credential(upgrade_input(&lease)).await.unwrap().secret;
            let authenticated = db.authenticate_runtime_credential(&secret).await.unwrap().unwrap();
            assert_eq!(authenticated.creation_request_id, creation);
            let retry = db.provision_upgrade_credential(upgrade_input(&lease)).await.unwrap();
            assert!(retry.secret == secret);
            // Relocation completion revokes the old credential. Primary history
            // must never resurrect that previous incarnation.
            db.revoke_runtime_credential(&lease.runtime.id, &creation).await.unwrap();
            assert!(db.provision_upgrade_credential(upgrade_input(&lease)).await.is_err());
        }).await;
}

#[tokio::test]
async fn existing_enrollment_rejects_unbound_stale_credential() {
    with_isolated_postgres(|db| async move {
            let creation = requested(&db).await;
            register(&db, &creation).await;
            complete(&db, &creation).await.unwrap();
            let lease = upgrade(&db, &creation).await;
            db.provision_upgrade_credential(upgrade_input(&lease)).await.unwrap();
            db.connection().await.unwrap().execute(
                "UPDATE runtime_core_credentials SET agent_runtime_id=NULL WHERE creation_request_id=$1", &[&creation],
            ).await.unwrap();
            assert!(db.provision_upgrade_credential(upgrade_input(&lease)).await.is_err());
        }).await;
}

#[tokio::test]
async fn existing_enrollment_http_requires_exact_runner_and_live_upgrade_lease() {
    use crate::auth::test_support::{core_auth_with_runner_credentials, runner_credential_config};
    with_isolated_postgres(|db| async move {
        let creation = requested(&db).await;
        register(&db, &creation).await;
        complete(&db, &creation).await.unwrap();
        let lease = upgrade(&db, &creation).await;
        let auth = core_auth_with_runner_credentials(
            "service",
            vec![
                runner_credential_config(
                    "runner",
                    "runner-secret",
                    "auth-runner",
                    &[RunnerClass::Kata],
                    "auth-host",
                    false,
                ),
                runner_credential_config(
                    "other",
                    "other-secret",
                    "other-runner",
                    &[RunnerClass::Kata],
                    "other-host",
                    false,
                ),
            ],
            "usage",
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/api/core/v1/runtime-control-requests/{}/runtime-credential",
            listener.local_addr().unwrap(),
            lease.request.id
        );
        let app = crate::api::router(db.store.clone(), auth);
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::new();
        for (bearer, runner, token) in [
            ("service", "auth-runner", "upgrade-lease"),
            ("usage", "auth-runner", "upgrade-lease"),
            ("other-secret", "auth-runner", "upgrade-lease"),
            ("other-secret", "other-runner", "upgrade-lease"),
            ("runner-secret", "auth-runner", "old-lease"),
        ] {
            let response = client
                .post(&url)
                .bearer_auth(bearer)
                .json(&serde_json::json!({"runnerId":runner,"leaseToken":token}))
                .send()
                .await
                .unwrap();
            assert!(response.status().is_client_error());
        }
        let body = serde_json::json!({"runnerId":"auth-runner","leaseToken":"upgrade-lease"});
        let response = client
            .post(&url)
            .bearer_auth("runner-secret")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let first = response.json::<RuntimeBootstrapCredential>().await.unwrap();
        let second = client
            .post(&url)
            .bearer_auth("runner-secret")
            .json(&body)
            .send()
            .await
            .unwrap()
            .json::<RuntimeBootstrapCredential>()
            .await
            .unwrap();
        assert!(first.secret == second.secret);
        server.abort();
    })
    .await;
}

#[tokio::test]
async fn runtime_bootstrap_direct_completion_and_immutable_binding() {
    with_isolated_postgres(|db| async move {
        let request = requested(&db).await;
        let secret = db
            .provision_runtime_credential(provision(&request))
            .await
            .unwrap()
            .secret;
        complete(&db, &request).await.unwrap();
        assert!(
            db.authenticate_runtime_credential(&secret)
                .await
                .unwrap()
                .is_some()
        );
    })
    .await;
    with_isolated_postgres(|db| async move {
        let request = requested(&db).await;
        let secret = db
            .provision_runtime_credential(provision(&request))
            .await
            .unwrap()
            .secret;
        let runtime = register(&db, &request).await;
        assert!(
            register_named(&db, &request, "different-machine")
                .await
                .is_err()
        );
        assert_eq!(
            db.authenticate_runtime_credential(&secret)
                .await
                .unwrap()
                .unwrap()
                .agent_runtime_id,
            runtime
        );
    })
    .await;
}
#[tokio::test]
async fn runtime_bootstrap_origin_precedes_runtime_and_replays_after_reconnect() {
    with_isolated_postgres(|db| async move {
        let request = requested(&db).await;
        let secret = db
            .provision_runtime_credential(provision(&request))
            .await
            .unwrap()
            .secret;
        assert!(
            db.authenticate_runtime_credential(&secret)
                .await
                .unwrap()
                .is_none()
        );
        let runtime = register(&db, &request).await;
        let mut padded = provision(&request);
        padded.lease_token = "  test-launch-lease\n".into();
        assert!(
            db.provision_runtime_credential(padded)
                .await
                .unwrap()
                .secret
                == secret
        );
        assert!(
            db.provision_runtime_credential(provision(&request))
                .await
                .unwrap()
                .secret
                == secret
        );
        assert_eq!(
            db.authenticate_runtime_credential(&secret)
                .await
                .unwrap()
                .unwrap()
                .agent_runtime_id,
            runtime
        );
        complete(&db, &request).await.unwrap();
        assert!(
            db.authenticate_runtime_credential(&secret)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            db.provision_runtime_credential(provision(&request))
                .await
                .is_err()
        );
    })
    .await;
}
#[tokio::test]
async fn runtime_bootstrap_rejects_wrong_host_lease_and_expired_completion() {
    with_isolated_postgres(|db| async move {
        let request = requested(&db).await;
        let secret = db
            .provision_runtime_credential(provision(&request))
            .await
            .unwrap()
            .secret;
        let mut wrong = provision(&request);
        wrong.source_host_id = "other-host".into();
        assert!(db.provision_runtime_credential(wrong).await.is_err());
        let mut wrong = provision(&request);
        wrong.lease_token = "wrong".into();
        assert!(db.provision_runtime_credential(wrong).await.is_err());
        register(&db, &request).await;
        expire(&db, &request).await;
        assert!(
            db.provision_runtime_credential(provision(&request))
                .await
                .is_err()
        );
        assert!(complete(&db, &request).await.is_err());
        assert!(
            db.authenticate_runtime_credential(&secret)
                .await
                .unwrap()
                .is_none()
        );
    })
    .await;
}
#[tokio::test]
async fn runtime_bootstrap_live_takeover_recovers_same_installed_secret() {
    with_isolated_postgres(|db| async move {
        let request = requested(&db).await;
        let secret = db
            .provision_runtime_credential(provision(&request))
            .await
            .unwrap()
            .secret;
        register(&db, &request).await;
        expire(&db, &request).await;
        db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
            runner_id: "auth-runner".into(),
            source_host_id: Some("auth-host".into()),
            lease_token: "new-lease".into(),
            lease_seconds: Some(300),
            runner_capacity: None,
            now: None,
        })
        .await
        .unwrap()
        .unwrap();
        assert!(
            db.authenticate_runtime_credential(&secret)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            db.provision_runtime_credential(provision(&request))
                .await
                .is_err()
        );
        let mut input = provision(&request);
        input.lease_token = "new-lease".into();
        assert!(db.provision_runtime_credential(input).await.unwrap().secret == secret);
        assert!(
            db.authenticate_runtime_credential(&secret)
                .await
                .unwrap()
                .is_some()
        );
    })
    .await;
}
#[tokio::test]
async fn runtime_bootstrap_offboarding_both_orders_and_control_fence() {
    for enroll in [true, false] {
        with_isolated_postgres(|db| async move {
            let request = requested(&db).await;
            let runtime = register(&db, &request).await;
            let secret = if enroll {
                Some(
                    db.provision_runtime_credential(provision(&request))
                        .await
                        .unwrap()
                        .secret,
                )
            } else {
                None
            };
            let project_id = db.agent_runtime(&runtime).await.unwrap().project_id;
            db.admin_archive_unrecoverable_runtime(AdminArchiveUnrecoverableRuntimeInput {
                admin_verified_email: "operator@finite.test".into(),
                admin_workos_user_id: "operator".into(),
                project_id,
                expected_agent_runtime_id: runtime,
                expected_source_host_id: "auth-host".into(),
                expected_source_machine_id: "auth-machine".into(),
                expected_owner_email: "runtime-auth@finite.test".into(),
                operator_observed_compute_absent: true,
                operator_observed_durable_state_absent: true,
                owner_acknowledged_unrecoverable: true,
                now: None,
            })
            .await
            .unwrap();
            assert!(
                db.provision_runtime_credential(provision(&request))
                    .await
                    .is_err()
            );
            if let Some(secret) = secret {
                assert!(
                    db.authenticate_runtime_credential(&secret)
                        .await
                        .unwrap()
                        .is_none()
                );
            }
        })
        .await;
    }
    with_isolated_postgres(|db| async move {
        let request = requested(&db).await;
        let runtime = register(&db, &request).await;
        db.request_runtime_restart(RequestRuntimeRestartInput {
            verified_email: "runtime-auth@finite.test".into(),
            workos_user_id: "runtime-auth-user".into(),
            project_id: db.agent_runtime(&runtime).await.unwrap().project_id,
            now: None,
        })
        .await
        .unwrap();
        assert!(
            db.provision_runtime_credential(provision(&request))
                .await
                .is_err()
        );
    })
    .await;
}
#[tokio::test]
async fn creation_credential_survives_restart_and_stop_resume_without_reviving_revocation() {
    use crate::{RunnerClass, RunnerLeaseCapacity};
    with_isolated_postgres(|db| async move {
        let request = requested(&db).await;
        let secret = db
            .provision_runtime_credential(provision(&request))
            .await
            .unwrap()
            .secret;
        let runtime = register(&db, &request).await;
        let project = complete(&db, &request).await.unwrap().project.id;
        for (index, stop) in [false, true, false, false].into_iter().enumerate() {
            // Explicitly revoke before the final restart: lifecycle success
            // must never resurrect a credential revoked by an operator.
            if index == 3 {
                db.revoke_runtime_credential(&runtime, &request)
                    .await
                    .unwrap();
            }
            let input = RequestRuntimeRestartInput {
                verified_email: "runtime-auth@finite.test".into(),
                workos_user_id: "runtime-auth-user".into(),
                project_id: project.clone(),
                now: None,
            };
            let operation = if stop {
                db.request_runtime_stop(input).await.unwrap()
            } else {
                db.request_runtime_restart(input).await.unwrap()
            };
            let lease = db
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: "auth-runner".into(),
                    lease_token: "control-lease".into(),
                    lease_seconds: Some(300),
                    source_host_id: Some("auth-host".into()),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                            RuntimeCapabilitiesV1 {
                                restart: true,
                                stop: true,
                                ..Default::default()
                            },
                        )),
                        ..Default::default()
                    }),
                    now: None,
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(lease.request.id, operation.id);
            db.complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: operation.id,
                runner_id: "auth-runner".into(),
                lease_token: "control-lease".into(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: None,
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: None,
                now: None,
            })
            .await
            .unwrap();
            assert_eq!(
                db.authenticate_runtime_credential(&secret)
                    .await
                    .unwrap()
                    .is_some(),
                !stop && index != 3
            );
        }
    })
    .await;
}
#[tokio::test]
async fn runtime_bootstrap_real_http_requires_matching_runner_lease() {
    use crate::auth::test_support::{core_auth_with_runner_credentials, runner_credential_config};
    with_isolated_postgres(|db| async move {
        let request = requested(&db).await;
        let auth = core_auth_with_runner_credentials(
            "service",
            vec![
                runner_credential_config(
                    "runner",
                    "runner-secret",
                    "auth-runner",
                    &[crate::RunnerClass::Kata],
                    "auth-host",
                    false,
                ),
                runner_credential_config(
                    "other",
                    "other-secret",
                    "other-runner",
                    &[crate::RunnerClass::Kata],
                    "other-host",
                    false,
                ),
            ],
            "usage",
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/api/core/v1/agent-creation-requests/{request}/runtime-credential",
            listener.local_addr().unwrap()
        );
        let app = crate::api::router(db.store.clone(), auth);
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::new();
        let body = serde_json::json!({"runnerId":"auth-runner","leaseToken":"test-launch-lease"});
        for bearer in ["service", "usage", "unknown", "other-secret"] {
            let response = client
                .post(&url)
                .bearer_auth(bearer)
                .json(&body)
                .send()
                .await
                .unwrap();
            assert!(
                response.status() == reqwest::StatusCode::UNAUTHORIZED
                    || response.status() == reqwest::StatusCode::FORBIDDEN
            );
        }
        let response = client
            .post(&url)
            .bearer_auth("runner-secret")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let issued = response.json::<RuntimeBootstrapCredential>().await.unwrap();
        let replay = client
            .post(&url)
            .bearer_auth("runner-secret")
            .json(&body)
            .send()
            .await
            .unwrap()
            .json::<RuntimeBootstrapCredential>()
            .await
            .unwrap();
        assert!(issued.secret == replay.secret);
        assert!(
            db.authenticate_runtime_credential(&issued.secret)
                .await
                .unwrap()
                .is_none()
        );
        register(&db, &request).await;
        complete(&db, &request).await.unwrap();
        assert!(
            db.authenticate_runtime_credential(&issued.secret)
                .await
                .unwrap()
                .is_some()
        );
        server.abort();
    })
    .await;
}
