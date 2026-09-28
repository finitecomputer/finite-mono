use super::*;

#[tokio::test]
async fn relocation_failure_and_cancel_revoke_only_pending_credentials() {
    with_isolated_postgres(|db| async move {
        let fixture = prepare_relocation(&db, "failure", true).await;
        let predecessor = fixture.predecessor_secret.as_deref().unwrap();
        let app = router(db.store.clone(), scoped_test_auth());

        let first_lease = "api-relocation-failure-relocation-lease";
        let (status, first) =
            provision_relocation_over_http(&app, &fixture.request_id, first_lease).await;
        assert_eq!(status, StatusCode::OK);
        let first_secret = first["secret"].as_str().unwrap().to_string();
        assert!(
            db.authenticate_runtime_credential(&first_secret)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            credential_state_for_creation(&db, &fixture.request_id).await,
            (false, false, None)
        );
        assert!(
            db.authenticate_runtime_credential(predecessor)
                .await
                .unwrap()
                .is_some()
        );
        db.fail_agent_creation_request(FailAgentCreationRequestInput {
            request_id: fixture.request_id.clone(),
            runner_id: "runner-oslo-1".to_string(),
            lease_token: first_lease.to_string(),
            failure_message: "synthetic prelaunch failure".to_string(),
            provisioned_finite_private_api_key_id: None,
            now: None,
        })
        .await
        .unwrap();
        assert!(
            db.authenticate_runtime_credential(predecessor)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            db.authenticate_runtime_credential(&first_secret)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            credential_state_for_creation(&db, &fixture.request_id).await,
            (true, false, None)
        );

        let second_fixture = enqueue_relocation(&db, &fixture).await;
        let second_lease = "api-relocation-failure-register-lease";
        lease_relocation(&db, &second_fixture, second_lease).await;
        let (status, second) =
            provision_relocation_over_http(&app, &second_fixture.request_id, second_lease).await;
        assert_eq!(status, StatusCode::OK);
        let second_secret = second["secret"].as_str().unwrap().to_string();
        assert!(
            second_secret != first_secret,
            "retry reused revoked credential"
        );
        assert_eq!(
            credential_state_for_creation(&db, &second_fixture.request_id).await,
            (false, false, None)
        );
        register_relocation(&db, &second_fixture, second_lease).await;
        db.fail_agent_creation_request(FailAgentCreationRequestInput {
            request_id: second_fixture.request_id.clone(),
            runner_id: "runner-oslo-1".to_string(),
            lease_token: second_lease.to_string(),
            failure_message: "synthetic post-register failure".to_string(),
            provisioned_finite_private_api_key_id: None,
            now: None,
        })
        .await
        .unwrap();
        assert!(
            db.authenticate_runtime_credential(predecessor)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            db.authenticate_runtime_credential(&second_secret)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            credential_state_for_creation(&db, &second_fixture.request_id).await,
            (true, false, None)
        );

        let third_fixture = enqueue_relocation(&db, &fixture).await;
        let third_lease = "api-relocation-failure-cancel-lease";
        lease_relocation(&db, &third_fixture, third_lease).await;
        let (status, third) =
            provision_relocation_over_http(&app, &third_fixture.request_id, third_lease).await;
        assert_eq!(status, StatusCode::OK);
        let third_secret = third["secret"].as_str().unwrap().to_string();
        assert!(
            third_secret != second_secret,
            "retry reused revoked credential"
        );
        assert_eq!(
            credential_state_for_creation(&db, &third_fixture.request_id).await,
            (false, false, None)
        );
        db.cancel_agent_creation_request(CancelAgentCreationRequestInput {
            request_id: third_fixture.request_id.clone(),
            now: None,
        })
        .await
        .unwrap();
        assert!(
            db.authenticate_runtime_credential(predecessor)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            db.authenticate_runtime_credential(&third_secret)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            credential_state_for_creation(&db, &third_fixture.request_id).await,
            (true, false, None)
        );

        let fourth_fixture = enqueue_relocation(&db, &fixture).await;
        let fourth_lease = "api-relocation-failure-complete-lease";
        lease_relocation(&db, &fourth_fixture, fourth_lease).await;
        let (status, fourth) =
            provision_relocation_over_http(&app, &fourth_fixture.request_id, fourth_lease).await;
        assert_eq!(status, StatusCode::OK);
        let fourth_secret = fourth["secret"].as_str().unwrap().to_string();
        assert!(
            fourth_secret != third_secret,
            "fresh attempt reused revoked credential"
        );
        register_relocation(&db, &fourth_fixture, fourth_lease).await;
        complete_relocation(&db, &fourth_fixture, fourth_lease).await;
        assert!(
            db.authenticate_runtime_credential(predecessor)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            db.authenticate_runtime_credential(&fourth_secret)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            credential_state_for_creation(&db, &fourth_fixture.request_id).await,
            (false, true, Some(fourth_fixture.runtime_id.clone()))
        );
        assert_eq!(
            current_credential_state(&db, &fourth_fixture.runtime_id).await,
            (false, true, Some(fourth_fixture.runtime_id))
        );
    })
    .await;
}

#[tokio::test]
async fn relocation_credential_provisioning_rejects_invalid_predecessor_state() {
    with_isolated_postgres(|db| async move {
        let revoked = prepare_relocation(&db, "revoked-predecessor", true).await;
        execute_test_sql(
            &db,
            "UPDATE runtime_core_credentials
             SET revoked = TRUE
             WHERE agent_runtime_id = $1",
            &[&revoked.runtime_id],
        )
        .await;
        let app = router(db.store.clone(), scoped_test_auth());
        let (status, _) = provision_relocation_over_http(
            &app,
            &revoked.request_id,
            "api-relocation-revoked-predecessor-relocation-lease",
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        let missing = prepare_relocation(&db, "missing-predecessor", true).await;
        execute_test_sql(&db,
            "UPDATE runtime_core_credentials SET agent_runtime_id=NULL, revoked=TRUE WHERE creation_request_id=$1",
            &[&missing.origin_request_id],
        ).await;
        expire_creation_lease(&db, &missing.request_id).await;
        assert_legacy_cannot_lease(&db, &missing).await;
        lease_relocation(&db, &missing, "api-relocation-missing-predecessor-relocation-lease").await;
        let (status, _) = provision_relocation_over_http(&app, &missing.request_id,
            "api-relocation-missing-predecessor-relocation-lease").await;
        assert_eq!(status, StatusCode::CONFLICT);

        let inactive_link = prepare_relocation(&db, "inactive-link", true).await;
        execute_test_sql(
            &db,
            "UPDATE project_runtime_links
             SET active = FALSE
             WHERE project_id = $1 AND agent_runtime_id = $2",
            &[&inactive_link.project_id, &inactive_link.runtime_id],
        )
        .await;
        let (status, _) = provision_relocation_over_http(
            &app,
            &inactive_link.request_id,
            "api-relocation-inactive-link-relocation-lease",
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        let offboarding = prepare_relocation(&db, "offboarding", true).await;
        execute_test_sql(
            &db,
            "UPDATE agent_runtimes
             SET offboarding_phase = 'retirement_requested'
             WHERE id = $1",
            &[&offboarding.runtime_id],
        )
        .await;
        let (status, _) = provision_relocation_over_http(
            &app,
            &offboarding.request_id,
            "api-relocation-offboarding-relocation-lease",
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        let wrong_host = prepare_relocation(&db, "wrong-host", true).await;
        execute_test_sql(
            &db,
            "UPDATE agent_creation_requests
             SET target_source_host_id = 'different-source-host'
             WHERE id = $1",
            &[&wrong_host.request_id],
        )
        .await;
        let (status, _) = provision_relocation_over_http(
            &app,
            &wrong_host.request_id,
            "api-relocation-wrong-host-relocation-lease",
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        let wrong_machine = prepare_relocation(&db, "wrong-machine", true).await;
        execute_test_sql(
            &db,
            "UPDATE agent_creation_requests
             SET relocation_spec = jsonb_set(
                 relocation_spec,
                 '{relocation,sourceMachineId}',
                 to_jsonb('different-machine'::text)
             )
             WHERE id = $1",
            &[&wrong_machine.request_id],
        )
        .await;
        let (status, _) = provision_relocation_over_http(
            &app,
            &wrong_machine.request_id,
            "api-relocation-wrong-machine-relocation-lease",
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        for (run, status_value) in [
            ("failed-origin", "failed"),
            ("cancelled-origin", "cancelled"),
        ] {
            let origin = prepare_relocation(&db, run, true).await;
            let status_value = status_value.to_string();
            execute_test_sql(
                &db,
                "UPDATE agent_creation_requests SET status = $1 WHERE id = $2",
                &[&status_value, &origin.origin_request_id],
            )
            .await;
            let (status, _) = provision_relocation_over_http(
                &app,
                &origin.request_id,
                &format!("api-relocation-{run}-relocation-lease"),
            )
            .await;
            assert_eq!(status, StatusCode::CONFLICT);
        }
    })
    .await;
}

#[tokio::test]
async fn relocation_completion_rolls_back_when_successor_is_missing_or_revoked() {
    with_isolated_postgres(|db| async move {
        let missing = prepare_relocation(&db, "missing-successor", true).await;
        let old_state = current_credential_state(&db, &missing.runtime_id).await;
        let app = router(db.store.clone(), scoped_test_auth());
        let missing_lease = "api-relocation-missing-successor-relocation-lease";
        register_relocation(&db, &missing, missing_lease).await;
        let result = db
            .complete_agent_creation_request(relocation_completion(&missing, missing_lease))
            .await;
        assert!(result.is_err());
        assert_eq!(
            current_credential_state(&db, &missing.runtime_id).await,
            old_state
        );

        let revoked = prepare_relocation(&db, "revoked-successor", true).await;
        let old_state = current_credential_state(&db, &revoked.runtime_id).await;
        let revoked_lease = "api-relocation-revoked-successor-relocation-lease";
        let (status, successor) =
            provision_relocation_over_http(&app, &revoked.request_id, revoked_lease).await;
        assert_eq!(status, StatusCode::OK);
        let successor_secret = successor["secret"].as_str().unwrap().to_string();
        execute_test_sql(
            &db,
            "UPDATE runtime_core_credentials
             SET revoked = TRUE, activated = FALSE
             WHERE creation_request_id = $1",
            &[&revoked.request_id],
        )
        .await;
        register_relocation(&db, &revoked, revoked_lease).await;
        let result = db
            .complete_agent_creation_request(relocation_completion(&revoked, revoked_lease))
            .await;
        assert!(result.is_err());
        assert_eq!(
            current_credential_state(&db, &revoked.runtime_id).await,
            old_state
        );
        assert_eq!(
            credential_state_for_creation(&db, &revoked.request_id).await,
            (true, false, None)
        );
        assert!(
            db.authenticate_runtime_credential(&successor_secret)
                .await
                .unwrap()
                .is_none()
        );
    })
    .await;
}

#[tokio::test]
async fn relocation_completion_rolls_back_when_lease_expires_before_activation() {
    with_isolated_postgres(|db| async move {
        let fixture = prepare_relocation(&db, "activation-expiry", true).await;
        let predecessor = fixture.predecessor_secret.as_deref().unwrap();
        let app = router(db.store.clone(), scoped_test_auth());
        let lease = "api-relocation-activation-expiry-relocation-lease";
        let (status, successor) =
            provision_relocation_over_http(&app, &fixture.request_id, lease).await;
        assert_eq!(status, StatusCode::OK);
        let successor_secret = successor["secret"].as_str().unwrap().to_string();
        register_relocation(&db, &fixture, lease).await;
        let predecessor_state = current_credential_state(&db, &fixture.runtime_id).await;

        // The lease expires inside the completion transaction, after the
        // handoff passed its liveness check and bound the successor, and
        // before activation reads the clock again.
        execute_test_sql(
            &db,
            "CREATE FUNCTION expire_lease_when_successor_binds() RETURNS trigger
             LANGUAGE plpgsql AS $$
             BEGIN
               UPDATE agent_creation_requests
               SET lease_expires_at = clock_timestamp() - INTERVAL '1 second'
               WHERE id = NEW.creation_request_id;
               RETURN NEW;
             END $$",
            &[],
        )
        .await;
        execute_test_sql(
            &db,
            "CREATE TRIGGER expire_lease_when_successor_binds
             AFTER UPDATE OF agent_runtime_id ON runtime_core_credentials
             FOR EACH ROW
             WHEN (OLD.agent_runtime_id IS NULL AND NEW.agent_runtime_id IS NOT NULL)
             EXECUTE FUNCTION expire_lease_when_successor_binds()",
            &[],
        )
        .await;
        let error = db
            .complete_agent_creation_request(relocation_completion(&fixture, lease))
            .await
            .err();
        assert!(
            matches!(
                error,
                Some(crate::CoreError::AgentCreationRequestLeaseConflict)
            ),
            "completion must fail as a whole when the successor cannot activate: {error:?}"
        );
        // A Runner sees the same 409 that an expired lease already returns at
        // the first clock read, so it takes its existing failure path.
        assert_eq!(
            complete_relocation_over_http(&app, &fixture, lease).await,
            StatusCode::CONFLICT
        );
        assert_eq!(creation_status(&db, &fixture.request_id).await, "launching");
        assert_eq!(
            current_credential_state(&db, &fixture.runtime_id).await,
            predecessor_state
        );
        assert_eq!(
            credential_state_for_creation(&db, &fixture.request_id).await,
            (false, false, None)
        );
        assert!(
            db.authenticate_runtime_credential(predecessor)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            db.authenticate_runtime_credential(&successor_secret)
                .await
                .unwrap()
                .is_none()
        );

        // The request stays retryable: a later lease reuses the same pending
        // successor and completes the handoff.
        execute_test_sql(
            &db,
            "DROP TRIGGER expire_lease_when_successor_binds ON runtime_core_credentials",
            &[],
        )
        .await;
        expire_creation_lease(&db, &fixture.request_id).await;
        let retry = "api-relocation-activation-expiry-retry-lease";
        lease_relocation(&db, &fixture, retry).await;
        let (status, again) =
            provision_relocation_over_http(&app, &fixture.request_id, retry).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            again["secret"].as_str() == Some(successor_secret.as_str()),
            "retry must reuse the pending successor"
        );
        register_relocation(&db, &fixture, retry).await;
        complete_relocation(&db, &fixture, retry).await;
        assert_eq!(creation_status(&db, &fixture.request_id).await, "running");
        assert_eq!(
            credential_state_for_creation(&db, &fixture.request_id).await,
            (false, true, Some(fixture.runtime_id.clone()))
        );
        assert!(
            db.authenticate_runtime_credential(&successor_secret)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            db.authenticate_runtime_credential(predecessor)
                .await
                .unwrap()
                .is_none()
        );
    })
    .await;
}
