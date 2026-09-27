use super::*;
use crate::store::hosted_hermes::{ApplyStatus, HostedReport, SetHostedAccess};
use crate::store::runtime_credentials::ProvisionUpgradeCredential;
use crate::{
    AdminRuntimeUpgradeInput, CompleteRuntimeControlRequestInput, LeaseRuntimeControlRequestInput,
    RequestRuntimeStopInput,
};

async fn run_relocation_credential_handoff(same_host: bool) {
    with_isolated_postgres(|db| async move {
        let suffix = if same_host { "same" } else { "cross" };
        let mut runtime = create_runtime(&db, suffix, true).await;
        if !same_host { runtime.target_host = "relocation-cross-target".into(); }
        let source_host = runtime.source_host.clone();
        let target_host = runtime.target_host.clone();
        let source_machine = runtime.source_machine.clone();
        let runner = "runner-oslo-1".to_string();
        let owner_email = format!("{suffix}@finite.vip");
        let owner_workos = format!("api-relocation-{suffix}-owner");
        let admin_email = format!("{suffix}-admin@finite.vip");
        let admin_workos = format!("api-relocation-{suffix}-admin");
        let project_id = runtime.project_id.clone();
        let runtime_id = runtime.runtime_id.clone();
        let old_secret = runtime.predecessor_secret.as_ref().unwrap().clone();
        let initial_hosted = db.hosted_access(&runtime_id, &owner_workos).await.unwrap();
        // Keep API fixtures enrolled but without hosted access; only this
        // lifecycle enables it so both initial states stay covered.
        let initial_hosted = db.set_hosted_access(&runtime_id, &owner_workos, SetHostedAccess {
            enabled: true, expected_generation: initial_hosted.generation,
        }).await.unwrap();
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
                runner_capacity: Some(relocation_capacity()),
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

        let fixture = enqueue_relocation(&db, &runtime).await;
        let relocation_id = fixture.request_id.clone();
        let relocation_lease_token = format!("{suffix}-relocation-lease");
        assert_legacy_cannot_lease(&db, &fixture).await;
        lease_relocation(&db, &fixture, &relocation_lease_token).await;

        // This is the regression boundary: a replacement incarnation must
        // receive a new credential before it can register and complete.
        let replacement_secret = db
            .provision_relocation_credential(ProvisionRuntimeCredential {
                creation_request_id: relocation_id.clone(),
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
        let latest_native = db.query_json(
            "SELECT jsonb_build_array(hosted_username, hosted_password, hosted_signing_secret) FROM runtime_core_credentials WHERE creation_request_id=$1",
            &[&runtime.origin_request_id],
        ).await.remove(0);

        register_relocation(&db, &fixture, &relocation_lease_token).await;
        complete_relocation(&db, &fixture, &relocation_lease_token).await;

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
            relocated_desired.username == serde_json::from_value::<Option<String>>(latest_native[0].clone()).unwrap()
                && relocated_desired.password == serde_json::from_value::<Option<String>>(latest_native[1].clone()).unwrap()
                && relocated_desired.signing_secret == serde_json::from_value::<Option<String>>(latest_native[2].clone()).unwrap()
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
        assert_eq!(credential_state_for_creation(&db, &runtime.origin_request_id).await, (true, false, None));

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
                runner_capacity: Some(relocation_capacity()),
                now: None,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(upgrade_lease.request.id, upgrade.id);
        // A matching host alone cannot authorize a credential whose recorded
        // relocation belongs to a different machine.
        execute_test_sql(&db,
            "UPDATE agent_creation_requests SET relocation_spec=jsonb_set(relocation_spec, '{relocation,sourceMachineId}', to_jsonb('different-machine'::text)) WHERE id=$1",
            &[&relocation_id],
        ).await;
        let rejected = db.provision_upgrade_credential(ProvisionUpgradeCredential {
            request_id: upgrade.id.clone(), prepare_hosted_access: false,
            runner_id: runner.clone(), lease_token: upgrade_lease_token.clone(),
            source_host_id: target_host.clone(),
        }).await;
        assert!(matches!(rejected, Err(crate::CoreError::ProviderOperationTransitionConflict)));
        execute_test_sql(&db,
            "UPDATE agent_creation_requests SET relocation_spec=jsonb_set(relocation_spec, '{relocation,sourceMachineId}', to_jsonb($2::text)) WHERE id=$1",
            &[&relocation_id, &source_machine],
        ).await;
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
            runtime_capabilities: relocation_capacity().runtime_capabilities,
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
    run_relocation_credential_handoff(true).await;
}

#[tokio::test]
async fn relocation_replaces_credential_and_preserves_later_upgrade_cross_host() {
    run_relocation_credential_handoff(false).await;
}
