use super::*;

#[tokio::test]
async fn runtime_upgrade_first_use_gate_is_fail_closed_without_blocking_restart() {
    with_isolated_postgres(|db| async move {
        let app = router_with_runtime_upgrades(db.store.clone(), test_auth(), false);
        let admin = operator_identity_headers("admin@finite.vip");
        let (upgrade_status, upgrade_body) = send_json(
            &app,
            "POST",
            "/api/core/v1/admin/projects/missing/runtime/upgrade",
            &admin,
            Some(serde_json::json!({ "targetRuntimeArtifactId": "artifact-v2" })),
        )
        .await;
        assert_eq!(upgrade_status, StatusCode::CONFLICT);
        assert!(
            upgrade_body["error"]
                .as_str()
                .unwrap_or_default()
                .contains("not enabled")
        );

        let (restart_status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/admin/projects/missing/runtime/restart",
            &admin,
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(restart_status, StatusCode::NOT_FOUND);
    })
    .await;
}

#[tokio::test]
async fn runtime_retirement_product_gate_is_independently_fail_closed() {
    with_isolated_postgres(|db| async move {
        let user = identity_headers("owner@finite.vip", "true");
        let disabled = router_with_runtime_features(db.store.clone(), test_auth(), true, false);
        let (disabled_status, disabled_body) = send_json(
            &disabled,
            "POST",
            "/api/core/v1/me/projects/missing/runtime/destroy",
            &user,
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(disabled_status, StatusCode::CONFLICT);
        assert!(
            disabled_body["error"]
                .as_str()
                .unwrap_or_default()
                .contains("not enabled")
        );

        let enabled = router_with_runtime_features(db.store.clone(), test_auth(), true, true);
        let (enabled_status, _) = send_json(
            &enabled,
            "POST",
            "/api/core/v1/me/projects/missing/runtime/destroy",
            &user,
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(enabled_status, StatusCode::NOT_FOUND);
    })
    .await;
}

#[tokio::test]
async fn core_api_admin_runtimes_and_runtime_control_feed_the_runner_queue() {
    with_isolated_postgres(|db| async move {
        let store = db.store.clone();
        let launch_code = issue_test_launch_code(&store).await;
        let app = admin_router(store);
        let (project_id, runtime_id) = provision_hosted_agent(&app, &launch_code).await;
        let admin = operator_identity_headers("admin@finite.vip");
        let service = [("authorization".to_string(), "Bearer core-token".to_string())];

        let (status, runtimes) =
            send_json(&app, "GET", "/api/core/v1/admin/runtimes", &admin, None).await;
        assert_eq!(status, StatusCode::OK);
        let runtimes = runtimes.as_array().unwrap().clone();
        assert_eq!(runtimes.len(), 1);
        let overview = &runtimes[0];
        assert_eq!(overview["project_id"], project_id.as_str());
        assert_eq!(overview["agent_runtime_id"], runtime_id.as_str());
        assert_eq!(overview["owner_email"], "owner@finite.vip");
        assert_eq!(overview["source_host_id"], "oslo-host-1");
        assert_eq!(overview["runtime_artifact_version_label"], "v1");
        assert_eq!(overview["runtime_status"], "unknown");
        assert_eq!(overview["lifecycle_status"], "online");
        assert_eq!(overview["runtime_health"]["status"], "unknown");
        assert_eq!(overview["runtime_capabilities"]["restart"], true);
        assert_eq!(
            overview["runtime_capabilities"]["recover_known_good_chat"],
            false
        );
        assert_eq!(overview["runtime_capabilities"]["runtime_upgrade"], true);
        assert_eq!(overview["runtime_capabilities"]["stop"], true);
        assert_eq!(
            overview["runtime_capabilities"]["runtime_retirement"],
            false
        );

        // Admin restart succeeds even though the admin does not own the project.
        let (status, restart) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/projects/{project_id}/runtime/restart"),
            &admin,
            Some(serde_json::json!({ "now": "2026-05-25T13:03:00Z" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(restart["kind"], "restart");
        assert_eq!(restart["status"], "requested");
        assert_eq!(restart["agent_runtime_id"], runtime_id.as_str());
        assert!(restart.get("lease_token").is_none());
        let restart_id = restart["id"].as_str().unwrap().to_string();

        // The runner consumes the admin-created request through the same lease
        // endpoint and shape as owner-created requests.
        let (status, lease) = send_json(
            &app,
            "POST",
            "/api/core/v1/runtime-control-requests/lease",
            &service,
            Some(serde_json::json!({
                "runnerId": "runner-oslo-1",
                "leaseToken": "restart-lease-1",
                "leaseSeconds": 60,
                "sourceHostId": "oslo-host-1",
                "now": "2026-05-25T13:04:00Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(lease["request"]["id"], restart_id.as_str());
        assert_eq!(lease["request"]["status"], "launching");
        assert_eq!(lease["runtime"]["source_machine_id"], "oslo-agent-001");

        let (status, completed) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/runtime-control-requests/{restart_id}/complete"),
            &service,
            Some(serde_json::json!({
                "runnerId": "runner-oslo-1",
                "leaseToken": "restart-lease-1",
                "now": "2026-05-25T13:05:00Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(completed["status"], "succeeded");

        let target_reference = format!(
            "ghcr.io/finitecomputer/agent-runtime:v2@sha256:{}",
            "b".repeat(64)
        );
        let (status, _) = send_json(
            &app,
            "PUT",
            "/api/core/v1/runtime-artifacts/artifact-v2",
            &service,
            Some(serde_json::json!({
                "kind": "oci_image",
                "reference": target_reference,
                "versionLabel": "v2",
                "stateSchemaVersion": "state-v1",
                "promoted": true,
                "now": "2026-05-25T13:05:10Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, upgrade) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/projects/{project_id}/runtime/upgrade"),
            &admin,
            Some(serde_json::json!({
                "targetRuntimeArtifactId": "artifact-v2",
                "now": "2026-05-25T13:05:20Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(upgrade["kind"], "upgrade");
        assert_eq!(upgrade["target_runtime_artifact_id"], "artifact-v2");
        let upgrade_id = upgrade["id"].as_str().unwrap();
        let (status, lease) = send_json(
            &app,
            "POST",
            "/api/core/v1/runtime-control-requests/lease",
            &service,
            Some(serde_json::json!({
                "runnerId": "runner-oslo-1",
                "leaseToken": "upgrade-lease-1",
                "leaseSeconds": 60,
                "sourceHostId": "oslo-host-1",
                "runnerCapacity": { "runnerClasses": ["kata"] },
                "now": "2026-05-25T13:05:30Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(lease["request"]["id"], upgrade_id);
        assert_eq!(lease["target_runtime_artifact"]["id"], "artifact-v2");
        // The restart just completed: the lifecycle latch is `online`
        // (health cleared until the poller reports), and the lease
        // carries a value every runner generation parses.
        assert_eq!(
            db.store
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .host_facts
                .runtime_status,
            RuntimeSummaryStatus::Online
        );
        assert_eq!(lease["runtime"]["host_facts"]["runtime_status"], "online");
        let parsed: crate::RuntimeControlLease = serde_json::from_value(lease.clone()).unwrap();
        assert_eq!(
            parsed.runtime.host_facts.runtime_status,
            RuntimeSummaryStatus::Online
        );
        let (status, upgraded) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/runtime-control-requests/{upgrade_id}/complete"),
            &service,
            Some(serde_json::json!({
                "runnerId": "runner-oslo-1",
                "leaseToken": "upgrade-lease-1",
                "runtimeArtifactId": "artifact-v2",
                "stateSchemaVersion": "state-v1",
                "runtimeHost": "http://127.0.0.1:41002",
                "publishedAppUrls": ["http://127.0.0.1:41002/contact"],
                "now": "2026-05-25T13:05:40Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(upgraded["status"], "succeeded");

        // Recovery remains disabled until it is more than a restart alias.
        let (status, _) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/projects/{project_id}/runtime/recover-known-good-chat"),
            &admin,
            Some(serde_json::json!({ "now": "2026-05-25T13:06:00Z" })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        // Both admin actions are audited with the admin's email as actor.
        let (status, events) = send_json(
            &app,
            "GET",
            "/api/core/v1/finite-private/admin-audit-events",
            &admin,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let events = events.as_array().unwrap().clone();
        let admin_actions = events
            .iter()
            .filter(|event| event["actor"] == "admin@finite.vip")
            .map(|event| event["action"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(admin_actions.contains(&"runtime.admin_restart".to_string()));
        assert!(admin_actions.contains(&"runtime.admin_upgrade".to_string()));
        assert!(!admin_actions.contains(&"runtime.admin_recover_known_good_chat".to_string()));
    })
    .await;
}
