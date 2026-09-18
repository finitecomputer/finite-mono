use super::*;
use crate::store::hosted_hermes::{ApplyStatus, HostedReport, SetHostedAccess};

#[tokio::test]
async fn automatic_hosted_access_enrollment_preserves_existing_intent_and_revocation() {
    for already_enrolled in [false, true] {
        with_isolated_postgres(move |db| async move {
            let creation = requested(&db).await;
            if already_enrolled {
                // An N-1 Core provisioned a disabled credential. Upgrading Core
                // must not reinterpret existing serving intent as a new default.
                db.provision_runtime_credential(provision(&creation))
                    .await
                    .unwrap();
            }
            let runtime = register(&db, &creation).await;
            complete(&db, &creation).await.unwrap();
            let lease = upgrade(&db, &creation).await;
            let input = || ProvisionUpgradeCredential {
                prepare_hosted_access: true,
                ..upgrade_input(&lease)
            };
            let mut wrong_lease = input();
            wrong_lease.lease_token = "wrong-lease".into();
            assert!(db.provision_upgrade_credential(wrong_lease).await.is_err());
            assert_eq!(
                db.hosted_access(&runtime, "runtime-auth-user")
                    .await
                    .unwrap()
                    .enrolled,
                already_enrolled
            );

            let secret = db
                .provision_upgrade_credential(input())
                .await
                .unwrap()
                .secret;
            let access = db
                .hosted_access(&runtime, "runtime-auth-user")
                .await
                .unwrap();
            assert_eq!(access.enabled, !already_enrolled);
            assert_eq!(access.generation, 1);
            assert!(access.applied_generation.is_none());
            assert!(
                db.hosted_route_targets_for_host("auth-host")
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(
                db.provision_upgrade_credential(input())
                    .await
                    .unwrap()
                    .secret
                    == secret
            );

            if !already_enrolled {
                db.report_hosted(
                    &secret,
                    HostedReport {
                        generation: 1,
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
                db.set_hosted_access(
                    &runtime,
                    "runtime-auth-user",
                    SetHostedAccess {
                        enabled: false,
                        expected_generation: 1,
                    },
                )
                .await
                .unwrap();
                // Both candidate and predecessor-style retries preserve the
                // operator's disable and never regenerate signing material.
                db.provision_upgrade_credential(input()).await.unwrap();
                db.provision_upgrade_credential(upgrade_input(&lease))
                    .await
                    .unwrap();
                let access = db
                    .hosted_access(&runtime, "runtime-auth-user")
                    .await
                    .unwrap();
                assert!(!access.enabled);
                assert_eq!(access.generation, 2);
            }
            db.revoke_runtime_credential(&runtime, &creation)
                .await
                .unwrap();
            assert!(db.provision_upgrade_credential(input()).await.is_err());
            assert!(
                db.authenticate_runtime_credential(&secret)
                    .await
                    .unwrap()
                    .is_none()
            );
        })
        .await;
    }
}

#[tokio::test]
async fn automatic_hosted_access_creation_retries_keep_native_credentials() {
    with_isolated_postgres(|db| async move {
        let creation = requested(&db).await;
        let input = || ProvisionRuntimeCredential {
            prepare_hosted_access: true,
            ..provision(&creation)
        };
        let (first, second) = tokio::join!(
            db.provision_runtime_credential(input()),
            db.provision_runtime_credential(input()),
        );
        let secret = first.unwrap().secret;
        assert!(secret == second.unwrap().secret);
        let runtime = register(&db, &creation).await;
        let origins = crate::hosted_hermes::HostedHermesOrigins::from_json(
            r#"{"auth-host":"https://agent.example.test"}"#,
        )
        .unwrap();
        let before = db.hosted_desired(&secret, &origins).await.unwrap().unwrap();
        // Replaying a launch through predecessor behavior leaves candidate
        // intent and credentials intact; there is no persisted-schema change.
        db.provision_runtime_credential(provision(&creation))
            .await
            .unwrap();
        let after = db.hosted_desired(&secret, &origins).await.unwrap().unwrap();
        assert!(before.password == after.password && before.signing_secret == after.signing_secret);
        assert!(after.enabled);
        assert_eq!(after.generation, 1);
        db.set_hosted_access(
            &runtime,
            "runtime-auth-user",
            SetHostedAccess {
                enabled: false,
                expected_generation: 1,
            },
        )
        .await
        .unwrap();
        db.provision_runtime_credential(input()).await.unwrap();
        let after = db.hosted_desired(&secret, &origins).await.unwrap().unwrap();
        assert!(!after.enabled && after.password.is_none() && after.signing_secret.is_none());
    })
    .await;
}
