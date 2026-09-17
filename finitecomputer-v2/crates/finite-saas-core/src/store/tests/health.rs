use super::*;

/// Runner-ferried standing readiness (2026-08 audit synthesis, H1 slice
/// 3): a report writes the runtime row's latest-report columns scoped to
/// the runner credential's host, and the admin overview projects
/// ready / not_ready(+reason) / unknown(stale) at read time — no sweeper.
#[tokio::test]
async fn postgres_runtime_health_reports_record_scope_and_project() {
    with_isolated_postgres(|store| async move {
        let run = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let host = format!("health-report-host-{run}");
        let launch_code = issue_test_launch_code(&store, "2026-08-24T12:00:00Z").await;
        store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: format!("artifact-health-report-{run}"),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/finite-agent-runtime:health-report@sha256:{}",
                    "7".repeat(64)
                ),
                version_label: "health-report-v1".to_string(),
                source_git_sha: None,
                finitec_version: None,
                hermes_source_ref: None,
                finite_platform_plugin_ref: None,
                state_schema_version: "state-v1".to_string(),
                base_image: Some("python:3.11-trixie".to_string()),
                recover_known_good_chat: false,
                promoted: true,
                now: None,
            })
            .await
            .unwrap();
        let created = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: format!("health-report-owner-{run}@finite.vip"),
                workos_user_id: format!("workos_health_report_owner_{run}"),
                display_name: "Health Report Agent".to_string(),
                launch_code,
                idempotency_key: format!("health-report-{run}"),
                now: None,
            })
            .await
            .unwrap();
        let lease = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: format!("runner-health-{run}"),
                source_host_id: None,
                lease_token: format!("lease-health-{run}"),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: None,
            })
            .await
            .unwrap()
            .expect("health report request should lease");
        assert_eq!(lease.request.id, created.request.id);
        let completed = store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: lease.request.id.clone(),
                runner_id: format!("runner-health-{run}"),
                lease_token: format!("lease-health-{run}"),
                source_host_id: host.clone(),
                source_machine_id: format!("health-report-agent-{run}"),
                runtime_artifact_id: Some(format!("artifact-health-report-{run}")),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://127.0.0.1:41001/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Health Report Agent".to_string()),
                hostname: None,
                runtime_host: Some("http://127.0.0.1:41001".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: None,
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                agent_npub: None,
                now: None,
            })
            .await
            .unwrap();
        let runtime_id = completed.request.agent_runtime_id.clone().unwrap();
        async fn overview_health(
            store: &CoreStore,
            runtime_id: &str,
        ) -> crate::RuntimeHealthProjection {
            store
                .admin_runtime_overviews()
                .await
                .unwrap()
                .into_iter()
                .find(|overview| overview.agent_runtime_id == runtime_id)
                .unwrap()
                .runtime_health
        }
        let report = |ready: bool, reason: Option<&str>, observed_at: &str, now: Option<&str>| {
            RecordRuntimeHealthReportInput {
                source_host_id: host.clone(),
                agent_runtime_id: runtime_id.clone(),
                ready,
                reason: reason.map(str::to_string),
                observed_at: observed_at.to_string(),
                agent_npub: Some(format!("npub1{}", "q".repeat(58))),
                report_interval_seconds: Some(60),
                now: now.map(str::to_string),
            }
        };

        // No report yet: the named unknown state, never a frozen ready.
        let health = overview_health(&store, &runtime_id).await;
        assert_eq!(health.status, crate::RuntimeHealthStatus::Unknown);
        assert_eq!(health.reported_at, None);

        // A fresh ready report projects ready with the pinned npub as
        // anti-squat evidence.
        let ack = store
            .record_runtime_health_report(report(true, None, "2026-08-24T11:59:00Z", None))
            .await
            .unwrap();
        assert_eq!(ack.agent_runtime_id, runtime_id);
        let health = overview_health(&store, &runtime_id).await;
        assert_eq!(health.status, crate::RuntimeHealthStatus::Ready);
        assert_eq!(
            health.agent_npub.as_deref(),
            Some(format!("npub1{}", "q".repeat(58)).as_str())
        );

        // A fresh not-ready report surfaces its reason.
        store
            .record_runtime_health_report(report(
                false,
                Some("model endpoint 503"),
                "2026-08-24T12:00:00Z",
                None,
            ))
            .await
            .unwrap();
        let health = overview_health(&store, &runtime_id).await;
        assert_eq!(health.status, crate::RuntimeHealthStatus::NotReady);
        assert_eq!(health.reason.as_deref(), Some("model endpoint 503"));

        // A report recorded long ago (runner stopped reporting) crosses
        // the 3x cadence deadline and projects the named stale state.
        store
            .record_runtime_health_report(report(
                true,
                None,
                "2020-01-01T00:00:00Z",
                Some("2020-01-01T00:00:00Z"),
            ))
            .await
            .unwrap();
        let health = overview_health(&store, &runtime_id).await;
        assert_eq!(health.status, crate::RuntimeHealthStatus::Stale);
        assert_eq!(health.report_interval_seconds, Some(60));

        // Existing state written by the previous Core: a row latched
        // `pending_first_report` alongside a fresh ready report from the
        // incarnation before the control. Migration 0024 rewrites it to
        // the current representation in one statement — `online` with
        // the report cleared — so the derived status is `unknown` until
        // the poller reports, never `online` off the old report. The
        // pin stays (same runtime, same principal).
        let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        let raw_handle = tokio::spawn(async move {
            let _ = connection.await;
        });
        raw.execute(
            "UPDATE agent_runtimes
                 SET host_facts = jsonb_set(host_facts, '{runtime_status}',
                                            to_jsonb('pending_first_report'::text)),
                     health_reported_at = now(),
                     health_observed_at = now(),
                     health_ready = TRUE,
                     health_reason = NULL,
                     health_report_interval_seconds = 60
                 WHERE id = $1",
            &[&runtime_id],
        )
        .await
        .unwrap();
        raw.batch_execute(include_str!(
            "../../../migrations/0024_runtime_status_pending_first_report_remap.sql"
        ))
        .await
        .unwrap();
        // Reapplying (every Core startup does) is a no-op.
        raw.batch_execute(include_str!(
            "../../../migrations/0024_runtime_status_pending_first_report_remap.sql"
        ))
        .await
        .unwrap();
        drop(raw);
        raw_handle.abort();
        let migrated = store
            .admin_runtime_overviews()
            .await
            .unwrap()
            .into_iter()
            .find(|overview| overview.agent_runtime_id == runtime_id)
            .unwrap();
        assert_eq!(migrated.lifecycle_status, RuntimeSummaryStatus::Online);
        assert_eq!(migrated.runtime_status, RuntimeSummaryStatus::Unknown);
        assert_eq!(
            migrated.runtime_health,
            crate::RuntimeHealthProjection {
                agent_npub: Some(format!("npub1{}", "q".repeat(58))),
                ..crate::RuntimeHealthProjection::unreported()
            }
        );
        assert!(
            CORE_SCHEMA_SQL
                .contains("WHERE host_facts->>'runtime_status' = 'pending_first_report'"),
            "the remap must ship in the startup schema concat"
        );

        // Scope: the credential's host guards the write. Another host's
        // runtime id and an unknown id both fail closed as not-found.
        let wrong_host = RecordRuntimeHealthReportInput {
            source_host_id: format!("other-host-{run}"),
            ..report(true, None, "2026-08-24T12:01:00Z", None)
        };
        assert!(matches!(
            store.record_runtime_health_report(wrong_host).await,
            Err(CoreError::ProjectRuntimeNotFound)
        ));
        let unknown_runtime = RecordRuntimeHealthReportInput {
            agent_runtime_id: format!("runtime-missing-{run}"),
            ..report(true, None, "2026-08-24T12:01:00Z", None)
        };
        assert!(matches!(
            store.record_runtime_health_report(unknown_runtime).await,
            Err(CoreError::ProjectRuntimeNotFound)
        ));

        // Bounded fields reject out-of-shape reports.
        let bad_npub = RecordRuntimeHealthReportInput {
            agent_npub: Some("not-an-npub".to_string()),
            ..report(true, None, "2026-08-24T12:01:00Z", None)
        };
        assert!(matches!(
            store.record_runtime_health_report(bad_npub).await,
            Err(CoreError::InvalidRuntimeHealthReport)
        ));
        let bad_interval = RecordRuntimeHealthReportInput {
            report_interval_seconds: Some(86_400),
            ..report(true, None, "2026-08-24T12:01:00Z", None)
        };
        assert!(matches!(
            store.record_runtime_health_report(bad_interval).await,
            Err(CoreError::InvalidRuntimeHealthReport)
        ));
        let long_reason = RecordRuntimeHealthReportInput {
            reason: Some("x".repeat(crate::MAX_RUNTIME_HEALTH_REPORT_REASON_CHARS + 1)),
            ..report(false, None, "2026-08-24T12:01:00Z", None)
        };
        assert!(matches!(
            store.record_runtime_health_report(long_reason).await,
            Err(CoreError::InvalidRuntimeHealthReport)
        ));
        let bad_observed = RecordRuntimeHealthReportInput {
            observed_at: "not-a-time".to_string(),
            ..report(true, None, "2026-08-24T12:01:00Z", None)
        };
        assert!(matches!(
            store.record_runtime_health_report(bad_observed).await,
            Err(CoreError::InvalidTimestamp)
        ));
    })
    .await;
}

/// The migration runs inside CORE_SCHEMA_SQL at every Core startup, so
/// reapplying it against an already-migrated database must be a no-op.
#[tokio::test]
async fn postgres_runtime_health_reports_migration_reapplies_cleanly() {
    with_isolated_postgres(|db| async move {
        let (raw, connection) = tokio_postgres::connect(&db.url, NoTls).await.unwrap();
        let handle = tokio::spawn(async move {
            let _ = connection.await;
        });
        raw.batch_execute(include_str!(
            "../../../migrations/0022_runtime_health_reports.sql"
        ))
        .await
        .unwrap();
        raw.batch_execute(include_str!(
            "../../../migrations/0022_runtime_health_reports.sql"
        ))
        .await
        .unwrap();
        drop(raw);
        handle.abort();
    })
    .await;
}
