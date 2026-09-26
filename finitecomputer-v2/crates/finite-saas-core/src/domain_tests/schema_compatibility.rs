use super::*;

/// Assert the three independent encodings of one enum agree.
macro_rules! assert_wire_encodings_agree {
    ($parse:path, $($variant:expr),+ $(,)?) => {
        $({
            let value = $variant;
            let encoded = serde_json::to_value(value).unwrap();
            let encoded = encoded
                .as_str()
                .unwrap_or_else(|| panic!("{value:?} does not serialize to a JSON string"));
            assert_eq!(
                encoded,
                value.as_str(),
                "serde and as_str disagree for {value:?}",
            );
            assert_eq!(
                $parse(value.as_str()),
                Some(value),
                "{} rejects the as_str it must round-trip for {value:?}",
                stringify!($parse),
            );
        })+
    };
}

/// TIMESTAMPTZ stores six fractional digits, so any stamp Core generates
/// with more would round on write and no longer round-trip byte-for-byte.
/// On macOS the clock ticks in microseconds and this can never fire; on
/// Linux (CI, production) nanosecond stamps are real.
#[test]
fn current_time_iso_never_exceeds_postgres_microsecond_precision() {
    for _ in 0..1_000 {
        let stamp = current_time_iso().unwrap();
        let parsed = parse_time(&stamp).unwrap();
        assert_eq!(parsed.nanosecond() % 1_000, 0, "{stamp}");
    }
}

/// No summary status value exists that a runner one release behind does
/// not already parse, and a reader one release behind survives the next
/// added variant anyway.
#[test]
fn runtime_summary_status_wire_is_n_minus_one_safe_and_forward_tolerant() {
    const N_MINUS_ONE_KNOWN: [&str; 4] = ["online", "offline", "stale", "unknown"];
    for status in [
        RuntimeSummaryStatus::Online,
        RuntimeSummaryStatus::Offline,
        RuntimeSummaryStatus::Stale,
        RuntimeSummaryStatus::Unknown,
    ] {
        assert!(
            N_MINUS_ONE_KNOWN.contains(&status.as_str()),
            "{status:?} would not parse on an N-1 runner"
        );
    }
    // An unrecognised string parses as `unknown`, via both surfaces.
    assert_eq!(
        parse_runtime_summary_status("pending_second_report"),
        Some(RuntimeSummaryStatus::Unknown)
    );
    assert_eq!(
        serde_json::from_str::<RuntimeSummaryStatus>("\"pending_second_report\"").unwrap(),
        RuntimeSummaryStatus::Unknown
    );
    // Known strings still parse precisely, and the wire shape is unchanged.
    assert_eq!(
        serde_json::from_str::<RuntimeSummaryStatus>("\"stale\"").unwrap(),
        RuntimeSummaryStatus::Stale
    );
    assert_eq!(
        serde_json::to_string(&RuntimeSummaryStatus::Stale).unwrap(),
        "\"stale\""
    );
    assert!(serde_json::from_str::<RuntimeSummaryStatus>("7").is_err());
    // Enums without a safe fallback stay strict.
    assert_eq!(parse_runtime_health_status("bogus"), None);
    assert!(serde_json::from_str::<RuntimeHealthStatus>("\"bogus\"").is_err());
}

/// `wire_enum!` now generates serde, `as_str`, and `parse_*` from one
/// variant list, so the three cannot drift by construction. This keeps
/// checking them because the guarantee depends on serde's `rename`
/// behaving as assumed, and because an enum added outside the macro would
/// otherwise reintroduce the three hand-written surfaces unnoticed.
#[test]
fn enum_serde_as_str_and_parse_encodings_agree() {
    use BillingClass::*;
    assert_wire_encodings_agree!(parse_billing_class, Grandfathered, Sponsored, Standard);

    use BillingSubscriptionStatus::*;
    assert_wire_encodings_agree!(
        parse_billing_subscription_status,
        Incomplete,
        IncompleteExpired,
        Trialing,
        BillingSubscriptionStatus::Active,
        PastDue,
        Canceled,
        Unpaid,
        Paused,
    );

    assert_wire_encodings_agree!(
        parse_user_link_status,
        UserLinkStatus::Pending,
        UserLinkStatus::Linked,
    );

    assert_wire_encodings_agree!(
        parse_project_membership_role,
        ProjectMembershipRole::Owner,
        ProjectMembershipRole::Admin,
        ProjectMembershipRole::Member,
    );

    assert_wire_encodings_agree!(
        parse_runtime_summary_status,
        RuntimeSummaryStatus::Online,
        RuntimeSummaryStatus::Offline,
        RuntimeSummaryStatus::Stale,
        RuntimeSummaryStatus::Unknown,
    );

    assert_wire_encodings_agree!(
        parse_offboarding_phase,
        OffboardingPhase::RetirementRequested,
        OffboardingPhase::ReceiptVerified,
        OffboardingPhase::ComputeRemoved,
        OffboardingPhase::LinkDeactivated,
        OffboardingPhase::Archived,
    );

    assert_wire_encodings_agree!(
        parse_runtime_health_status,
        RuntimeHealthStatus::Ready,
        RuntimeHealthStatus::NotReady,
        RuntimeHealthStatus::Stale,
        RuntimeHealthStatus::Unknown,
    );

    assert_wire_encodings_agree!(parse_runtime_artifact_kind, RuntimeArtifactKind::OciImage);

    assert_wire_encodings_agree!(
        parse_hosting_tier,
        HostingTier::Standard,
        HostingTier::Confidential,
    );

    assert_wire_encodings_agree!(
        parse_runtime_resource_class,
        RuntimeResourceClass::Vcpu4Memory8Gib,
        RuntimeResourceClass::Vcpu2Memory4Gib,
    );

    assert_wire_encodings_agree!(
        parse_runner_class,
        RunnerClass::LocalDocker,
        RunnerClass::AppleContainer,
        RunnerClass::Kata,
        RunnerClass::Phala,
        RunnerClass::Enclavia,
    );

    assert_wire_encodings_agree!(
        parse_runtime_control_kind,
        RuntimeControlKind::Restart,
        RuntimeControlKind::RecoverKnownGoodChatRuntime,
        RuntimeControlKind::Upgrade,
        RuntimeControlKind::Stop,
        RuntimeControlKind::Destroy,
    );

    assert_wire_encodings_agree!(
        parse_runtime_control_request_status,
        RuntimeControlRequestStatus::Requested,
        RuntimeControlRequestStatus::Launching,
        RuntimeControlRequestStatus::ComputeUp,
        RuntimeControlRequestStatus::Ready,
        RuntimeControlRequestStatus::Succeeded,
        RuntimeControlRequestStatus::Stopped,
        RuntimeControlRequestStatus::Failed,
    );

    assert_wire_encodings_agree!(
        parse_runtime_lifecycle_stage,
        RuntimeLifecycleStage::Launch,
        RuntimeLifecycleStage::Compute,
        RuntimeLifecycleStage::Readiness,
        RuntimeLifecycleStage::Retirement,
        RuntimeLifecycleStage::Unknown,
    );

    assert_wire_encodings_agree!(
        parse_agent_creation_request_status,
        AgentCreationRequestStatus::Requested,
        AgentCreationRequestStatus::Launching,
        AgentCreationRequestStatus::Running,
        AgentCreationRequestStatus::Failed,
        AgentCreationRequestStatus::Cancelled,
    );

    assert_wire_encodings_agree!(
        parse_finite_private_grant_status,
        FinitePrivateGrantStatus::Active,
        FinitePrivateGrantStatus::Revoked,
    );

    assert_wire_encodings_agree!(
        parse_finite_private_api_key_status,
        FinitePrivateApiKeyStatus::Active,
        FinitePrivateApiKeyStatus::Revoked,
    );

    assert_wire_encodings_agree!(
        parse_finite_private_reservation_status,
        FinitePrivateReservationStatus::Reserved,
        FinitePrivateReservationStatus::Settled,
        FinitePrivateReservationStatus::Denied,
    );

    assert_wire_encodings_agree!(
        parse_finite_private_settlement_kind,
        FinitePrivateSettlementKind::Actual,
        FinitePrivateSettlementKind::Estimate,
    );
}

/// LEGACY-ROW CONTRACT. The existing-host import bridge is deleted, but
/// production may still hold rows from its 2026-07 near-ship test run.
/// This test plants those rows the way the bridge left them (raw SQL —
/// the writing machinery is gone; see git history for the original
/// reconcile/claim code) and pins the two behaviors that keep them inert:
///
/// 1. A project linked to an import candidate stays out of user-facing
///    project lists (`public_visible_projects` filters on
///    `import_candidate_id`).
/// 2. Its capability-less runtime refuses every runtime control
///    (`supports_runtime_control` fails closed on NULL capabilities).
///
/// A future importer must define its own linkage and lifecycle rather
/// than resurrecting these rows' semantics.
#[tokio::test]
async fn legacy_import_rows_stay_hidden_and_refuse_runtime_controls() {
    with_isolated_postgres(|db| async move {
        let owner = db
            .link_verified_user(LinkVerifiedUserInput {
                verified_email: "paul@finite.vip".to_string(),
                workos_user_id: "user_workos_paul".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        let owner_id = owner.id.clone();
        // link_verified_user already provisioned the personal org; the
        // bridge reused it the same way.
        let org_id = db
            .query_json(
                "SELECT to_jsonb(t.id) FROM customer_orgs t WHERE t.owner_user_id = $1",
                &[&owner_id],
            )
            .await[0]
            .as_str()
            .unwrap()
            .to_string();
        for statement in [
            format!(
                "INSERT INTO project_import_candidates \
                 (id, source_host_id, source_machine_id, source_import_key, owner_email, \
                  pending_user_id, customer_org_id, status, project_id, agent_runtime_id, \
                  claimed_by_user_id, host_facts, created_at, updated_at) \
                 VALUES ('candidate-legacy', 'box1', 'paul-smoke', 'box1:paul-smoke', \
                  'paul@finite.vip', '{owner_id}', '{org_id}', 'claimed', 'project-legacy', \
                  'runtime-legacy', '{owner_id}', \
                  '{{\"display_name\": \"Paul Smoke\", \"hostname\": null, \"runtime_host\": \"box1\", \
                    \"runtime_status\": \"online\", \"active_inference_profile\": null, \
                    \"hermes_available\": null, \"published_app_urls\": []}}', '{NOW}', '{NOW}')"
            ),
            format!(
                "INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, \
                  import_candidate_id, created_at, updated_at) \
                 VALUES ('project-legacy', '{org_id}', '{owner_id}', 'Paul Smoke', \
                  'candidate-legacy', '{NOW}', '{NOW}')"
            ),
            format!(
                "INSERT INTO agent_runtimes (id, project_id, source_host_id, source_machine_id, \
                  source_import_key, host_facts, created_at, updated_at) \
                 VALUES ('runtime-legacy', 'project-legacy', 'box1', 'paul-smoke', \
                  'box1:paul-smoke', \
                  '{{\"display_name\": \"Paul Smoke\", \"hostname\": null, \"runtime_host\": \"box1\", \
                    \"runtime_status\": \"online\", \"active_inference_profile\": null, \
                    \"hermes_available\": null, \"published_app_urls\": []}}', '{NOW}', '{NOW}')"
            ),
            format!(
                "INSERT INTO project_runtime_links (id, project_id, agent_runtime_id, active, created_at) \
                 VALUES ('link-legacy', 'project-legacy', 'runtime-legacy', TRUE, '{NOW}')"
            ),
            // The bridge granted the claiming user a hosted-web owner
            // membership; visibility reads flow through these rows.
            format!(
                "INSERT INTO chat_identities (id, user_id, kind, device_id, created_at) \
                 VALUES ('identity-legacy', '{owner_id}', 'hosted_web', 'dashboard-bridge-v1', '{NOW}')"
            ),
            format!(
                "INSERT INTO project_room_memberships (id, project_id, chat_identity_id, role, created_at) \
                 VALUES ('membership-legacy', 'project-legacy', 'identity-legacy', 'owner', '{NOW}')"
            ),
        ] {
            db.exec(&statement).await;
        }

        let visible = db
            .visible_projects_for_workos_user("user_workos_paul")
            .await
            .unwrap();
        let legacy = visible
            .iter()
            .find(|candidate| candidate.project.id == "project-legacy")
            .expect("the legacy project row is still readable internally");
        assert_eq!(
            legacy.project.import_candidate_id.as_deref(),
            Some("candidate-legacy"),
            "the import linkage that keeps this row hidden must survive reads"
        );

        let error = db
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "paul@finite.vip".to_string(),
                workos_user_id: "user_workos_paul".to_string(),
                project_id: "project-legacy".to_string(),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap_err();
        assert!(matches!(error, CoreError::RuntimeControlUnsupported));
    })
    .await;
}

#[test]
fn schema_is_postgres_first_and_contains_first_bridge_tables() {
    for table in [
        "users",
        "customer_orgs",
        // Written only by the deleted existing-host import bridge; the
        // table stays because production may hold rows from its 2026-07
        // test run and dropping schema is a rollback boundary.
        "project_import_candidates",
        "projects",
        "runtime_artifacts",
        "agent_runtimes",
        "runtime_relay_credentials",
        "project_runtime_links",
        "chat_identities",
        "project_room_memberships",
        // Writer removed; the table stays because production may hold rows
        // and dropping schema is a rollback boundary (separate gated
        // migration).
        "runtime_status_snapshots",
        "inference_profiles",
        "agent_creation_entitlements",
        "agent_creation_requests",
        "customer_billing_accounts",
        "finite_private_limit_profiles",
        "finite_private_grants",
        "finite_private_api_keys",
        "finite_private_admin_audit_events",
        "finite_private_reservations",
        "finite_private_daily_resets",
        "finite_private_notice_claims",
        "runner_capacity_fences",
    ] {
        assert!(CORE_SCHEMA_SQL.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")));
    }

    assert!(CORE_SCHEMA_SQL.contains("JSONB"));
    assert!(CORE_SCHEMA_SQL.contains("TIMESTAMPTZ"));
    assert!(CORE_SCHEMA_SQL.contains("finite-private-generous-v2"));
    assert!(CORE_SCHEMA_SQL.contains("200000000"));
    assert!(CORE_SCHEMA_SQL.contains(FINITE_PRIVATE_5X_LIMIT_PROFILE));
    assert!(CORE_SCHEMA_SQL.contains("500000000"));
    assert!(CORE_SCHEMA_SQL.contains("weekly_limit_units = NULL"));
    assert!(!CORE_SCHEMA_SQL.to_lowercase().contains("sqlite"));
    // Operator-rescue scripts stay out of CORE_SCHEMA_SQL by construction:
    // CORE_SCHEMA_SQL is an explicit concat! allowlist and each rescue
    // lives in its own const, so Core startup can never mutate user state
    // outside the migration ladder. Keyed on each rescue's audit action,
    // which appears nowhere else in the schema.
    assert!(!CORE_SCHEMA_SQL.contains("runtime.upgrade.rollback_rescue"));
    assert!(!CORE_SCHEMA_SQL.contains("runtime.lifecycle.reverse_remap"));
}

#[test]
fn expand_domain_reads_old_rows_and_n_minus_one_ignores_new_fields() {
    let old_project: Project = serde_json::from_value(json!({
        "id": "project-old",
        "customer_org_id": "org-old",
        "owner_user_id": "user-old",
        "display_name": "Old Agent",
        "import_candidate_id": null,
        "created_at": NOW,
        "updated_at": NOW
    }))
    .unwrap();
    assert_eq!(old_project.hosting_tier, None);
    assert_eq!(old_project.placement, None);

    let new_project = Project {
        hosting_tier: Some(HostingTier::Standard),
        placement: Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
        ..old_project
    };
    #[derive(Deserialize)]
    struct NMinusOneProject {
        id: String,
        display_name: String,
    }
    let legacy: NMinusOneProject =
        serde_json::from_value(serde_json::to_value(&new_project).unwrap()).unwrap();
    assert_eq!(legacy.id, "project-old");
    assert_eq!(legacy.display_name, "Old Agent");

    let old_runtime: AgentRuntime = serde_json::from_value(json!({
        "id": "runtime-old",
        "project_id": "project-old",
        "source_host_id": "legacy-host",
        "source_machine_id": "legacy-machine",
        "source_import_key": "legacy-host:legacy-machine",
        "runtime_artifact_id": null,
        "state_schema_version": null,
        "host_facts": {
            "display_name": "Old Agent",
            "hostname": null,
            "runtime_host": "legacy-host",
            "runtime_status": "online",
            "active_inference_profile": null,
            "hermes_available": true,
            "published_app_urls": []
        },
        "created_at": NOW,
        "updated_at": NOW
    }))
    .unwrap();
    assert_eq!(old_runtime.placement, None);
    assert_eq!(old_runtime.provider_runtime_handle, None);
    assert!(old_runtime.provider_runtime_handle_history.is_empty());

    let new_runtime = AgentRuntime {
        placement: Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
        provider_runtime_handle: Some(ProviderRuntimeHandleEnvelope::V1(ProviderRuntimeHandleV1 {
            runner_class: RunnerClass::Kata,
            opaque: json!({"container": "finite-kata-old"}),
        })),
        provider_runtime_handle_history: vec![ProviderRuntimeHandleEnvelope::V1(
            ProviderRuntimeHandleV1 {
                runner_class: RunnerClass::Kata,
                opaque: json!({"container": "finite-kata-old"}),
            },
        )],
        contact_endpoint: Some("https://old.example.test/contact".to_string()),
        ..old_runtime
    };
    #[derive(Deserialize)]
    struct NMinusOneRuntime {
        id: String,
        source_host_id: String,
        source_machine_id: String,
    }
    let legacy_runtime: NMinusOneRuntime =
        serde_json::from_value(serde_json::to_value(new_runtime).unwrap()).unwrap();
    assert_eq!(legacy_runtime.id, "runtime-old");
    assert_eq!(legacy_runtime.source_host_id, "legacy-host");
    assert_eq!(legacy_runtime.source_machine_id, "legacy-machine");
}

#[test]
fn versioned_runtime_identity_envelopes_fail_closed_on_unknown_schema() {
    let unknown_spec = serde_json::from_value::<RuntimeSpecEnvelope>(json!({
        "schema": "runtime_spec.v2",
        "spec": {}
    }));
    assert!(unknown_spec.is_err());

    let n_minus_one_spec = serde_json::from_value::<RuntimeSpecEnvelope>(json!({
        "schema": "runtime_spec.v1",
        "spec": {
            "operationId": "agent-request-old",
            "projectId": "project-old",
            "agentRuntimeId": "runtime-old",
            "placement": {
                "runnerClass": "kata",
                "runtimeResourceClass": "vcpu4_memory8_gib"
            },
            "runtimeArtifactId": "artifact-v1",
            "runtimeImageDigest": format!(
                "ghcr.io/finitecomputer/agent-runtime:v1@sha256:{}",
                "a".repeat(64)
            ),
            "stateSchemaVersion": "state-v1",
            "durableStateId": "runtime-old",
            "endpoints": {
                "servicePort": 8080,
                "healthPath": "/healthz",
                "contactPath": "/contact"
            },
            "environment": {},
            "secretReferences": ["FINITE_PRIVATE_API_KEY"]
        }
    }))
    .unwrap();
    assert_eq!(
        runtime_spec_v1(&n_minus_one_spec).boot_intent,
        RuntimeBootIntent::Normal
    );

    let unknown_handle = serde_json::from_value::<ProviderRuntimeHandleEnvelope>(json!({
        "schema": "provider_runtime_handle.v2",
        "handle": {"runnerClass": "phala", "opaque": {}}
    }));
    assert!(unknown_handle.is_err());
    assert!(matches!(
        normalize_runtime_contact_endpoint(Some("file:///tmp/contact")),
        Err(CoreError::InvalidRuntimeContactEndpoint)
    ));
}
