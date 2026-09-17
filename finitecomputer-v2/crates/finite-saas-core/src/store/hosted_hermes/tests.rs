use super::*;
use crate::store::runtime_credentials::tests::{complete, provision, register, requested};
use crate::test_support::with_isolated_postgres;
fn origins() -> crate::hosted_hermes::HostedHermesOrigins {
    crate::hosted_hermes::HostedHermesOrigins::from_json(
        r#"{"auth-host":"https://agent.example.test"}"#,
    )
    .unwrap()
}
#[tokio::test]
async fn hosted_routes_require_completed_enrollment_and_current_applied_intent() {
    with_isolated_postgres(|db| async move {
        let request = requested(&db).await;
        let runtime = register(&db, &request).await;
        // Existing, unenrolled assignments do not gain routes implicitly.
        assert!(
            db.hosted_route_targets_for_host("auth-host")
                .await
                .unwrap()
                .is_empty()
        );
        let secret = db
            .provision_runtime_credential(provision(&request))
            .await
            .unwrap()
            .secret;
        db.set_hosted_access(
            &runtime,
            "runtime-auth-user",
            SetHostedAccess {
                enabled: true,
                expected_generation: 1,
            },
        )
        .await
        .unwrap();
        db.report_hosted(
            &secret,
            HostedReport {
                generation: 2,
                status: ApplyStatus::Applied,
            },
        )
        .await
        .unwrap();
        // A live launch can report readiness before Core completes it.
        assert!(
            db.hosted_route_targets_for_host("auth-host")
                .await
                .unwrap()
                .is_empty()
        );
        let project = complete(&db, &request).await.unwrap().project.id;
        assert_eq!(
            db.hosted_route_targets_for_host("auth-host").await.unwrap(),
            vec![HostedRouteTarget {
                runtime_id: runtime.clone(),
                project_id: project,
                source_machine_id: "auth-machine".into(),
                generation: 2,
            }]
        );
        assert!(
            db.hosted_route_targets_for_host("other-host")
                .await
                .unwrap()
                .is_empty()
        );
        db.set_hosted_access(
            &runtime,
            "runtime-auth-user",
            SetHostedAccess {
                enabled: false,
                expected_generation: 2,
            },
        )
        .await
        .unwrap();
        assert!(
            db.hosted_route_targets_for_host("auth-host")
                .await
                .unwrap()
                .is_empty()
        );
        db.set_hosted_access(
            &runtime,
            "runtime-auth-user",
            SetHostedAccess {
                enabled: true,
                expected_generation: 3,
            },
        )
        .await
        .unwrap();
        assert!(
            db.hosted_route_targets_for_host("auth-host")
                .await
                .unwrap()
                .is_empty()
        );
        db.report_hosted(
            &secret,
            HostedReport {
                generation: 4,
                status: ApplyStatus::Error,
            },
        )
        .await
        .unwrap();
        assert!(
            db.hosted_route_targets_for_host("auth-host")
                .await
                .unwrap()
                .is_empty()
        );
        db.report_hosted(
            &secret,
            HostedReport {
                generation: 4,
                status: ApplyStatus::Applied,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            db.hosted_route_targets_for_host("auth-host").await.unwrap()[0].generation,
            4
        );
    })
    .await;
}

#[tokio::test]
async fn hosted_routes_withdraw_stale_readiness_and_inactive_assignments() {
    for mutation in [
        "UPDATE runtime_core_credentials SET activated=FALSE",
        "UPDATE runtime_core_credentials SET revoked=TRUE",
        "UPDATE runtime_core_credentials SET hosted_applied_generation=hosted_generation-1",
        "UPDATE runtime_core_credentials SET hosted_applied_generation=NULL",
        "UPDATE agent_creation_requests SET agent_runtime_id=NULL",
        "UPDATE agent_runtimes SET offboarding_phase='retirement_requested'",
        "UPDATE agent_runtimes SET host_facts=jsonb_set(host_facts, '{runtime_status}', '\"offline\"')",
    ] {
        with_isolated_postgres(|db| async move {
            let request = requested(&db).await;
            let secret = db
                .provision_runtime_credential(provision(&request))
                .await
                .unwrap()
                .secret;
            let runtime = register(&db, &request).await;
            complete(&db, &request).await.unwrap();
            db.set_hosted_access(
                &runtime,
                "runtime-auth-user",
                SetHostedAccess {
                    enabled: true,
                    expected_generation: 1,
                },
            )
            .await
            .unwrap();
            db.report_hosted(
                &secret,
                HostedReport {
                    generation: 2,
                    status: ApplyStatus::Applied,
                },
            )
            .await
            .unwrap();
            assert_eq!(
                db.hosted_route_targets_for_host("auth-host")
                    .await
                    .unwrap()
                    .len(),
                1
            );
            db.connection()
                .await
                .unwrap()
                .execute(mutation, &[])
                .await
                .unwrap();
            assert!(
                db.hosted_route_targets_for_host("auth-host")
                    .await
                    .unwrap()
                    .is_empty(),
                "{mutation}"
            );
        })
        .await;
    }
}
#[tokio::test]
async fn hosted_intent_is_owner_scoped_and_rotation_is_applied_not_assumed() {
    with_isolated_postgres(|db| async move {
        let request = requested(&db).await;
        let secret = db
            .provision_runtime_credential(provision(&request))
            .await
            .unwrap()
            .secret;
        let runtime = register(&db, &request).await;
        complete(&db, &request).await.unwrap();
        assert!(db.hosted_access(&runtime, "another-user").await.is_err());
        let initial = db
            .hosted_access(&runtime, "runtime-auth-user")
            .await
            .unwrap();
        assert!(initial.enrolled && !initial.enabled);
        assert!(
            db.hosted_login(&runtime, "runtime-auth-user", &origins())
                .await
                .is_err()
        );
        let enabled = db
            .set_hosted_access(
                &runtime,
                "runtime-auth-user",
                SetHostedAccess {
                    enabled: true,
                    expected_generation: 1,
                },
            )
            .await
            .unwrap();
        assert_eq!(enabled.generation, 2);
        assert_eq!(enabled.apply_status, "pending");
        assert!(
            db.set_hosted_access(
                &runtime,
                "runtime-auth-user",
                SetHostedAccess {
                    enabled: false,
                    expected_generation: 1
                }
            )
            .await
            .is_err()
        );
        let desired = db
            .hosted_desired(&secret, &origins())
            .await
            .unwrap()
            .unwrap();
        assert!(desired.enabled);
        assert!(
            db.hosted_login(&runtime, "runtime-auth-user", &origins())
                .await
                .is_err()
        );
        assert!(
            db.report_hosted(
                &secret,
                HostedReport {
                    generation: 1,
                    status: ApplyStatus::Applied
                }
            )
            .await
            .is_err()
        );
        assert!(
            db.report_hosted(
                &secret,
                HostedReport {
                    generation: 2,
                    status: ApplyStatus::Applied
                }
            )
            .await
            .unwrap()
        );
        let login = db
            .hosted_login(&runtime, "runtime-auth-user", &origins())
            .await
            .unwrap();
        assert_eq!(Some(login.password), desired.password);
        let disabled = db
            .set_hosted_access(
                &runtime,
                "runtime-auth-user",
                SetHostedAccess {
                    enabled: false,
                    expected_generation: 2,
                },
            )
            .await
            .unwrap();
        assert_eq!(disabled.apply_status, "pending");
        assert!(
            db.hosted_login(&runtime, "runtime-auth-user", &origins())
                .await
                .is_err()
        );
        let off = db
            .hosted_desired(&secret, &origins())
            .await
            .unwrap()
            .unwrap();
        assert!(!off.enabled && off.password.is_none() && off.signing_secret.is_none());
        db.report_hosted(
            &secret,
            HostedReport {
                generation: 3,
                status: ApplyStatus::Applied,
            },
        )
        .await
        .unwrap();
        db.set_hosted_access(
            &runtime,
            "runtime-auth-user",
            SetHostedAccess {
                enabled: true,
                expected_generation: 3,
            },
        )
        .await
        .unwrap();
        let fresh = db
            .hosted_desired(&secret, &origins())
            .await
            .unwrap()
            .unwrap();
        assert!(
            fresh.password != desired.password && fresh.signing_secret != desired.signing_secret
        );
        let retry = db
            .set_hosted_access(
                &runtime,
                "runtime-auth-user",
                SetHostedAccess {
                    enabled: true,
                    expected_generation: 4,
                },
            )
            .await
            .unwrap();
        assert_eq!(retry.generation, 4);
        assert!(
            db.hosted_desired(&"f".repeat(64), &origins())
                .await
                .unwrap()
                .is_none()
        );
    })
    .await;
}
#[tokio::test]
async fn owner_transfer_and_placement_changes_fence_old_runtime_credentials() {
    for change in ["owner", "host", "machine", "link"] {
        with_isolated_postgres(|db| async move {
                let request = requested(&db).await;
                let secret = db.provision_runtime_credential(provision(&request)).await.unwrap().secret;
                let runtime = register(&db,&request).await;
                let project = complete(&db,&request).await.unwrap().project.id;
                db.set_hosted_access(&runtime,"runtime-auth-user",SetHostedAccess{enabled:true,expected_generation:1}).await.unwrap();
                db.report_hosted(&secret,HostedReport{generation:2,status:ApplyStatus::Applied}).await.unwrap();
                assert_eq!(db.hosted_route_targets_for_host("auth-host").await.unwrap().len(), 1);
                match change {
                    "owner" => {
                        let user = db.link_verified_user(LinkVerifiedUserInput{verified_email:"next@finite.test".into(),workos_user_id:"next-owner".into(),now:None}).await.unwrap();
                        db.query_json("UPDATE projects SET owner_user_id=$2 WHERE id=$1 RETURNING to_jsonb(id)",&[&project,&user.id]).await;
                    }
                    "host" => { db.query_json("UPDATE agent_runtimes SET source_host_id='changed' WHERE id=$1 RETURNING to_jsonb(id)",&[&runtime]).await; }
                    "machine" => { db.query_json("UPDATE agent_runtimes SET source_machine_id='changed' WHERE id=$1 RETURNING to_jsonb(id)",&[&runtime]).await; }
                    _ => { db.query_json("UPDATE project_runtime_links SET active=FALSE WHERE agent_runtime_id=$1 RETURNING to_jsonb(id)",&[&runtime]).await; }
                }
                assert!(db.authenticate_runtime_credential(&secret).await.unwrap().is_none());
                assert!(db.hosted_route_targets_for_host("auth-host").await.unwrap().is_empty());
                assert!(db.hosted_route_targets_for_host("changed").await.unwrap().is_empty());
                assert!(db.hosted_desired(&secret,&origins()).await.unwrap().is_none());
                assert!(!db.report_hosted(&secret,HostedReport{generation:2,status:ApplyStatus::Applied}).await.unwrap());
                assert!(db.hosted_login(&runtime,"runtime-auth-user",&origins()).await.is_err());
                if change == "owner" { assert!(db.hosted_login(&runtime,"next-owner",&origins()).await.is_err()); }
            }).await;
    }
}
#[tokio::test]
async fn existing_unenrolled_runtime_is_readable_but_cannot_enable() {
    with_isolated_postgres(|db| async move {
        let request = requested(&db).await;
        let runtime = register(&db, &request).await;
        complete(&db, &request).await.unwrap();
        assert!(
            !db.hosted_access(&runtime, "runtime-auth-user")
                .await
                .unwrap()
                .enrolled
        );
        assert!(
            db.set_hosted_access(
                &runtime,
                "runtime-auth-user",
                SetHostedAccess {
                    enabled: true,
                    expected_generation: 0
                }
            )
            .await
            .is_err()
        );
    })
    .await;
}
