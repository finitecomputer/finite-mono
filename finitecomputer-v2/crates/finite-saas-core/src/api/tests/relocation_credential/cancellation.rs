use super::*;
use crate::RequestRuntimeStopInput;

async fn owner_stop(db: &TestDb, fixture: &PreparedRelocation) -> Option<crate::CoreError> {
    db.request_runtime_stop(RequestRuntimeStopInput {
        verified_email: format!("{}@finite.vip", fixture.run),
        workos_user_id: format!("api-relocation-{}-owner", fixture.run),
        project_id: fixture.project_id.clone(),
        now: None,
    })
    .await
    .err()
}

async fn cancel_over_http(app: &Router, request_id: &str) -> (StatusCode, serde_json::Value) {
    send_json(
        app,
        "POST",
        &format!("/api/core/v1/agent-creation-requests/{request_id}/cancel"),
        &[("authorization".to_string(), format!("Bearer {TOKEN}"))],
        Some(serde_json::json!({})),
    )
    .await
}

fn refused(error: &Option<crate::CoreError>) -> bool {
    matches!(
        error,
        Some(crate::CoreError::RuntimeControlOperationConflict)
    )
}

/// What an operator sees printed for a cancel result.
fn printed<T: serde::Serialize>(outcome: &T) -> serde_json::Value {
    serde_json::to_value(outcome).unwrap()
}

#[tokio::test]
async fn requested_relocation_cancel_reopens_runtime_controls() {
    with_isolated_postgres(|db| async move {
        let runtime = create_runtime(&db, "cancel-requested", true).await;
        let fixture = enqueue_relocation(&db, &runtime).await;
        let app = router(db.store.clone(), scoped_test_auth());
        assert!(refused(&owner_stop(&db, &fixture).await));

        // No provider work happened, so cancelling releases the Runtime at once.
        let (status, cancelled) = cancel_over_http(&app, &fixture.request_id).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cancelled["status"], "cancelled");
        let accepted = owner_stop(&db, &fixture).await;
        assert!(accepted.is_none(), "{accepted:?}");
    })
    .await;
}

#[tokio::test]
async fn cancelled_launching_relocation_holds_controls_until_its_runner_releases() {
    with_isolated_postgres(|db| async move {
        let fixture = prepare_relocation(&db, "cancel-launching", true).await;
        let predecessor = fixture.predecessor_secret.as_deref().unwrap();
        let app = router(db.store.clone(), scoped_test_auth());
        let lease = "api-relocation-cancel-launching-relocation-lease";
        let (status, successor) =
            provision_relocation_over_http(&app, &fixture.request_id, lease).await;
        assert_eq!(status, StatusCode::OK);
        let successor_secret = successor["secret"].as_str().unwrap().to_string();
        // Registration is the Runner's report that target compute booted.
        register_relocation(&db, &fixture, lease).await;
        let predecessor_state = current_credential_state(&db, &fixture.runtime_id).await;

        let (status, cancelled) = cancel_over_http(&app, &fixture.request_id).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cancelled["status"], "cancelled");
        // Core keeps the lease, but the service response never carries it.
        assert!(cancelled["lease_token"].is_null(), "{cancelled}");
        assert!(!cancelled.to_string().contains(lease), "{cancelled}");
        // The target Runner still owns running target compute, so the source
        // stays closed to controls, even once the lease has expired.
        let held = owner_stop(&db, &fixture).await;
        assert!(
            refused(&held),
            "a cancelled relocation must hold source controls until its target is released: {held:?}"
        );
        expire_creation_lease(&db, &fixture.request_id).await;
        assert!(refused(&owner_stop(&db, &fixture).await));
        let (status, second_cancel) = cancel_over_http(&app, &fixture.request_id).await;
        assert_eq!(status, StatusCode::CONFLICT, "{second_cancel}");
        let another = db
            .admin_request_runtime_relocate_exact(crate::AdminRuntimeRelocateExactInput {
                admin_verified_email: "cancel-launching-admin@finite.vip".into(),
                admin_workos_user_id: "api-relocation-cancel-launching-admin".into(),
                project_id: fixture.project_id.clone(),
                expected_agent_runtime_id: fixture.runtime_id.clone(),
                expected_source_host_id: fixture.source_host.clone(),
                expected_source_machine_id: fixture.source_machine.clone(),
                target_source_host_id: fixture.target_host.clone(),
                expected_agent_npub: format!("npub1{}", "a".repeat(58)),
                durable_state_manifest_sha256: "d".repeat(64),
                operator_observed_compute_absent: true,
                now: None,
            })
            .await
            .err();
        assert!(refused(&another), "{another:?}");

        // The Runner learns of the cancel from its next Core call.
        assert_eq!(
            complete_relocation_over_http(&app, &fixture, lease).await,
            StatusCode::CONFLICT
        );

        // After proving its target is down, the Runner's failure record
        // releases the lease and the source reopens.
        assert_eq!(
            fail_relocation_over_http(&app, &fixture, lease).await,
            StatusCode::OK
        );
        assert_eq!(creation_status(&db, &fixture.request_id).await, "cancelled");
        let accepted = owner_stop(&db, &fixture).await;
        assert!(accepted.is_none(), "{accepted:?}");
        assert_eq!(
            credential_state_for_creation(&db, &fixture.request_id).await,
            (true, false, None)
        );
        assert!(
            db.authenticate_runtime_credential(&successor_secret)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            current_credential_state(&db, &fixture.runtime_id).await,
            predecessor_state
        );
        assert!(
            db.authenticate_runtime_credential(predecessor)
                .await
                .unwrap()
                .is_some()
        );
    })
    .await;
}

#[tokio::test]
async fn exact_relocation_cancel_checks_the_binding_and_releases_only_with_attestation() {
    with_isolated_postgres(|db| async move {
        let fixture = prepare_relocation(&db, "exact-cancel", true).await;
        let lease = "api-relocation-exact-cancel-relocation-lease";
        let exact = |runtime: &str, host: &str, confirm: bool| crate::CancelRelocationExactInput {
            relocation_request_id: fixture.request_id.clone(),
            expected_agent_runtime_id: runtime.to_string(),
            expected_target_source_host_id: host.to_string(),
            confirm_target_compute_stopped: confirm,
            now: None,
        };
        for (runtime, host) in [
            ("runtime_other", fixture.target_host.as_str()),
            (fixture.runtime_id.as_str(), "other-host"),
        ] {
            let error = db
                .cancel_relocation_exact(exact(runtime, host, false))
                .await
                .err();
            assert!(
                matches!(
                    error,
                    Some(crate::CoreError::ProviderOperationIdentityMismatch)
                ),
                "{error:?}"
            );
        }
        let ordinary = db
            .cancel_relocation_exact(crate::CancelRelocationExactInput {
                relocation_request_id: fixture.origin_request_id.clone(),
                ..exact(&fixture.runtime_id, &fixture.target_host, false)
            })
            .await
            .err();
        assert!(matches!(
            ordinary,
            Some(crate::CoreError::ProviderOperationIdentityMismatch)
        ));

        // The dry run prints the projection it would commit, then rolls back.
        let dry_run = crate::store::CoreStore::connect_dry_run(&db.url)
            .await
            .unwrap();
        let preview = printed(
            &dry_run
                .cancel_relocation_exact(exact(&fixture.runtime_id, &fixture.target_host, false))
                .await
                .unwrap(),
        );
        assert_eq!(creation_status(&db, &fixture.request_id).await, "launching");

        let cancelled = printed(
            &db.cancel_relocation_exact(exact(&fixture.runtime_id, &fixture.target_host, false))
                .await
                .unwrap(),
        );
        for view in [&preview, &cancelled] {
            assert_eq!(view["relocation_request_id"], fixture.request_id.as_str());
            assert_eq!(view["agent_runtime_id"], fixture.runtime_id.as_str());
            assert_eq!(view["target_source_host_id"], fixture.target_host.as_str());
            assert_eq!(view["runner_id"], "runner-oslo-1");
            assert_eq!(view["status"], "cancelled");
            assert_eq!(view["lease_held"], true, "{view}");
            assert!(view["lease_expires_at"].is_string(), "{view}");
            assert!(
                !view.to_string().contains(lease),
                "the lease token must not be printed: {view}"
            );
        }
        assert!(refused(&owner_stop(&db, &fixture).await));

        // The target Runner never returns. Release needs the attestation and
        // an expired lease; a held lease without an expiry never counts as
        // expired.
        for confirm in [false, true] {
            let error = db
                .cancel_relocation_exact(exact(&fixture.runtime_id, &fixture.target_host, confirm))
                .await
                .err();
            assert!(
                matches!(
                    error,
                    Some(crate::CoreError::AgentCreationRequestNotCancellable)
                ),
                "{error:?}"
            );
        }
        execute_test_sql(
            &db,
            "UPDATE agent_creation_requests SET lease_expires_at = NULL WHERE id = $1",
            &[&fixture.request_id],
        )
        .await;
        let error = db
            .cancel_relocation_exact(exact(&fixture.runtime_id, &fixture.target_host, true))
            .await
            .err();
        assert!(
            matches!(
                error,
                Some(crate::CoreError::AgentCreationRequestNotCancellable)
            ),
            "{error:?}"
        );
        expire_creation_lease(&db, &fixture.request_id).await;
        let error = db
            .cancel_relocation_exact(exact(&fixture.runtime_id, &fixture.target_host, false))
            .await
            .err();
        assert!(matches!(
            error,
            Some(crate::CoreError::AgentCreationRequestNotCancellable)
        ));
        let released = printed(
            &db.cancel_relocation_exact(exact(&fixture.runtime_id, &fixture.target_host, true))
                .await
                .unwrap(),
        );
        assert_eq!(released["status"], "cancelled");
        assert_eq!(released["lease_held"], false, "{released}");
        let accepted = owner_stop(&db, &fixture).await;
        assert!(accepted.is_none(), "{accepted:?}");
    })
    .await;
}
