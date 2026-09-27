use crate::store::IssueLaunchCodeBatchInput;
use crate::store::hosted_hermes::{ApplyStatus, HostedReport, SetHostedAccess};
use crate::store::runtime_credentials::{ProvisionRuntimeCredential, ProvisionUpgradeCredential};
use crate::test_support::{TestDb, with_isolated_postgres};
use crate::{
    AdminRuntimeRelocateExactInput, AdminRuntimeUpgradeInput, AgentCreationConfiguration,
    CompleteAgentCreationRequestInput, CompleteRuntimeControlRequestInput, HostingTier,
    LeaseAgentCreationRequestInput, LeaseRuntimeControlRequestInput,
    RegisterAgentCreationRuntimeInput, RequestAgentCreationInput, RequestRuntimeStopInput,
    RunnerClass, RunnerLeaseCapacity, RuntimeArtifactKind, RuntimeCapabilitiesEnvelope,
    RuntimeCapabilitiesV1, RuntimeSummaryStatus, UpsertRuntimeArtifactInput,
};

fn capabilities() -> RuntimeCapabilitiesEnvelope {
    RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
        restart: true,
        runtime_upgrade: true,
        stop: true,
        ..Default::default()
    })
}

fn capacity() -> RunnerLeaseCapacity {
    RunnerLeaseCapacity {
        runner_classes: vec![RunnerClass::Kata],
        runtime_capabilities: Some(capabilities()),
        supports_relocation_credentials: true,
        ..Default::default()
    }
}

async fn artifact(db: &TestDb, id: &str, digest: char) {
    db.upsert_runtime_artifact(UpsertRuntimeArtifactInput {
        id: id.into(),
        kind: RuntimeArtifactKind::OciImage,
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

async fn launch_code(db: &TestDb) -> String {
    db.issue_launch_code_batch(IssueLaunchCodeBatchInput {
        name: "relocation credential regression".into(),
        code_count: 1,
        expires_in_hours: Some(24),
        hosting_tier: None,
        created_by_workos_user_id: "relocation-test-operator".into(),
        now: None,
    })
    .await
    .unwrap()
    .codes[0]
        .code
        .clone()
}

async fn run_relocation_credential_handoff(same_host: bool, unbound_predecessor: bool) {
    with_isolated_postgres(|db| async move {
        let suffix = if same_host { "same" } else { "cross" };
        let source_host = format!("relocation-{suffix}-source");
        let target_host = if same_host {
            source_host.clone()
        } else {
            "relocation-cross-target".into()
        };
        let source_machine = format!("finite-kata-relocation-{suffix}");
        let runner = format!("runner-{suffix}");
        let owner_email = format!("relocation-{suffix}@finite.test");
        let owner_workos = format!("relocation-{suffix}-owner");
        let admin_email = format!("relocation-{suffix}-admin@finite.vip");
        let admin_workos = format!("relocation-{suffix}-admin");

        artifact(&db, &format!("relocation-{suffix}-v1"), '1').await;

        let created = db
            .request_agent_creation_configured(
                RequestAgentCreationInput {
                    verified_email: owner_email.clone(),
                    workos_user_id: owner_workos.clone(),
                    display_name: format!("Relocation {suffix}"),
                    launch_code: launch_code(&db).await,
                    idempotency_key: format!("relocation-{suffix}-create"),
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
        let creation = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: runner.clone(),
                source_host_id: Some(source_host.clone()),
                lease_token: format!("{suffix}-create-lease"),
                lease_seconds: Some(300),
                runner_capacity: Some(capacity()),
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(creation.request.id, created.request.id);
        let old_secret = db
            .provision_runtime_credential(ProvisionRuntimeCredential {
                creation_request_id: creation.request.id.clone(),
                prepare_hosted_access: true,
                runner_id: runner.clone(),
                source_host_id: source_host.clone(),
                lease_token: format!("{suffix}-create-lease"),
            })
            .await
            .unwrap()
            .secret;
        db.register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
            request_id: creation.request.id.clone(),
            runner_id: runner.clone(),
            lease_token: format!("{suffix}-create-lease"),
            source_host_id: source_host.clone(),
            source_machine_id: source_machine.clone(),
            runtime_artifact_id: Some(format!("relocation-{suffix}-v1")),
            state_schema_version: Some("state-v1".into()),
            provider_runtime_handle: None,
            contact_endpoint: Some("http://127.0.0.1:4200/contact".into()),
            runtime_capabilities: Some(capabilities()),
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
                lease_token: format!("{suffix}-create-lease"),
                source_host_id: source_host.clone(),
                source_machine_id: source_machine.clone(),
                runtime_artifact_id: Some(format!("relocation-{suffix}-v1")),
                state_schema_version: Some("state-v1".into()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://127.0.0.1:4200/contact".into()),
                runtime_capabilities: Some(capabilities()),
                display_name: None,
                hostname: None,
                runtime_host: Some(source_host.clone()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: None,
                hermes_available: Some(true),
                published_app_urls: vec![],
                agent_npub: Some(format!("npub1{}", "r".repeat(58))),
                now: None,
            })
            .await
            .unwrap();
        let project_id = completed.project.id;
        let runtime_id = completed.request.agent_runtime_id.clone().unwrap();
        let initial_hosted = db.hosted_access(&runtime_id, &owner_workos).await.unwrap();
        assert!(initial_hosted.enrolled && initial_hosted.enabled);
        db.report_hosted(&old_secret, HostedReport {
            generation: initial_hosted.generation,
            status: ApplyStatus::Applied,
        }).await.unwrap();
        let origins_json = if same_host {
            format!("{{\"{source_host}\":\"https://agent.example.test\"}}")
        } else {
            format!(
                "{{\"{source_host}\":\"https://agent.example.test\",\"{target_host}\":\"https://agent.example.test\"}}"
            )
        };
        let origins = crate::hosted_hermes::HostedHermesOrigins::from_json(&origins_json).unwrap();
        artifact(&db, &format!("relocation-{suffix}-v2"), '2').await;

        let stop = db
            .request_runtime_stop(RequestRuntimeStopInput {
                verified_email: owner_email.clone(),
                workos_user_id: owner_workos.clone(),
                project_id: project_id.clone(),
                now: None,
            })
            .await
            .unwrap();
        let stop_lease = db
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: runner.clone(),
                lease_token: format!("{suffix}-stop-lease"),
                lease_seconds: Some(300),
                source_host_id: Some(source_host.clone()),
                runner_capacity: Some(capacity()),
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stop_lease.request.id, stop.id);
        db.complete_runtime_control_request(CompleteRuntimeControlRequestInput {
            request_id: stop.id,
            runner_id: runner.clone(),
            lease_token: format!("{suffix}-stop-lease"),
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

        if unbound_predecessor {
            db.connection()
                .await
                .unwrap()
                .execute(
                    "UPDATE runtime_core_credentials
                     SET agent_runtime_id=NULL, revoked=TRUE
                     WHERE creation_request_id=$1",
                    &[&creation.request.id],
                )
                .await
                .unwrap();
        }

        let relocation = db
            .admin_request_runtime_relocate_exact(AdminRuntimeRelocateExactInput {
                admin_verified_email: admin_email.clone(),
                admin_workos_user_id: admin_workos.clone(),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: source_host.clone(),
                expected_source_machine_id: source_machine.clone(),
                target_source_host_id: target_host.clone(),
                expected_agent_npub: format!("npub1{}", "r".repeat(58)),
                durable_state_manifest_sha256: "a".repeat(64),
                operator_observed_compute_absent: same_host,
                now: None,
            })
            .await
            .unwrap();
        let relocation_lease_token = format!("{suffix}-relocation-lease");
        let legacy_capacity: RunnerLeaseCapacity = serde_json::from_value(serde_json::json!({
            "runnerClasses": ["kata"], "runtimeCapabilities": capabilities()
        })).unwrap();
        assert!(!legacy_capacity.supports_relocation_credentials);
        for runner_capacity in [None, Some(legacy_capacity)] {
            let legacy_lease = db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: format!("{runner}-legacy"), source_host_id: Some(target_host.clone()),
                lease_token: format!("{suffix}-legacy-relocation-lease"), lease_seconds: Some(300),
                runner_capacity, now: None,
            }).await.unwrap();
            assert!(legacy_lease.is_none());
        }
        let relocation_lease = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: runner.clone(),
                source_host_id: Some(target_host.clone()),
                lease_token: relocation_lease_token.clone(),
                lease_seconds: Some(300),
                runner_capacity: Some(capacity()),
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(relocation_lease.request.id, relocation.id);

        if unbound_predecessor {
            let rejected = db
                .provision_relocation_credential(ProvisionRuntimeCredential {
                    creation_request_id: relocation.id,
                    prepare_hosted_access: false,
                    runner_id: runner,
                    source_host_id: target_host,
                    lease_token: relocation_lease_token,
                })
                .await;
            assert!(rejected.is_err());
            return;
        }

        // This is the regression boundary: a replacement incarnation must
        // receive a new credential before it can register and complete.
        let replacement_secret = db
            .provision_relocation_credential(ProvisionRuntimeCredential {
                creation_request_id: relocation.id.clone(),
                prepare_hosted_access: false,
                runner_id: runner.clone(),
                source_host_id: target_host.clone(),
                lease_token: relocation_lease_token.clone(),
            })
            .await
            .unwrap()
            .unwrap()
            .secret;
        assert!(old_secret != replacement_secret);

        // Change owner intent AFTER provisioning: completion must copy the
        // latest state, including deliberate disablement, not a launch snapshot.
        let latest_hosted = if same_host {
            // Previously applied state must become pending in the new process.
            db.hosted_access(&runtime_id, &owner_workos).await.unwrap()
        } else {
            db.set_hosted_access(&runtime_id, &owner_workos, SetHostedAccess {
                enabled: false, expected_generation: initial_hosted.generation,
            }).await.unwrap()
        };
        let latest_native = db.connection().await.unwrap().query_one(
            "SELECT hosted_username,hosted_password,hosted_signing_secret FROM runtime_core_credentials WHERE creation_request_id=$1",
            &[&creation.request.id],
        ).await.unwrap();

        db.register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
            request_id: relocation.id.clone(),
            runner_id: runner.clone(),
            lease_token: relocation_lease_token.clone(),
            source_host_id: target_host.clone(),
            source_machine_id: source_machine.clone(),
            runtime_artifact_id: Some(format!("relocation-{suffix}-v1")),
            state_schema_version: Some("state-v1".into()),
            provider_runtime_handle: None,
            contact_endpoint: Some("http://127.0.0.1:4200/contact".into()),
            runtime_capabilities: Some(capabilities()),
            display_name: None,
            hostname: None,
            runtime_host: Some(target_host.clone()),
            runtime_status: Some(RuntimeSummaryStatus::Unknown),
            active_inference_profile: None,
            hermes_available: Some(true),
            published_app_urls: vec![],
            now: None,
        })
        .await
        .unwrap();
        db.complete_agent_creation_request(CompleteAgentCreationRequestInput {
            request_id: relocation.id.clone(),
            runner_id: runner.clone(),
            lease_token: relocation_lease_token,
            source_host_id: target_host.clone(),
            source_machine_id: source_machine.clone(),
            runtime_artifact_id: Some(format!("relocation-{suffix}-v1")),
            state_schema_version: Some("state-v1".into()),
            provider_runtime_handle: None,
            contact_endpoint: Some("http://127.0.0.1:4200/contact".into()),
            runtime_capabilities: Some(capabilities()),
            display_name: None,
            hostname: None,
            runtime_host: Some(target_host.clone()),
            runtime_status: Some(RuntimeSummaryStatus::Online),
            active_inference_profile: None,
            hermes_available: Some(true),
            published_app_urls: vec![],
            agent_npub: Some(format!("npub1{}", "r".repeat(58))),
            now: None,
        })
        .await
        .unwrap();

        assert!(
            db.authenticate_runtime_credential(&old_secret)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            db.authenticate_runtime_credential(&replacement_secret)
                .await
                .unwrap()
                .is_some()
        );
        let relocated_hosted = db.hosted_access(&runtime_id, &owner_workos).await.unwrap();
        assert_eq!(relocated_hosted.enrolled, initial_hosted.enrolled);
        assert_eq!(relocated_hosted.enabled, latest_hosted.enabled);
        assert_eq!(relocated_hosted.generation, latest_hosted.generation);
        assert_eq!(relocated_hosted.apply_status, "pending");
        assert_eq!(relocated_hosted.applied_generation, None);
        assert!(db.hosted_route_targets_for_host(&target_host).await.unwrap().is_empty());
        let relocated_desired = db
            .hosted_desired(&replacement_secret, &origins)
            .await
            .unwrap()
            .unwrap();
        assert!(
            relocated_desired.username == latest_native.get::<_, Option<String>>(0)
                && relocated_desired.password == latest_native.get::<_, Option<String>>(1)
                && relocated_desired.signing_secret == latest_native.get::<_, Option<String>>(2)
        );
        db.report_hosted(
            &replacement_secret,
            HostedReport {
                generation: latest_hosted.generation,
                status: ApplyStatus::Applied,
            },
        )
        .await
        .unwrap();
        let routes = db.hosted_route_targets_for_host(&target_host).await.unwrap();
        assert_eq!(routes.len(), usize::from(latest_hosted.enabled));
        if latest_hosted.enabled { assert_eq!(routes[0].runtime_id, runtime_id); }
        let old_row = db
            .connection()
            .await
            .unwrap()
            .query_one(
                "SELECT revoked FROM runtime_core_credentials WHERE token_sha256=$1",
                &[&super::super::digest(&old_secret)],
            )
            .await
            .unwrap();
        assert!(old_row.get::<_, bool>(0));

        let upgrade = db
            .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                admin_verified_email: admin_email,
                admin_workos_user_id: admin_workos,
                project_id,
                target_runtime_artifact_id: format!("relocation-{suffix}-v2"),
                now: None,
            })
            .await
            .unwrap();
        let upgrade_lease_token = format!("{suffix}-upgrade-lease");
        let upgrade_lease = db
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: runner.clone(),
                lease_token: upgrade_lease_token.clone(),
                lease_seconds: Some(300),
                source_host_id: Some(target_host.clone()),
                runner_capacity: Some(capacity()),
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(upgrade_lease.request.id, upgrade.id);
        // A matching host alone cannot authorize a credential whose recorded
        // relocation belongs to a different machine.
        let client = db.connection().await.unwrap();
        client.execute(
            "UPDATE agent_creation_requests SET relocation_spec=jsonb_set(relocation_spec, '{relocation,sourceMachineId}', to_jsonb('different-machine'::text)) WHERE id=$1",
            &[&relocation.id],
        ).await.unwrap();
        let rejected = db.provision_upgrade_credential(ProvisionUpgradeCredential {
            request_id: upgrade.id.clone(), prepare_hosted_access: false,
            runner_id: runner.clone(), lease_token: upgrade_lease_token.clone(),
            source_host_id: target_host.clone(),
        }).await;
        assert!(matches!(rejected, Err(crate::CoreError::ProviderOperationTransitionConflict)));
        client.execute(
            "UPDATE agent_creation_requests SET relocation_spec=jsonb_set(relocation_spec, '{relocation,sourceMachineId}', to_jsonb($2::text)) WHERE id=$1",
            &[&relocation.id, &source_machine],
        ).await.unwrap();
        drop(client);
        let upgrade_secret = db
            .provision_upgrade_credential(ProvisionUpgradeCredential {
                request_id: upgrade.id.clone(),
                prepare_hosted_access: false,
                runner_id: runner.clone(),
                lease_token: upgrade_lease_token.clone(),
                source_host_id: target_host.clone(),
            })
            .await
            .unwrap()
            .secret;
        assert!(upgrade_secret == replacement_secret);
        db.complete_runtime_control_request(CompleteRuntimeControlRequestInput {
            request_id: upgrade.id,
            runner_id: runner,
            lease_token: upgrade_lease_token,
            runtime_artifact_id: Some(format!("relocation-{suffix}-v2")),
            state_schema_version: Some("state-v1".into()),
            runtime_capabilities: Some(capabilities()),
            runtime_host: Some(target_host),
            published_app_urls: Some(vec!["http://127.0.0.1:4200/contact".into()]),
            retirement_snapshot: None,
            now: None,
        })
        .await
        .unwrap();
        assert!(
            db.authenticate_runtime_credential(&replacement_secret)
                .await
                .unwrap()
                .is_some()
        );
    })
    .await;
}

#[tokio::test]
async fn relocation_replaces_credential_and_preserves_later_upgrade_same_host() {
    run_relocation_credential_handoff(true, false).await;
}

#[tokio::test]
async fn relocation_replaces_credential_and_preserves_later_upgrade_cross_host() {
    run_relocation_credential_handoff(false, false).await;
}

#[tokio::test]
async fn relocation_rejects_missing_predecessor_with_credential_history() {
    run_relocation_credential_handoff(false, true).await;
}
