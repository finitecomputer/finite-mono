use super::*;

/// Row-scoped runtime-control lifecycle against Postgres: restart drives the
/// runtime back Online, and destroy offboards it (link deactivated, relay
/// credential dropped, and every Finite Private key bound to the runtime or
/// project revoked) — all without the deleted full-state rewrite. Also
/// exercises the enqueue dedup and the source-host-partitioned control lease.
#[tokio::test]
async fn postgres_runtime_control_lifecycle_row_scoped() {
    with_isolated_postgres(|store| async move {
            let launch_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
            let run = "rc-lifecycle";
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
            store
                .request_agent_creation_configured(
                    RequestAgentCreationInput {
                        verified_email: email.clone(),
                        workos_user_id: workos.clone(),
                        display_name: "RC Agent".to_string(),
                        launch_code: launch_code.clone(),
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
                    runtime_capabilities: Some(kata_runtime_capabilities()),
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
            let unrelated_project_id = format!("project-unrelated-{run}");
            let unrelated_membership_id = format!("membership-unrelated-{run}");
            let (raw, raw_connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let raw_connection = tokio::spawn(async move {
                let _ = raw_connection.await;
            });
            raw.execute(
                "INSERT INTO projects (
                   id, customer_org_id, owner_user_id, display_name, created_at, updated_at
                 )
                 SELECT $2, customer_org_id, owner_user_id, 'Unrelated Agent',
                        CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
                 FROM projects WHERE id = $1",
                &[&project_id, &unrelated_project_id],
            )
            .await
            .unwrap();
            raw.execute(
                "INSERT INTO project_room_memberships (
                   id, project_id, chat_identity_id, role, created_at
                 )
                 SELECT $2, $3, chat_identity_id, role, CURRENT_TIMESTAMP
                 FROM project_room_memberships
                 WHERE project_id = $1 AND archived_at IS NULL
                 LIMIT 1",
                &[&project_id, &unrelated_membership_id, &unrelated_project_id],
            )
            .await
            .unwrap();
            drop(raw);
            raw_connection.abort();
            let visible_before_destroy = store
                .visible_projects_for_workos_user(&workos)
                .await
                .unwrap()
                .into_iter()
                .map(|visible| visible.project.id)
                .collect::<BTreeSet<_>>();
            assert_eq!(
                visible_before_destroy,
                BTreeSet::from([project_id.clone(), unrelated_project_id.clone()])
            );

            let exact_artifact_retry = UpsertRuntimeArtifactInput {
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
            };
            store
                .upsert_runtime_artifact(exact_artifact_retry.clone())
                .await
                .unwrap();
            let mut material_mutation = exact_artifact_retry;
            material_mutation.version_label = "mutated-in-place".to_string();
            assert!(matches!(
                store
                    .upsert_runtime_artifact(material_mutation)
                    .await
                    .unwrap_err(),
                CoreError::RuntimeArtifactImmutable
            ));

            // Restart: enqueue is deduped (same in-flight request), leased only by
            // the runtime's own source host, and completion drives it Online.
            let restart = store
                .request_runtime_restart(RequestRuntimeRestartInput {
                    verified_email: email.clone(),
                    workos_user_id: workos.clone(),
                    project_id: project_id.clone(),
                    now: None,
                })
                .await
                .unwrap();
            let restart_again = store
                .request_runtime_restart(RequestRuntimeRestartInput {
                    verified_email: email.clone(),
                    workos_user_id: workos.clone(),
                    project_id: project_id.clone(),
                    now: None,
                })
                .await
                .unwrap();
            assert_eq!(restart.id, restart_again.id, "enqueue must dedup in-flight");

            // A runner on a DIFFERENT host must not claim this request.
            let other_host_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("runner-other-{run}"),
                    lease_token: format!("ctl-other-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some("someotherhost".to_string()),
                    runner_capacity: None,
                    now: None,
                })
                .await
                .unwrap();
            assert!(other_host_lease.is_none(), "partitioned by source host");

            let wrong_class_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("phala-runner-{run}"),
                    lease_token: format!("ctl-phala-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some(host.to_string()),
                    runner_capacity: Some(crate::RunnerLeaseCapacity {
                        runner_classes: vec![crate::RunnerClass::Phala],
                        ..crate::RunnerLeaseCapacity::default()
                    }),
                    now: None,
                })
                .await
                .unwrap();
            assert!(
                wrong_class_lease.is_none(),
                "Phala worker must not claim Kata control work"
            );

            let unspecified_class_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("unspecified-runner-{run}"),
                    lease_token: format!("ctl-unspecified-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some(host.to_string()),
                    runner_capacity: Some(crate::RunnerLeaseCapacity::default()),
                    now: None,
                })
                .await
                .unwrap();
            assert!(
                unspecified_class_lease.is_none(),
                "empty advertised class set supports nothing"
            );

            let control_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("ctl-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some(host.to_string()),
                    runner_capacity: Some(crate::RunnerLeaseCapacity {
                        draining: true,
                        runner_classes: vec![crate::RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..crate::RunnerLeaseCapacity::default()
                    }),
                    now: None,
                })
                .await
                .unwrap()
                .expect("draining host runner should still lease its own control request");
            assert_eq!(control_lease.request.id, restart.id);
            // The running incarnation reports ready before the restart; the
            // completion below must clear that report (it spoke for the
            // previous incarnation) while keeping the principal it pinned.
            store
                .record_runtime_health_report(RecordRuntimeHealthReportInput {
                    source_host_id: host.to_string(),
                    agent_runtime_id: runtime_id.clone(),
                    ready: true,
                    reason: None,
                    observed_at: current_time_iso().unwrap(),
                    agent_npub: Some(format!("npub1{}", "q".repeat(58))),
                    report_interval_seconds: Some(60),
                    now: None,
                })
                .await
                .unwrap();
            assert_eq!(
                store
                    .admin_runtime_overviews()
                    .await
                    .unwrap()
                    .into_iter()
                    .find(|o| o.agent_runtime_id == runtime_id)
                    .unwrap()
                    .runtime_status,
                RuntimeSummaryStatus::Online
            );
            store
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: restart.id.clone(),
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("ctl-{run}"),
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
            let overview_online = store
                .admin_runtime_overviews()
                .await
                .unwrap()
                .into_iter()
                .find(|o| o.agent_runtime_id == runtime_id)
                .unwrap();
            // Completion is the runner's bounded readiness wait, not a
            // standing observation: the lifecycle latch says the restart
            // succeeded, the stored report is cleared, and the user-facing
            // status stays the named unknown state until the poller reports.
            assert_eq!(overview_online.lifecycle_status, RuntimeSummaryStatus::Online);
            assert_eq!(
                overview_online.runtime_health,
                crate::RuntimeHealthProjection {
                    agent_npub: Some(format!("npub1{}", "q".repeat(58))),
                    ..crate::RuntimeHealthProjection::unreported()
                },
                "the pre-restart report is cleared; the pinned principal is kept"
            );
            assert_eq!(
                overview_online.lifecycle_status,
                store
                    .agent_runtime(&runtime_id)
                    .await
                    .unwrap()
                    .host_facts
                    .runtime_status
            );
            assert_eq!(overview_online.runtime_status, RuntimeSummaryStatus::Unknown);
            assert!(overview_online.runtime_link_active);
            // The restarted runtime is a standing-health target for its own
            // host's runner (and for nobody else's).
            let targets = store.runtime_health_targets_for_host(host).await.unwrap();
            assert_eq!(targets.source_host_id, host);
            let target = targets
                .targets
                .iter()
                .find(|target| target.agent_runtime_id == runtime_id)
                .expect("restarted runtime should be a health target");
            assert_eq!(target.source_machine_id, machine);
            assert_eq!(target.lifecycle_status, RuntimeSummaryStatus::Online);
            assert_eq!(
                target.agent_npub.as_deref(),
                Some(format!("npub1{}", "q".repeat(58)).as_str()),
                "the attribution pin survives a restart of the same runtime"
            );
            assert!(
                store
                    .runtime_health_targets_for_host("someotherhost")
                    .await
                    .unwrap()
                    .targets
                    .is_empty()
            );
            // A report presenting another principal is refused, not recorded:
            // a reallocated port never wears this runtime's name.
            assert!(matches!(
                store
                    .record_runtime_health_report(RecordRuntimeHealthReportInput {
                        source_host_id: host.to_string(),
                        agent_runtime_id: runtime_id.clone(),
                        ready: true,
                        reason: None,
                        observed_at: current_time_iso().unwrap(),
                        agent_npub: Some(format!("npub1{}", "z".repeat(58))),
                        report_interval_seconds: Some(60),
                        now: None,
                    })
                    .await,
                Err(CoreError::RuntimeHealthReportPrincipalMismatch)
            ));
            // A fresh ready report is what makes the derived status online.
            store
                .record_runtime_health_report(RecordRuntimeHealthReportInput {
                    source_host_id: host.to_string(),
                    agent_runtime_id: runtime_id.clone(),
                    ready: true,
                    reason: None,
                    observed_at: current_time_iso().unwrap(),
                    agent_npub: Some(format!("npub1{}", "q".repeat(58))),
                    report_interval_seconds: Some(60),
                    now: None,
                })
                .await
                .unwrap();
            let overview_online = store
                .admin_runtime_overviews()
                .await
                .unwrap()
                .into_iter()
                .find(|o| o.agent_runtime_id == runtime_id)
                .unwrap();
            assert_eq!(overview_online.lifecycle_status, RuntimeSummaryStatus::Online);
            assert_eq!(overview_online.runtime_status, RuntimeSummaryStatus::Online);
            assert_eq!(
                overview_online.runtime_health.status,
                crate::RuntimeHealthStatus::Ready
            );

            // Upgrade: target is an explicit promoted, digest-pinned artifact;
            // the lease carries it and completion updates artifact/endpoint
            // facts without offboarding the Runtime or revoking its key.
            store
                .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                    id: "artifact-rc-v2".to_string(),
                    kind: RuntimeArtifactKind::OciImage,
                    reference: format!(
                        "ghcr.io/finitecomputer/agent-runtime:v2@sha256:{}",
                        "b".repeat(64)
                    ),
                    version_label: "v2".to_string(),
                    source_git_sha: Some("git-v2".to_string()),
                    finitec_version: None,
                    hermes_source_ref: Some(
                        "nix:packages.x86_64-linux.hermes-agent-runtime".to_string(),
                    ),
                    finite_platform_plugin_ref: Some("plugin-v2".to_string()),
                    state_schema_version: "state-v1".to_string(),
                    base_image: None,
                    canary_runtime_id: None,
                    recover_known_good_chat: true,
                    promoted: true,
                    now: None,
                })
                .await
                .unwrap();
            let changed_binding = store
                .admin_request_runtime_upgrade_exact(AdminRuntimeUpgradeExactInput {
                    admin_verified_email: format!("admin-{run}@finite.vip"),
                    admin_workos_user_id: format!("admin-workos-{run}"),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: "runtime-replaced-after-plan".to_string(),
                    expected_source_host_id: host.to_string(),
                    expected_source_machine_id: machine.to_string(),
                    target_runtime_artifact_id: "artifact-rc-v2".to_string(),
                    now: None,
                })
                .await
                .unwrap_err();
            assert!(matches!(changed_binding, CoreError::RuntimeSpecMismatch));
            let upgrade = store
                .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                    admin_verified_email: format!("admin-{run}@finite.vip"),
                    admin_workos_user_id: format!("admin-workos-{run}"),
                    project_id: project_id.clone(),
                    target_runtime_artifact_id: "artifact-rc-v2".to_string(),
                    now: None,
                })
                .await
                .unwrap();
            assert_eq!(
                store.runtime_control_request(&upgrade.id).await.unwrap(),
                upgrade,
                "operator polling reads the exact persisted request"
            );
            let conflicting_stop = store
                .request_runtime_stop(RequestRuntimeStopInput {
                    verified_email: email.clone(),
                    workos_user_id: workos.clone(),
                    project_id: project_id.clone(),
                    now: None,
                })
                .await
                .unwrap_err();
            assert!(matches!(
                conflicting_stop,
                CoreError::RuntimeControlOperationConflict
            ));

            let (raw, raw_connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let raw_connection = tokio::spawn(async move {
                let _ = raw_connection.await;
            });
            raw.execute(
                "UPDATE runtime_artifacts
                 SET retired_at = GREATEST(clock_timestamp(), promoted_at)
                 WHERE id = 'artifact-rc-v2'",
                &[],
            )
            .await
            .unwrap();
            raw.execute(
                "INSERT INTO agent_runtimes (
                   id, project_id, source_host_id, source_machine_id, source_import_key,
                   runtime_artifact_id, state_schema_version,
                   placement_runner_class, runtime_resource_class, runtime_capabilities,
                   host_facts, created_at, updated_at
                 )
                 SELECT 'runtime-healthy-behind-poison', project_id, source_host_id,
                        'healthy-behind-poison', 'rchost/healthy-behind-poison',
                        runtime_artifact_id, state_schema_version,
                        placement_runner_class, runtime_resource_class, runtime_capabilities,
                        host_facts,
                        CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
                 FROM agent_runtimes WHERE id = $1",
                &[&runtime_id],
            )
            .await
            .unwrap();
            raw.execute(
                "INSERT INTO runtime_control_requests (
                   id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                   requested_by_user_id, kind, status, created_at, updated_at
                 )
                 SELECT 'runtime_ctl_healthy_behind_poison', $1,
                        'runtime-healthy-behind-poison', $2, 'healthy-behind-poison',
                        owner_user_id, 'restart', 'requested',
                        CURRENT_TIMESTAMP + INTERVAL '1 second',
                        CURRENT_TIMESTAMP + INTERVAL '1 second'
                 FROM projects WHERE id = $1",
                &[&project_id, &host],
            )
            .await
            .unwrap();
            let healthy_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("ctl-retired-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some(host.to_string()),
                    runner_capacity: Some(crate::RunnerLeaseCapacity {
                        runner_classes: vec![crate::RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..crate::RunnerLeaseCapacity::default()
                    }),
                    now: None,
                })
                .await
                .unwrap()
                .expect("poisoned upgrade must not starve a healthy request");
            assert_eq!(
                healthy_lease.request.id,
                "runtime_ctl_healthy_behind_poison"
            );
            let poisoned = raw
                .query_one(
                    "SELECT status, failure_message
                     FROM runtime_control_requests WHERE id = $1",
                    &[&upgrade.id],
                )
                .await
                .unwrap();
            assert_eq!(poisoned.get::<_, String>("status"), "failed");
            assert!(
                poisoned
                    .get::<_, Option<String>>("failure_message")
                    .unwrap_or_default()
                    .contains("retired")
            );
            raw.execute(
                "UPDATE runtime_artifacts SET retired_at = NULL WHERE id = 'artifact-rc-v2'",
                &[],
            )
            .await
            .unwrap();
            raw.execute(
                "UPDATE agent_creation_requests SET runtime_spec = NULL
                 WHERE agent_runtime_id = $1",
                &[&runtime_id],
            )
            .await
            .unwrap();
            let upgrade_store = store
                .store
                .clone()
                .with_runtime_environment(BTreeMap::from([(
                    "FINITE_BRAIN_SERVER_URL".to_string(),
                    "https://brain.finite.computer".to_string(),
                )]))
                .unwrap()
                .with_runtime_secret_references(vec![
                    "FAL_KEY".to_string(),
                    "XAI_API_KEY".to_string(),
                ])
                .unwrap();
            let upgrade = store
                .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                    admin_verified_email: format!("admin-{run}@finite.vip"),
                    admin_workos_user_id: format!("admin-workos-{run}"),
                    project_id: project_id.clone(),
                    target_runtime_artifact_id: "artifact-rc-v2".to_string(),
                    now: Some("2026-07-10T12:00:01Z".to_string()),
                })
                .await
                .unwrap();
            let upgrade_lease = upgrade_store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("ctl-upgrade-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some(host.to_string()),
                    runner_capacity: Some(crate::RunnerLeaseCapacity {
                        runner_classes: vec![crate::RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..crate::RunnerLeaseCapacity::default()
                    }),
                    now: None,
                })
                .await
                .unwrap()
                .expect("upgrade should lease");
            assert_eq!(upgrade_lease.request.id, upgrade.id);
            assert_eq!(
                upgrade_lease
                    .target_runtime_artifact
                    .as_ref()
                    .map(|artifact| artifact.id.as_str()),
                Some("artifact-rc-v2")
            );
            assert_eq!(
                runtime_spec_v1(upgrade_lease.runtime_spec.as_ref().unwrap()).secret_references,
                vec!["FINITE_PRIVATE_API_KEY", "FAL_KEY", "XAI_API_KEY"]
            );
            assert_eq!(
                runtime_spec_v1(upgrade_lease.runtime_spec.as_ref().unwrap()).environment,
                BTreeMap::from([(
                    "FINITE_BRAIN_SERVER_URL".to_string(),
                    "https://brain.finite.computer".to_string(),
                )])
            );
            raw.execute(
                "UPDATE runtime_artifacts
                 SET retired_at = GREATEST(clock_timestamp(), promoted_at)
                 WHERE id = 'artifact-rc-v2'",
                &[],
            )
            .await
            .unwrap();
            upgrade_store
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: upgrade.id.clone(),
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("ctl-upgrade-{run}"),
                    runtime_artifact_id: Some("artifact-rc-v2".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                        RuntimeCapabilitiesV1 {
                            recover_known_good_chat: true,
                            runtime_retirement: true,
                            ..*kata_runtime_capabilities().v1()
                        },
                    )),
                    runtime_host: Some("http://127.0.0.1:41002".to_string()),
                    published_app_urls: Some(vec!["http://127.0.0.1:41002/contact".to_string()]),
                    retirement_snapshot: None,
                    now: None,
                })
                .await
                .unwrap();
            // The upgrade latched `online` (health cleared, nothing reported
            // yet). A runtime that dies in exactly this state is today's
            // recovery shape: it must be relocatable under the operator's
            // compute-absent attestation, and only under it.
            let relocate = |attested: bool| AdminRuntimeRelocateExactInput {
                admin_verified_email: format!("admin-{run}@finite.vip"),
                admin_workos_user_id: format!("admin-workos-{run}"),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: host.to_string(),
                expected_source_machine_id: machine.to_string(),
                target_source_host_id: format!("{host}-target"),
                expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                durable_state_manifest_sha256: "b".repeat(64),
                operator_observed_compute_absent: attested,
                now: None,
            };
            assert!(matches!(
                store.admin_request_runtime_relocate_exact(relocate(false)).await,
                Err(CoreError::RuntimeControlUnsupported)
            ));
            let relocation = store
                .admin_request_runtime_relocate_exact(relocate(true))
                .await
                .expect("a dead just-upgraded runtime relocates under attestation");
            assert_eq!(relocation.status, AgentCreationRequestStatus::Requested);
            // Withdraw it again: the rest of this test exercises the runtime
            // in place.
            store
                .cancel_agent_creation_request(CancelAgentCreationRequestInput {
                    request_id: relocation.id,
                    now: None,
                })
                .await
                .unwrap();
            let refreshed_capabilities: Value = raw
                .query_one(
                    "SELECT runtime_capabilities FROM agent_runtimes WHERE id = $1",
                    &[&runtime_id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(
                refreshed_capabilities["capabilities"]["recover_known_good_chat"],
                true
            );
            let refreshed_contact_endpoint: Option<String> = raw
                .query_one(
                    "SELECT contact_endpoint FROM agent_runtimes WHERE id = $1",
                    &[&runtime_id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(
                refreshed_contact_endpoint.as_deref(),
                Some("http://127.0.0.1:41002/contact")
            );
            // The creation request (not the cancelled relocation above).
            let upgraded_spec: Value = raw
                .query_one(
                    "SELECT runtime_spec FROM agent_creation_requests
                     WHERE agent_runtime_id = $1 AND relocation_spec IS NULL",
                    &[&runtime_id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(upgraded_spec["spec"]["runtimeArtifactId"], "artifact-rc-v2");
            assert_eq!(
                upgraded_spec["spec"]["secretReferences"],
                serde_json::json!(["FINITE_PRIVATE_API_KEY", "FAL_KEY", "XAI_API_KEY"])
            );
            assert_eq!(
                upgraded_spec["spec"]["environment"]["FINITE_BRAIN_SERVER_URL"],
                "https://brain.finite.computer"
            );
            assert_ne!(runtime_id, machine);
            assert_eq!(
                upgraded_spec["spec"]["durableStateId"], runtime_id,
                "legacy synthesis names the durable root by the Agent Runtime id, never the source machine"
            );
            drop(raw);
            raw_connection.abort();
            let upgraded = store
                .admin_runtime_overviews()
                .await
                .unwrap()
                .into_iter()
                .find(|overview| overview.agent_runtime_id == runtime_id)
                .unwrap();
            assert_eq!(
                upgraded.runtime_artifact_id.as_deref(),
                Some("artifact-rc-v2")
            );
            assert!(upgraded.runtime_link_active);
            let key_before_destroy = store
                .finite_private_admin_state()
                .await
                .unwrap()
                .api_keys
                .into_iter()
                .find(|key| key.id == provisioned.api_key.id)
                .unwrap();
            assert_eq!(key_before_destroy.status, FinitePrivateApiKeyStatus::Active);

            // Retirement requires a receipt bound to this exact request and
            // RuntimeSpec. Postgres stores that receipt and performs the
            // target-scoped offboarding in the same transaction.
            let changed_retirement_binding = store
                .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                    admin_verified_email: format!("admin-{run}@finite.vip"),
                    admin_workos_user_id: format!("admin-workos-{run}"),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: "runtime-replaced-after-review".to_string(),
                    expected_source_host_id: host.to_string(),
                    expected_source_machine_id: machine.to_string(),
                    now: Some("2026-07-10T12:03:50Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(
                changed_retirement_binding,
                CoreError::RuntimeSpecMismatch
            ));
            let destroy = store
                .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                    admin_verified_email: format!("admin-{run}@finite.vip"),
                    admin_workos_user_id: format!("admin-workos-{run}"),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: host.to_string(),
                    expected_source_machine_id: machine.to_string(),
                    now: Some("2026-07-10T12:04:00Z".to_string()),
                })
                .await
                .unwrap();
            let destroy_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("ctl-destroy-{run}"),
                    lease_seconds: Some(60),
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
                    now: Some("2026-07-10T12:04:10Z".to_string()),
                })
                .await
                .unwrap()
                .expect("retirement should lease to a capable Kata runner");
            assert_eq!(destroy_lease.request.id, destroy.id);
            let destroy_spec = runtime_spec_v1(destroy_lease.runtime_spec.as_ref().unwrap());
            let receipt = RuntimeRetirementSnapshotReceipt {
                schema: crate::RUNTIME_RETIREMENT_SNAPSHOT_SCHEMA.to_string(),
                request_id: destroy.id.clone(),
                project_id: project_id.clone(),
                agent_runtime_id: runtime_id.clone(),
                durable_state_id: destroy_spec.durable_state_id.clone(),
                runtime_artifact_id: destroy_spec.runtime_artifact_id.clone(),
                backend: crate::RUNTIME_RETIREMENT_BACKEND_BORG.to_string(),
                locator: crate::runtime_retirement_archive_locator(&destroy.id),
                zip_bytes: 8192,
                zip_sha256: "a".repeat(64),
                manifest_sha256: "b".repeat(64),
                created_at: "2026-07-10T12:04:20Z".to_string(),
                verified_at: "2026-07-10T12:04:30Z".to_string(),
                recovery_authority_id: "finite-assisted-test".to_string(),
                retention_policy: crate::RUNTIME_RETIREMENT_RETENTION_INDEFINITE.to_string(),
            };
            let completion = CompleteRuntimeControlRequestInput {
                request_id: destroy.id.clone(),
                runner_id: format!("runner-{run}"),
                lease_token: format!("ctl-destroy-{run}"),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: None,
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: Some(receipt.clone()),
                now: Some("2026-07-10T12:04:40Z".to_string()),
            };
            store
                .complete_runtime_control_request(completion.clone())
                .await
                .unwrap();
            store
                .complete_runtime_control_request(completion)
                .await
                .expect("identical Postgres completion replay must be idempotent");

            let (raw, raw_connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let raw_connection = tokio::spawn(async move {
                let _ = raw_connection.await;
            });
            let stored_snapshot = postgres_runtime_retirement_snapshot(&raw, &destroy.id)
                .await
                .unwrap()
                .expect("retirement receipt must be stored");
            assert_eq!(stored_snapshot.receipt, receipt);
            let active_link_count: i64 = raw
                .query_one(
                    "SELECT COUNT(*) FROM project_runtime_links
                     WHERE project_id = $1 AND agent_runtime_id = $2 AND active = TRUE",
                    &[&project_id, &runtime_id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(active_link_count, 0);
            drop(raw);
            raw_connection.abort();

            let visible_after = store
                .visible_projects_for_workos_user(&workos)
                .await
                .unwrap()
                .into_iter()
                .map(|visible| visible.project.id)
                .collect::<BTreeSet<_>>();
            assert_eq!(visible_after, BTreeSet::from([unrelated_project_id]));
            let key_after_destroy = store
                .finite_private_admin_state()
                .await
                .unwrap()
                .api_keys
                .into_iter()
                .find(|key| key.id == provisioned.api_key.id)
                .unwrap();
            assert_eq!(key_after_destroy.status, FinitePrivateApiKeyStatus::Revoked);
        })
        .await;
}
