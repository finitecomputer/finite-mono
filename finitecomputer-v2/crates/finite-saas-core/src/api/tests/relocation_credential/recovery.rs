use super::*;
use crate::store::runtime_credentials::{ProvisionUpgradeCredential, RecoverRelocatedCredential};
use crate::{AdminRuntimeUpgradeExactInput, LeaseRuntimeControlRequestInput};

async fn legacy_relocated(db: &TestDb, run: &str) -> PreparedRelocation {
    let runtime = create_runtime(db, run, true).await;
    let workos = format!("api-relocation-{run}-owner");
    let hosted = db
        .hosted_access(&runtime.runtime_id, &workos)
        .await
        .unwrap();
    db.set_hosted_access(
        &runtime.runtime_id,
        &workos,
        crate::store::hosted_hermes::SetHostedAccess {
            enabled: true,
            expected_generation: hosted.generation,
        },
    )
    .await
    .unwrap();
    let fixture = enqueue_relocation(db, &runtime).await;
    let token = format!("api-relocation-{run}-relocation-lease");
    lease_relocation(db, &fixture, &token).await;
    db.provision_relocation_credential(ProvisionRuntimeCredential {
        creation_request_id: fixture.request_id.clone(),
        runner_id: "runner-oslo-1".into(),
        lease_token: token.clone(),
        source_host_id: fixture.target_host.clone(),
        prepare_hosted_access: false,
    })
    .await
    .unwrap();
    register_relocation(db, &fixture, &token).await;
    complete_relocation(db, &fixture, &token).await;
    // Reconstruct the persisted pre-#993 boundary: relocation completed,
    // no successor was delivered, and its predecessor remains bound/revoked.
    execute_test_sql(
        db,
        "DELETE FROM runtime_core_credentials WHERE creation_request_id=$1",
        &[&fixture.request_id],
    )
    .await;
    execute_test_sql(db, "UPDATE runtime_core_credentials SET agent_runtime_id=$2,revoked=TRUE,activated=FALSE WHERE creation_request_id=$1", &[&fixture.origin_request_id, &fixture.runtime_id]).await;
    artifact(db, &format!("{run}-recovery-v2"), '2').await;
    fixture
}

fn recovery_input(f: &PreparedRelocation) -> RecoverRelocatedCredential {
    RecoverRelocatedCredential {
        upgrade: AdminRuntimeUpgradeExactInput {
            admin_verified_email: format!("{}-recovery-admin@finite.vip", f.run),
            admin_workos_user_id: format!("{}-recovery-admin", f.run),
            project_id: f.project_id.clone(),
            expected_agent_runtime_id: f.runtime_id.clone(),
            expected_source_host_id: f.target_host.clone(),
            expected_source_machine_id: f.source_machine.clone(),
            target_runtime_artifact_id: format!("{}-recovery-v2", f.run),
            now: None,
        },
        expected_owner_email: format!("{}@finite.vip", f.run),
        expected_agent_npub: format!("npub1{}", "a".repeat(58)),
        expected_predecessor_creation_request_id: f.origin_request_id.clone(),
        expected_relocation_request_id: f.request_id.clone(),
        confirm_relocation_credential_loss: true,
    }
}

#[tokio::test]
async fn legacy_relocation_recovery_rejects_changed_or_ambiguous_state_atomically() {
    with_isolated_postgres(|db| async move {
        for case in ["owner", "host", "machine", "principal", "origin", "relocation", "attestation", "artifact", "same-image", "offboarding", "inactive", "unrevoked", "active-control", "extra-credential"] {
            let f = legacy_relocated(&db, &format!("deny-{case}")).await;
            let mut input = recovery_input(&f);
            match case {
                "owner" => input.expected_owner_email = "someone-else@finite.vip".into(),
                "host" => input.upgrade.expected_source_host_id = "other-host".into(),
                "machine" => input.upgrade.expected_source_machine_id = "other-machine".into(),
                "principal" => input.expected_agent_npub = format!("npub1{}", "q".repeat(58)),
                "origin" => input.expected_predecessor_creation_request_id = f.request_id.clone(),
                "relocation" => input.expected_relocation_request_id = f.origin_request_id.clone(),
                "attestation" => input.confirm_relocation_credential_loss = false,
                "artifact" => input.upgrade.target_runtime_artifact_id = "missing-artifact".into(),
                "same-image" => input.upgrade.target_runtime_artifact_id = f.artifact_id.clone(),
                "offboarding" => execute_test_sql(&db, "UPDATE agent_runtimes SET offboarding_phase='retirement_requested' WHERE id=$1", &[&f.runtime_id]).await,
                "inactive" => execute_test_sql(&db, "UPDATE project_runtime_links SET active=FALSE WHERE agent_runtime_id=$1", &[&f.runtime_id]).await,
                "unrevoked" => execute_test_sql(&db, "UPDATE runtime_core_credentials SET revoked=FALSE WHERE creation_request_id=$1", &[&f.origin_request_id]).await,
                "active-control" => { db.admin_request_runtime_upgrade_exact(input.upgrade.clone()).await.unwrap(); },
                "extra-credential" => execute_test_sql(&db, "INSERT INTO runtime_core_credentials (creation_request_id,source_host_id,bootstrap_secret,token_sha256,lease_sha256,owner_user_id) SELECT $1,source_host_id,$2,$3,lease_sha256,owner_user_id FROM runtime_core_credentials WHERE creation_request_id=$4", &[&f.request_id,&"d".repeat(64),&credential_hash(&"d".repeat(64)),&f.origin_request_id]).await,
                _ => unreachable!(),
            }
            let before = current_credential_state(&db, &f.runtime_id).await;
            assert!(db.recover_relocated_credential(input).await.is_err(), "accepted {case}");
            assert_eq!(current_credential_state(&db, &f.runtime_id).await, before, "changed {case}");
            let counts = db.query_json("SELECT jsonb_build_array((SELECT count(*) FROM runtime_core_credentials WHERE creation_request_id=$1),(SELECT count(*) FROM runtime_control_requests WHERE agent_runtime_id=$2))", &[&f.request_id, &f.runtime_id]).await;
            assert_eq!(counts[0], serde_json::json!([usize::from(case == "extra-credential"),usize::from(case == "active-control")]), "partial recovery for {case}");
        }
    }).await;
}

#[tokio::test]
async fn legacy_relocation_recovery_delivers_new_credential_through_exact_upgrade() {
    with_isolated_postgres(|db| async move {
        let f = legacy_relocated(&db, "legacy-recovery").await;
        let old = f.predecessor_secret.as_ref().unwrap();
        assert!(db.authenticate_runtime_credential(old).await.unwrap().is_none());
        let request = db.recover_relocated_credential(recovery_input(&f)).await
            .expect("an explicit recovery must restore upgrade eligibility without reviving the old token");
        assert_eq!(request.agent_runtime_id, f.runtime_id);
        let lease = db.lease_runtime_control_request(LeaseRuntimeControlRequestInput {
            runner_id: "runner-oslo-1".into(), lease_token: "recovery-upgrade-lease".into(),
            lease_seconds: Some(300), source_host_id: Some(f.target_host.clone()),
            runner_capacity: Some(relocation_capacity()), now: None,
        }).await.unwrap().unwrap();
        assert_eq!(lease.request.id, request.id);
        let provision = || ProvisionUpgradeCredential {
            request_id: request.id.clone(), runner_id: "runner-oslo-1".into(),
            lease_token: "recovery-upgrade-lease".into(), source_host_id: f.target_host.clone(),
            prepare_hosted_access: false,
        };
        let delivered = db.provision_upgrade_credential(provision()).await.unwrap();
        assert!(delivered.expected_previous_credential_sha256.as_deref() == Some(credential_hash(old).as_str()));
        let fresh = delivered.secret;
        assert!(fresh != *old);
        assert!(db.provision_upgrade_credential(provision()).await.unwrap().secret == fresh);
        assert!(db.authenticate_runtime_credential(old).await.unwrap().is_none());
        assert!(db.authenticate_runtime_credential(&fresh).await.unwrap().is_some());
        assert_eq!(credential_state_for_creation(&db, &f.origin_request_id).await, (true, false, None));
        assert!(db.recover_relocated_credential(recovery_input(&f)).await.is_err());
        let copied = db.query_json("SELECT to_jsonb(a.hosted_enabled=b.hosted_enabled AND a.hosted_generation=b.hosted_generation AND a.hosted_username=b.hosted_username AND a.hosted_password=b.hosted_password AND a.hosted_signing_secret=b.hosted_signing_secret AND b.hosted_applied_generation IS NULL AND b.hosted_apply_status='pending') FROM runtime_core_credentials a, runtime_core_credentials b WHERE a.creation_request_id=$1 AND b.creation_request_id=$2", &[&f.origin_request_id, &f.request_id]).await;
        assert_eq!(copied[0], serde_json::json!(true));
        db.complete_runtime_control_request(crate::CompleteRuntimeControlRequestInput {
        trial_archive: None,
            request_id: request.id, runner_id: "runner-oslo-1".into(),
            lease_token: "recovery-upgrade-lease".into(),
            runtime_artifact_id: Some(format!("{}-recovery-v2", f.run)),
            state_schema_version: Some("state-v1".into()),
            runtime_capabilities: relocation_capacity().runtime_capabilities,
            runtime_host: Some(f.target_host.clone()), published_app_urls: Some(vec!["https://recovery.example.test/contact".into()]),
            retirement_snapshot: None, now: None,
        }).await.unwrap();
        assert!(db.authenticate_runtime_credential(&fresh).await.unwrap().is_some());
    }).await;
}

#[tokio::test]
async fn legacy_relocation_recovery_dry_run_and_concurrent_calls_preserve_one_successor() {
    with_isolated_postgres(|db| async move {
        let f = legacy_relocated(&db, "legacy-race").await;
        let preview = CoreStore::connect_dry_run(&db.url).await.unwrap();
        preview.recover_relocated_credential(recovery_input(&f)).await.unwrap();
        assert_eq!(current_credential_state(&db, &f.runtime_id).await, (true, false, Some(f.runtime_id.clone())));
        let counts = db.query_json("SELECT jsonb_build_array((SELECT count(*) FROM runtime_core_credentials WHERE creation_request_id=$1),(SELECT count(*) FROM runtime_control_requests WHERE agent_runtime_id=$2))", &[&f.request_id, &f.runtime_id]).await;
        assert_eq!(counts[0], serde_json::json!([0,0]));
        let (a,b) = tokio::join!(db.recover_relocated_credential(recovery_input(&f)), db.recover_relocated_credential(recovery_input(&f)));
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        let counts = db.query_json("SELECT jsonb_build_array((SELECT count(*) FROM runtime_core_credentials WHERE creation_request_id=$1),(SELECT count(*) FROM runtime_control_requests WHERE agent_runtime_id=$2))", &[&f.request_id, &f.runtime_id]).await;
        assert_eq!(counts[0], serde_json::json!([1,1]));
    }).await;
}

#[tokio::test]
async fn legacy_relocation_recovery_retries_failed_delivery_without_rotating_again() {
    with_isolated_postgres(|db| async move {
        let f = legacy_relocated(&db, "recovery-retry").await;
        let input = recovery_input(&f);
        let first = db
            .recover_relocated_credential(input.clone())
            .await
            .unwrap();
        let lease_input = || LeaseRuntimeControlRequestInput {
            runner_id: "runner-oslo-1".into(),
            lease_token: "retry-lease".into(),
            lease_seconds: Some(300),
            source_host_id: Some(f.target_host.clone()),
            runner_capacity: Some(relocation_capacity()),
            now: None,
        };
        db.lease_runtime_control_request(lease_input())
            .await
            .unwrap()
            .unwrap();
        let provision = |id: String| ProvisionUpgradeCredential {
            request_id: id,
            runner_id: "runner-oslo-1".into(),
            lease_token: "retry-lease".into(),
            source_host_id: f.target_host.clone(),
            prepare_hosted_access: false,
        };
        let first_credential = db
            .provision_upgrade_credential(provision(first.id.clone()))
            .await
            .unwrap();
        db.fail_runtime_control_request(crate::FailRuntimeControlRequestInput {
            request_id: first.id,
            runner_id: "runner-oslo-1".into(),
            lease_token: "retry-lease".into(),
            failure_message: "synthetic failure before candidate launch".into(),
            failure_stage: None,
            now: None,
        })
        .await
        .unwrap();
        let retry = db
            .admin_request_runtime_upgrade_exact(input.upgrade)
            .await
            .unwrap();
        assert_eq!(
            db.lease_runtime_control_request(lease_input())
                .await
                .unwrap()
                .unwrap()
                .request
                .id,
            retry.id
        );
        let credential = db
            .provision_upgrade_credential(provision(retry.id))
            .await
            .unwrap();
        assert!(credential.secret == first_credential.secret);
        assert!(credential.expected_previous_credential_sha256.is_some());
        assert!(
            credential.expected_previous_credential_sha256
                == first_credential.expected_previous_credential_sha256
        );
        assert!(
            db.authenticate_runtime_credential(f.predecessor_secret.as_ref().unwrap())
                .await
                .unwrap()
                .is_none()
        );
    })
    .await;
}

#[test]
fn legacy_relocation_recovery_wire_fails_closed_on_old_runner() {
    // Retained pre-recovery response reader, including deny_unknown_fields.
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct PreviousBootstrapCredential {
        secret: String,
    }
    use crate::store::runtime_credentials::RuntimeBootstrapCredential;
    let ordinary = RuntimeBootstrapCredential {
        secret: "a".repeat(64),
        expected_previous_credential_sha256: None,
    };
    let ordinary_wire = serde_json::to_value(&ordinary).unwrap();
    assert!(
        serde_json::from_value::<PreviousBootstrapCredential>(ordinary_wire.clone())
            .unwrap()
            .secret
            == ordinary.secret
    );
    assert!(
        serde_json::from_value::<RuntimeBootstrapCredential>(ordinary_wire)
            .unwrap()
            .expected_previous_credential_sha256
            .is_none()
    );
    let repair = RuntimeBootstrapCredential {
        expected_previous_credential_sha256: Some("b".repeat(64)),
        ..ordinary
    };
    assert!(
        serde_json::from_value::<PreviousBootstrapCredential>(
            serde_json::to_value(repair).unwrap()
        )
        .is_err()
    );
}

fn credential_hash(value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

#[tokio::test]
async fn legacy_relocation_recovery_migration_preserves_existing_credentials() {
    with_isolated_postgres(|db| async move {
        let f = legacy_relocated(&db, "recovery-migration").await;
        execute_test_sql(&db, "DROP TABLE runtime_credential_recoveries", &[]).await;
        let before = current_credential_state(&db, &f.runtime_id).await;
        let (client, connection) = tokio_postgres::connect(&db.url, tokio_postgres::NoTls).await.unwrap();
        let connection_task = tokio::spawn(connection);
        let migration = include_str!("../../../../migrations/0034_runtime_credential_recovery.sql");
        client.batch_execute(migration).await.unwrap();
        assert_eq!(current_credential_state(&db, &f.runtime_id).await, before);
        assert!(db.authenticate_runtime_credential(f.predecessor_secret.as_ref().unwrap()).await.unwrap().is_none());
        let operation = db.recover_relocated_credential(recovery_input(&f)).await.unwrap();
        client.batch_execute(migration).await.unwrap();
        assert_eq!(db.query_json("SELECT to_jsonb(count(*)) FROM runtime_credential_recoveries WHERE runtime_control_request_id=$1", &[&operation.id]).await[0], serde_json::json!(1));
        drop(client);
        connection_task.await.unwrap().unwrap();
    }).await;
}
