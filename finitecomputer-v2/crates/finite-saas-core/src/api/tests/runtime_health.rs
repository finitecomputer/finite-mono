use super::*;

/// The runner-ferried standing-readiness endpoint is runner-authed, scopes
/// every write to the credential's source host, and feeds the admin
/// runtime overview's read-time health projection (H1 slice 3).
#[tokio::test]
async fn core_api_runtime_health_reports_are_runner_authed_and_host_scoped() {
    with_isolated_postgres(|db| async move {
        let store = db.store.clone();
        let auth = core_auth_with_runner_credentials(
            "core-token",
            vec![
                runner_credential_config(
                    "runner-health-a-current",
                    "runner-health-a-token",
                    "runner-health-a",
                    &[RunnerClass::Kata],
                    "health-host-a",
                    false,
                ),
                runner_credential_config(
                    "runner-health-b-current",
                    "runner-health-b-token",
                    "runner-health-b",
                    &[RunnerClass::Kata],
                    "health-host-b",
                    false,
                ),
            ],
            "usage-token",
        );
        let app = router(store.clone(), auth);

        store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: "artifact-health-api-v1".to_string(),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/finite-agent-runtime:health-api@sha256:{}",
                    "9".repeat(64)
                ),
                version_label: "health-api-v1".to_string(),
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
        let launch_code = issue_test_launch_code(&store).await;
        store
            .request_agent_creation(crate::RequestAgentCreationInput {
                verified_email: "health-api-owner@finite.vip".to_string(),
                workos_user_id: "workos_health_api_owner".to_string(),
                display_name: "Health Api Agent".to_string(),
                launch_code,
                idempotency_key: "health-api-1".to_string(),
                now: None,
            })
            .await
            .unwrap();
        let lease = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-health-b".to_string(),
                source_host_id: None,
                lease_token: "lease-health-b".to_string(),
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
                runner_id: "runner-health-b".to_string(),
                lease_token: "lease-health-b".to_string(),
                source_host_id: "health-host-b".to_string(),
                source_machine_id: "health-api-agent-001".to_string(),
                runtime_artifact_id: Some("artifact-health-api-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://127.0.0.1:41002/contact".to_string()),
                runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                    RuntimeCapabilitiesV1 {
                        restart: true,
                        recover_known_good_chat: false,
                        runtime_upgrade: true,
                        stop: true,
                        runtime_retirement: false,
                    },
                )),
                display_name: Some("Health Api Agent".to_string()),
                hostname: None,
                runtime_host: Some("http://127.0.0.1:41002".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: None,
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                agent_npub: None,
                now: None,
            })
            .await
            .unwrap();
        let runtime_id = completed.request.agent_runtime_id.unwrap();
        let runner_b = || {
            vec![(
                "authorization".to_string(),
                "Bearer runner-health-b-token".to_string(),
            )]
        };
        let report_body = |ready: bool, reason: Option<&str>| {
            serde_json::json!({
                "agentRuntimeId": runtime_id,
                "ready": ready,
                "reason": reason,
                "observedAt": "2026-08-24T12:00:00Z",
                "agentNpub": format!("npub1{}", "q".repeat(58)),
                "reportIntervalSeconds": 60
            })
        };

        // Unauthenticated and out-of-scope reports are rejected; the
        // in-scope runner records and is answered with the ack.
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/runtime-health-reports",
            &[],
            Some(report_body(true, None)),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/runtime-health-reports",
            &[(
                "authorization".to_string(),
                "Bearer runner-health-a-token".to_string(),
            )],
            Some(report_body(true, None)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "a runner must not report for another host's runtime"
        );
        let (status, ack) = send_json(
            &app,
            "POST",
            "/api/core/v1/runtime-health-reports",
            &runner_b(),
            Some(report_body(true, None)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ack["agentRuntimeId"], runtime_id);

        // A report presenting another principal than the one on record
        // (pinned by the first report above) is refused, not recorded.
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/runtime-health-reports",
            &runner_b(),
            Some(serde_json::json!({
                "agentRuntimeId": runtime_id,
                "ready": true,
                "observedAt": "2026-08-24T12:00:30Z",
                "agentNpub": format!("npub1{}", "z".repeat(58)),
                "reportIntervalSeconds": 60
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        // The poll-target listing is runner-authed and scoped to the
        // credential's host the same way.
        let (status, _) = send_json(
            &app,
            "GET",
            "/api/core/v1/runtime-health-targets",
            &[],
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, targets) = send_json(
            &app,
            "GET",
            "/api/core/v1/runtime-health-targets",
            &[(
                "authorization".to_string(),
                "Bearer runner-health-a-token".to_string(),
            )],
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(targets["sourceHostId"], "health-host-a");
        assert_eq!(targets["targets"], serde_json::json!([]));
        let (status, targets) = send_json(
            &app,
            "GET",
            "/api/core/v1/runtime-health-targets",
            &runner_b(),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(targets["sourceHostId"], "health-host-b");
        let target = targets["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|target| target["agentRuntimeId"] == runtime_id)
            .expect("the reporting host's runtime is listed");
        assert_eq!(target["sourceMachineId"], "health-api-agent-001");
        assert_eq!(target["contactEndpoint"], "http://127.0.0.1:41002/contact");
        assert_eq!(target["agentNpub"], format!("npub1{}", "q".repeat(58)));
        assert_eq!(target["lifecycleStatus"], "online");
        assert_eq!(target["reportIntervalSeconds"], 60);

        // Out-of-shape bodies fail closed.
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/runtime-health-reports",
            &runner_b(),
            Some(serde_json::json!({
                "agentRuntimeId": runtime_id,
                "ready": true,
                "observedAt": "not-a-time"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // The admin overview projects the fresh report at read time.
        let (status, overviews) = send_json(
            &app,
            "GET",
            "/api/core/v1/admin/runtimes",
            &operator_identity_headers("health-api-admin@finite.vip"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let overview = overviews
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["agent_runtime_id"] == runtime_id)
            .expect("the seeded runtime should appear in the admin overview");
        assert_eq!(overview["runtime_health"]["status"], "ready");
        assert_eq!(
            overview["runtime_health"]["agent_npub"],
            format!("npub1{}", "q".repeat(58))
        );

        // A fresh not-ready report surfaces its reason in the projection.
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/runtime-health-reports",
            &runner_b(),
            Some(report_body(false, Some("unreachable"))),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, overviews) = send_json(
            &app,
            "GET",
            "/api/core/v1/admin/runtimes",
            &operator_identity_headers("health-api-admin@finite.vip"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let overview = overviews
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["agent_runtime_id"] == runtime_id)
            .unwrap();
        assert_eq!(overview["runtime_health"]["status"], "not_ready");
        assert_eq!(overview["runtime_health"]["reason"], "unreachable");
    })
    .await;
}
