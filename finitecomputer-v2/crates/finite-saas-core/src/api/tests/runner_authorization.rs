use super::*;

#[test]
fn runner_capability_authorization_is_explicit_and_legacy_kata_is_narrow() {
    let current_kata = VerifiedRunnerCredential {
        credential_id: "kata-current".to_string(),
        runner_id: "kata-worker".to_string(),
        runner_classes: vec![RunnerClass::Kata],
        source_host_id: "kata-host".to_string(),
        legacy_kata_compatibility: false,
    };
    assert!(authorize_runner_capacity(&current_kata, None).is_err());
    assert!(
        authorize_runner_runtime_capabilities(&current_kata, None).is_err(),
        "current credentials must advertise on registration and completion"
    );
    assert!(
        authorize_runner_capacity(
            &current_kata,
            Some(RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Kata],
                ..RunnerLeaseCapacity::default()
            })
        )
        .is_err()
    );

    let legacy_kata = VerifiedRunnerCredential {
        legacy_kata_compatibility: true,
        ..current_kata.clone()
    };
    let compatibility = authorize_runner_capacity(&legacy_kata, None).unwrap();
    let RuntimeCapabilitiesEnvelope::V1(capabilities) = compatibility.runtime_capabilities.unwrap();
    assert_eq!(
        capabilities,
        RuntimeCapabilitiesV1 {
            native_hermes_chat: false,
            restart: true,
            recover_known_good_chat: false,
            runtime_upgrade: true,
            stop: true,
            runtime_retirement: false,
        }
    );
    assert_eq!(
        authorize_runner_runtime_capabilities(&legacy_kata, None).unwrap(),
        legacy_kata_runtime_capabilities()
    );

    assert!(
        authorize_runner_capacity(
            &current_kata,
            Some(RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Kata],
                runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                    RuntimeCapabilitiesV1 {
                        recover_known_good_chat: true,
                        ..RuntimeCapabilitiesV1::default()
                    }
                )),
                ..RunnerLeaseCapacity::default()
            })
        )
        .is_ok()
    );
    let retirement = RuntimeCapabilitiesV1 {
        runtime_retirement: true,
        ..RuntimeCapabilitiesV1::default()
    };
    assert!(
        authorize_runner_capacity(
            &current_kata,
            Some(RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Kata],
                runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(retirement)),
                ..RunnerLeaseCapacity::default()
            })
        )
        .is_ok()
    );

    let current_phala = VerifiedRunnerCredential {
        credential_id: "phala-current".to_string(),
        runner_id: "phala-worker".to_string(),
        runner_classes: vec![RunnerClass::Phala],
        source_host_id: "phala-host".to_string(),
        legacy_kata_compatibility: false,
    };
    assert!(
        authorize_runner_capacity(
            &current_phala,
            Some(RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Phala],
                runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                    RuntimeCapabilitiesV1 {
                        runtime_upgrade: true,
                        ..RuntimeCapabilitiesV1::default()
                    }
                )),
                ..RunnerLeaseCapacity::default()
            })
        )
        .is_err()
    );
    assert!(
        authorize_runner_capacity(
            &current_phala,
            Some(RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Phala],
                runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                    RuntimeCapabilitiesV1 {
                        runtime_retirement: true,
                        ..RuntimeCapabilitiesV1::default()
                    }
                )),
                ..RunnerLeaseCapacity::default()
            })
        )
        .is_err()
    );
    assert!(
        authorize_runner_capacity(
            &current_phala,
            Some(RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Phala],
                runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                    RuntimeCapabilitiesV1 {
                        recover_known_good_chat: true,
                        ..RuntimeCapabilitiesV1::default()
                    }
                )),
                ..RunnerLeaseCapacity::default()
            })
        )
        .is_err()
    );
    assert!(
        authorize_runner_capacity(
            &current_phala,
            Some(RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Phala],
                runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                    RuntimeCapabilitiesV1 {
                        restart: true,
                        stop: true,
                        ..RuntimeCapabilitiesV1::default()
                    }
                )),
                ..RunnerLeaseCapacity::default()
            })
        )
        .is_ok()
    );
}

#[tokio::test]
async fn runner_keyring_enforces_worker_class_source_and_revocation_bindings() {
    with_isolated_postgres(|db| async move {
        let auth = core_auth_with_runner_credentials(
            "service-token",
            vec![
                runner_credential_config(
                    "kata-current",
                    "kata-current-token",
                    "kata-worker-1",
                    &[RunnerClass::Kata],
                    "kata-host-1",
                    false,
                ),
                runner_credential_config(
                    "kata-next",
                    "kata-next-token",
                    "kata-worker-1",
                    &[RunnerClass::Kata],
                    "kata-host-1",
                    false,
                ),
                runner_credential_config(
                    "kata-revoked",
                    "kata-revoked-token",
                    "kata-worker-1",
                    &[RunnerClass::Kata],
                    "kata-host-1",
                    true,
                ),
                runner_credential_config(
                    "phala-current",
                    "phala-current-token",
                    "phala-worker-1",
                    &[RunnerClass::Phala],
                    "phala-host-1",
                    false,
                ),
            ],
            "usage-token",
        );
        let app = router(db.store.clone(), auth);

        for token in ["kata-current-token", "kata-next-token"] {
            let headers = vec![("authorization".to_string(), format!("Bearer {token}"))];
            let (status, body) = send_json(
                &app,
                "POST",
                "/api/core/v1/agent-creation-requests/lease",
                &headers,
                Some(serde_json::json!({
                    "runnerId": "kata-worker-1",
                    "leaseToken": "lease-token",
                    "runnerCapacity": runner_capacity_json(RunnerClass::Kata)
                })),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert!(body.is_null(), "empty queue must return no lease");
        }

        let kata = vec![(
            "authorization".to_string(),
            "Bearer kata-current-token".to_string(),
        )];
        for body in [
            serde_json::json!({
                "runnerId": "phala-worker-1",
                "leaseToken": "lease-token",
                "runnerCapacity": { "runnerClasses": ["kata"] }
            }),
            serde_json::json!({
                "runnerId": "kata-worker-1",
                "leaseToken": "lease-token",
                "runnerCapacity": { "runnerClasses": [] }
            }),
            serde_json::json!({
                "runnerId": "kata-worker-1",
                "leaseToken": "lease-token",
                "runnerCapacity": runner_capacity_json(RunnerClass::Phala)
            }),
        ] {
            let (status, _) = send_json(
                &app,
                "POST",
                "/api/core/v1/agent-creation-requests/lease",
                &kata,
                Some(body),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN);
        }

        let phala = vec![(
            "authorization".to_string(),
            "Bearer phala-current-token".to_string(),
        )];
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/agent-creation-requests/lease",
            &phala,
            Some(serde_json::json!({
                "runnerId": "phala-worker-1",
                "leaseToken": "lease-token",
                "runnerCapacity": { "runnerClasses": ["kata"] }
            })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, body) = send_json(
            &app,
            "POST",
            "/api/core/v1/runtime-control-requests/lease",
            &phala,
            Some(serde_json::json!({
                "runnerId": "phala-worker-1",
                "leaseToken": "lease-token",
                "sourceHostId": "phala-host-1",
                "runnerCapacity": runner_capacity_json(RunnerClass::Phala)
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.is_null());

        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/runtime-control-requests/lease",
            &phala,
            Some(serde_json::json!({
                "runnerId": "phala-worker-1",
                "leaseToken": "lease-token",
                "sourceHostId": "kata-host-1",
                "runnerCapacity": { "runnerClasses": ["phala"] }
            })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let revoked = vec![(
            "authorization".to_string(),
            "Bearer kata-revoked-token".to_string(),
        )];
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/agent-creation-requests/lease",
            &revoked,
            Some(serde_json::json!({
                "runnerId": "kata-worker-1",
                "leaseToken": "lease-token",
                "runnerCapacity": { "runnerClasses": ["kata"] }
            })),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    })
    .await;
}

#[tokio::test]
async fn core_api_recovery_and_capacity_release_require_substrate_runner_authority() {
    with_isolated_postgres(|db| async move {
        let auth = core_auth_with_runner_credentials(
            "service",
            vec![
                runner_credential_config(
                    "kata",
                    "kata-token",
                    "kata",
                    &[RunnerClass::Kata],
                    "host",
                    false,
                ),
                runner_credential_config(
                    "substrate",
                    "substrate-token",
                    "substrate",
                    &[RunnerClass::Substrate],
                    "host",
                    false,
                ),
            ],
            "usage",
        );
        let app = router(db.store.clone(), auth);
        for (token, runner, expected) in [
            ("service", "substrate", StatusCode::UNAUTHORIZED),
            ("invalid", "substrate", StatusCode::UNAUTHORIZED),
            ("kata-token", "kata", StatusCode::FORBIDDEN),
            ("substrate-token", "another-runner", StatusCode::FORBIDDEN),
            ("substrate-token", "substrate", StatusCode::NOT_FOUND),
        ] {
            let (status, _) = send_json(
                &app,
                "POST",
                "/api/core/v1/agent-creation-requests/missing/release",
                &[("authorization".into(), format!("Bearer {token}"))],
                Some(serde_json::json!({ "runnerId": runner, "leaseToken": "first" })),
            )
            .await;
            assert_eq!(status, expected);
        }
        for (token, expected) in [
            ("service", StatusCode::UNAUTHORIZED),
            ("kata-token", StatusCode::FORBIDDEN),
            ("substrate-token", StatusCode::OK),
        ] {
            let (status, body) = send_json(
                &app,
                "POST",
                "/api/core/v1/runtimes/runtime_missing/recover",
                &[("authorization".into(), format!("Bearer {token}"))],
                Some(serde_json::json!({})),
            )
            .await;
            assert_eq!(status, expected);
            if status == StatusCode::OK {
                assert!(body.is_null());
            }
        }
    })
    .await;
}
