use super::*;

#[test]
fn user_agent_creation_json_cannot_select_provider_placement() {
    let error = serde_json::from_value::<CreateAgentRequest>(serde_json::json!({
        "displayName": "Provider injection",
        "launchCode": "finite_test",
        "idempotencyKey": "provider-injection",
        "runnerClass": "phala"
    }))
    .expect_err("runnerClass must remain outside the user boundary");
    assert!(error.to_string().contains("unknown field `runnerClass`"));
}

#[test]
fn public_runtime_contact_prefers_explicit_normalized_endpoint_over_n_minus_one_urls() {
    let runtime = AgentRuntime {
        id: "runtime-public-contact".to_string(),
        project_id: "project-public-contact".to_string(),
        source_host_id: "internal-host".to_string(),
        source_machine_id: "internal-machine".to_string(),
        source_import_key: "internal-host:internal-machine".to_string(),
        runtime_artifact_id: None,
        state_schema_version: None,
        placement: None,
        provider_runtime_handle: None,
        provider_runtime_handle_history: Vec::new(),
        contact_endpoint: Some("https://contact.example.test/".to_string()),
        runtime_capabilities: None,
        host_facts: crate::HostOwnedRuntimeFacts {
            display_name: "Contact test".to_string(),
            hostname: None,
            runtime_host: "internal-host".to_string(),
            runtime_status: RuntimeSummaryStatus::Online,
            active_inference_profile: None,
            hermes_available: Some(true),
            published_app_urls: vec!["https://legacy.example.test/wrong".to_string()],
        },
        created_at: "2026-07-11T12:00:00Z".to_string(),
        updated_at: "2026-07-11T12:00:00Z".to_string(),
    };

    assert_eq!(
        public_runtime_contact_endpoint(&runtime).as_deref(),
        Some("https://contact.example.test")
    );
    let legacy_runtime = AgentRuntime {
        contact_endpoint: None,
        host_facts: crate::HostOwnedRuntimeFacts {
            published_app_urls: vec![
                "not-a-contact".to_string(),
                "https://legacy.example.test/contact/".to_string(),
            ],
            ..runtime.host_facts.clone()
        },
        ..runtime
    };
    assert_eq!(
        public_runtime_contact_endpoint(&legacy_runtime).as_deref(),
        Some("https://legacy.example.test/contact")
    );

    let unreported = RuntimeHealthProjection::unreported();
    let public_legacy = PublicAgentRuntime::project(legacy_runtime.clone(), &unreported);
    assert!(public_legacy.runtime_capabilities.is_none());
    // The lifecycle latch says online, but nothing has reported: the
    // derived status is the named unknown state and the latch stays
    // visible under its own name, with the health evidence alongside.
    assert_eq!(public_legacy.runtime_status, RuntimeSummaryStatus::Unknown);
    assert_eq!(public_legacy.lifecycle_status, RuntimeSummaryStatus::Online);
    let public_legacy_json = serde_json::to_value(&public_legacy).unwrap();
    assert!(public_legacy_json.get("runtime_capabilities").is_none());
    assert_eq!(public_legacy_json["runtime_status"], "unknown");
    assert_eq!(public_legacy_json["lifecycle_status"], "online");
    assert_eq!(public_legacy_json["runtime_health"]["status"], "unknown");
    // Older readers parse the new document (the added fields are
    // additive), and this reader parses a pre-health document.
    let pre_health: PublicAgentRuntime = serde_json::from_value(serde_json::json!({
        "id": "runtime-public-contact",
        "project_id": "project-public-contact",
        "contact_endpoint": null,
        "runtime_status": "online",
        "hermes_available": true,
        "created_at": "2026-07-11T12:00:00Z",
        "updated_at": "2026-07-11T12:00:00Z"
    }))
    .unwrap();
    assert_eq!(pre_health.runtime_status, RuntimeSummaryStatus::Online);
    assert_eq!(pre_health.runtime_health, None);

    let fresh_ready = RuntimeHealthProjection {
        status: RuntimeHealthStatus::Ready,
        reason: None,
        reported_at: Some("2026-07-11T12:00:00Z".to_string()),
        observed_at: Some("2026-07-11T11:59:59Z".to_string()),
        agent_npub: None,
        report_interval_seconds: Some(60),
    };
    let public_reported = PublicAgentRuntime::project(legacy_runtime.clone(), &fresh_ready);
    assert_eq!(public_reported.runtime_status, RuntimeSummaryStatus::Online);
    assert_eq!(
        public_reported.runtime_health,
        Some(PublicRuntimeHealth {
            status: RuntimeHealthStatus::Ready,
            reason: None,
            reported_at: Some("2026-07-11T12:00:00Z".to_string()),
            observed_at: Some("2026-07-11T11:59:59Z".to_string()),
            report_interval_seconds: Some(60),
        })
    );

    let public_current = PublicAgentRuntime::project(
        AgentRuntime {
            runtime_capabilities: Some(legacy_kata_runtime_capabilities()),
            ..legacy_runtime
        },
        &unreported,
    );
    assert_eq!(
        public_current.runtime_capabilities,
        Some(PublicRuntimeCapabilities {
            restart: true,
            recover_known_good_chat: false,
            runtime_upgrade: true,
            stop: true,
            runtime_retirement: false,
        })
    );
}

#[test]
fn runtime_control_request_view_redacts_runner_lease_fields() {
    let view = RuntimeControlRequestView::from(crate::RuntimeControlRequest {
        id: "runtime_ctl_123".to_string(),
        project_id: "project_123".to_string(),
        agent_runtime_id: "runtime_123".to_string(),
        source_host_id: "oslo-host-1".to_string(),
        source_machine_id: "oslo-agent-001".to_string(),
        requested_by_user_id: Some("user_123".to_string()),
        kind: crate::RuntimeControlKind::Destroy,
        target_runtime_artifact_id: None,
        status: crate::RuntimeControlRequestStatus::Launching,
        failure_stage: None,
        runner_id: Some("runner-1".to_string()),
        lease_token: Some("secret-lease-token".to_string()),
        lease_expires_at: Some("2026-05-25T13:10:00Z".to_string()),
        failure_message: None,
        created_at: "2026-05-25T13:00:00Z".to_string(),
        updated_at: "2026-05-25T13:00:00Z".to_string(),
        completed_at: None,
    });
    let json = serde_json::to_value(view).unwrap();

    assert!(json.get("runner_id").is_none());
    assert!(json.get("lease_token").is_none());
    assert!(json.get("lease_expires_at").is_none());
    assert_eq!(json["source_machine_id"], "oslo-agent-001");
}
