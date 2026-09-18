use super::*;

fn candidate(runtime_id: &str) -> UpsertRuntimeArtifactInput {
    UpsertRuntimeArtifactInput {
        id: "artifact-canary".into(),
        kind: RuntimeArtifactKind::OciImage,
        reference: format!("ghcr.io/finite/runtime@sha256:{}", "c".repeat(64)),
        version_label: "canary".into(),
        source_git_sha: None,
        finitec_version: None,
        hermes_source_ref: None,
        finite_platform_plugin_ref: None,
        state_schema_version: "db-v1".into(),
        base_image: None,
        recover_known_good_chat: false,
        canary_runtime_id: Some(runtime_id.into()),
        promoted: false,
        now: None,
    }
}

async fn create_agent(db: &TestDb, name: &str) -> AgentRuntime {
    let id = complete_self_serve_agent(
        db,
        &format!("{name}@finite.vip"),
        &format!("workos-{name}"),
        name,
        &format!("machine-{name}"),
        "artifact-v1",
        "2026-05-25T13:02:00Z",
    )
    .await;
    db.agent_runtime(&id).await.unwrap()
}

fn upgrade(runtime: &AgentRuntime, artifact_id: &str) -> AdminRuntimeUpgradeExactInput {
    AdminRuntimeUpgradeExactInput {
        admin_verified_email: "admin@finite.vip".into(),
        admin_workos_user_id: "workos-admin".into(),
        project_id: runtime.project_id.clone(),
        expected_agent_runtime_id: runtime.id.clone(),
        expected_source_host_id: runtime.source_host_id.clone(),
        expected_source_machine_id: runtime.source_machine_id.clone(),
        target_runtime_artifact_id: artifact_id.into(),
        now: None,
    }
}

async fn execute_upgrade(db: &TestDb, runtime: &AgentRuntime, artifact_id: &str) {
    let request = db
        .admin_request_runtime_upgrade_exact(upgrade(runtime, artifact_id))
        .await
        .unwrap();
    finish_control(db, runtime, artifact_id, request).await;
}

async fn finish_control(
    db: &TestDb,
    runtime: &AgentRuntime,
    artifact_id: &str,
    request: RuntimeControlRequest,
) {
    let lease = db
        .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
            runner_id: "canary-runner".into(),
            lease_token: "canary-lease".into(),
            lease_seconds: Some(300),
            source_host_id: Some(runtime.source_host_id.clone()),
            runner_capacity: Some(RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Kata],
                runtime_capabilities: Some(kata_runtime_capabilities()),
                ..RunnerLeaseCapacity::default()
            }),
            now: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lease.request.id, request.id);
    if request.kind == RuntimeControlKind::Upgrade {
        assert_eq!(lease.target_runtime_artifact.unwrap().id, artifact_id);
    }
    assert_eq!(
        runtime_spec_v1(lease.runtime_spec.as_ref().unwrap()).runtime_artifact_id,
        artifact_id
    );
    db.complete_runtime_control_request(CompleteRuntimeControlRequestInput {
        request_id: request.id,
        runner_id: "canary-runner".into(),
        lease_token: "canary-lease".into(),
        runtime_artifact_id: Some(artifact_id.into()),
        state_schema_version: Some("db-v1".into()),
        runtime_capabilities: Some(kata_runtime_capabilities()),
        runtime_host: Some("http://127.0.0.1:41002".into()),
        published_app_urls: Some(vec!["http://127.0.0.1:41002/contact".into()]),
        retirement_snapshot: None,
        now: None,
    })
    .await
    .unwrap();
    assert_eq!(
        db.agent_runtime(&runtime.id)
            .await
            .unwrap()
            .runtime_artifact_id
            .as_deref(),
        Some(artifact_id)
    );
}

#[tokio::test]
async fn scoped_canary_upgrades_only_its_runtime_without_changing_new_launches_and_rolls_back() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let canary = create_agent(&db, "canary").await;
        let input = candidate(&canary.id);
        let registered = db.upsert_runtime_artifact(input.clone()).await.unwrap();
        assert!(registered.promoted_at.is_none());
        assert_eq!(db.upsert_runtime_artifact(input).await.unwrap(), registered);
        // A new user's real creation → lease → completion still selects v1,
        // even though the scoped candidate was registered later.
        let other = create_agent(&db, "other").await;
        assert!(matches!(
            db.admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                admin_verified_email: "admin@finite.vip".into(),
                admin_workos_user_id: "workos-admin".into(),
                project_id: canary.project_id.clone(),
                target_runtime_artifact_id: "artifact-canary".into(),
                now: None,
            }).await,
            Err(CoreError::RuntimeSpecMismatch)
        ));
        assert!(matches!(
            db.admin_request_runtime_upgrade_exact(upgrade(&other, "artifact-canary"))
                .await,
            Err(CoreError::RuntimeArtifactNotPromoted)
        ));
        let mut wrong_binding = upgrade(&canary, "artifact-canary");
        wrong_binding.expected_source_machine_id = "replaced-machine".into();
        assert!(matches!(
            db.admin_request_runtime_upgrade_exact(wrong_binding).await,
            Err(CoreError::RuntimeSpecMismatch)
        ));
        execute_upgrade(&db, &canary, "artifact-canary").await;
        let restart = db
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "canary@finite.vip".into(),
                workos_user_id: "workos-canary".into(),
                project_id: canary.project_id.clone(),
                now: None,
            })
            .await
            .unwrap();
        finish_control(&db, &canary, "artifact-canary", restart).await;
        // Current candidate need not be globally promoted to return through
        // normal Core/Runner lifecycle control to the previous release.
        execute_upgrade(&db, &canary, "artifact-v1").await;
        assert_eq!(db.agent_runtime(&other.id).await.unwrap(), other);
    })
    .await;
}

#[tokio::test]
async fn scoped_canary_stays_immutable_unpromoted_and_excluded_for_legacy_readers_and_writers() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        let canary = create_agent(&db, "canary").await;
        let input = candidate(&canary.id);
        db.upsert_runtime_artifact(input.clone()).await.unwrap();
        for change in ["promote", "scope", "image"] {
            let mut mutation = input.clone();
            match change {
                "promote" => mutation.promoted = true,
                "scope" => mutation.canary_runtime_id = None,
                _ => mutation.reference = format!("ghcr.io/finite/runtime@sha256:{}", "d".repeat(64)),
            }
            assert!(matches!(db.upsert_runtime_artifact(mutation).await, Err(CoreError::RuntimeArtifactImmutable)));
        }
        let (raw, connection) = tokio_postgres::connect(&db.url, tokio_postgres::NoTls).await.unwrap();
        let task = tokio::spawn(async move { connection.await.unwrap(); });
        // The previous Core selector, with no awareness of the new column.
        let selected: String = raw.query_one("SELECT id FROM runtime_artifacts WHERE promoted_at IS NOT NULL AND retired_at IS NULL AND kind = 'oci_image' ORDER BY promoted_at DESC, created_at DESC, id DESC LIMIT 1", &[]).await.unwrap().get(0);
        assert_eq!(selected, "artifact-v1");
        // Previous writer SQL cannot mutate or promote an unmounted candidate.
        for sql in [
            "UPDATE runtime_artifacts SET promoted_at = clock_timestamp() WHERE id = 'artifact-canary'",
            "UPDATE runtime_artifacts SET reference = 'different-image' WHERE id = 'artifact-canary'",
            "UPDATE runtime_artifacts SET canary_runtime_id = NULL WHERE id = 'artifact-canary'",
        ] {
            let error = raw.execute(sql, &[]).await.unwrap_err();
            assert_eq!(error.code(), Some(&tokio_postgres::error::SqlState::CHECK_VIOLATION));
        }
        // Reapplying migrations must preserve the policy and registration.
        db.migrate().await.unwrap();
        assert_eq!(db.runtime_artifact("artifact-canary").await.unwrap().unwrap().canary_runtime_id.as_deref(), Some(canary.id.as_str()));
        raw.execute("UPDATE runtime_artifacts SET retired_at = clock_timestamp() WHERE id = 'artifact-canary'", &[]).await.unwrap();
        assert!(matches!(db.admin_request_runtime_upgrade_exact(upgrade(&canary, "artifact-canary")).await,
            Err(CoreError::RuntimeArtifactRetired)));
        drop(raw); task.abort();
    }).await;
}

#[tokio::test]
async fn scoped_canary_requires_existing_runtime_immutable_image_and_compatible_state() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        assert!(
            db.upsert_runtime_artifact(candidate("missing-runtime"))
                .await
                .is_err()
        );
        let canary = create_agent(&db, "canary").await;
        let mut input = candidate(&canary.id);
        input.reference = "ghcr.io/finite/runtime:latest".into();
        assert!(matches!(
            db.upsert_runtime_artifact(input).await,
            Err(CoreError::RuntimeUpgradeUnsupported)
        ));
        let mut input = candidate(&canary.id);
        input.state_schema_version = "incompatible".into();
        db.upsert_runtime_artifact(input).await.unwrap();
        assert!(matches!(
            db.admin_request_runtime_upgrade_exact(upgrade(&canary, "artifact-canary"))
                .await,
            Err(CoreError::RuntimeUpgradeStateSchemaIncompatible)
        ));
    })
    .await;
}

#[tokio::test]
async fn scoped_canary_rejects_provisional_launch_without_blocking_failure_or_cancellation() {
    with_isolated_postgres(|db| async move {
        promote_runtime_artifact(&db).await;
        for cancel in [false, true] {
            let name = if cancel { "cancelled" } else { "failed" };
            let requested = db.request_agent_creation(RequestAgentCreationInput {
                verified_email: format!("{name}@finite.vip"),
                workos_user_id: format!("workos-{name}"),
                display_name: name.into(),
                launch_code: issue_test_launch_code(&db).await,
                idempotency_key: name.into(),
                now: None,
            }).await.unwrap();
            let lease = db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner".into(), source_host_id: None,
                lease_token: "lease".into(), lease_seconds: Some(300),
                runner_capacity: None, now: None,
            }).await.unwrap().unwrap();
            let registered = db.register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                request_id: lease.request.id.clone(), runner_id: "runner".into(),
                lease_token: "lease".into(), source_host_id: "oslo-host-1".into(),
                source_machine_id: format!("machine-{name}"),
                runtime_artifact_id: Some("artifact-v1".into()), state_schema_version: None,
                provider_runtime_handle: None, contact_endpoint: None,
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: None, hostname: None, runtime_host: None,
                runtime_status: Some(RuntimeSummaryStatus::Unknown),
                active_inference_profile: None, hermes_available: None,
                published_app_urls: vec![], now: None,
            }).await.unwrap();
            let id = registered.request.agent_runtime_id.unwrap();
            assert!(matches!(db.upsert_runtime_artifact(candidate(&id)).await,
                Err(CoreError::RuntimeUpgradeUnsupported)));
            let cleaned = if cancel {
                db.cancel_agent_creation_request(CancelAgentCreationRequestInput {
                    request_id: requested.request.id, now: None,
                }).await.unwrap()
            } else {
                db.fail_agent_creation_request(FailAgentCreationRequestInput {
                    request_id: requested.request.id, runner_id: "runner".into(),
                    lease_token: "lease".into(), failure_message: "initial launch failed".into(),
                    provisioned_finite_private_api_key_id: None, now: None,
                }).await.unwrap()
            };
            assert!(cleaned.agent_runtime_id.is_none());
            assert!(db.runtime_artifact("artifact-canary").await.unwrap().is_none());
            assert!(db.all_agent_runtimes().await.is_empty());
        }
    }).await;
}
