use super::test_support::{
    TestClaims, authenticator, invalidly_signed_claims, now, register_user_record, signed_claims,
};
use super::*;

#[tokio::test]
async fn accepts_valid_standard_workos_access_token() {
    let claims = TestClaims::valid("user_valid", Some(test_support::OPERATOR_ORG_ID));
    let session = authenticator()
        .verify_access_token(&signed_claims(claims))
        .await
        .unwrap();
    assert_eq!(session.subject, "user_valid");
    assert_eq!(
        session.organization_id.as_deref(),
        Some(test_support::OPERATOR_ORG_ID)
    );
}

#[tokio::test]
async fn rejects_expired_access_token() {
    let claims = TestClaims::valid("user_expired", None).with_expiry(now() - 1);
    assert_eq!(
        authenticator()
            .verify_access_token(&signed_claims(claims))
            .await,
        Err(WorkosAuthError::InvalidToken)
    );
}

#[tokio::test]
async fn rejects_wrong_issuer() {
    let claims = TestClaims::valid("user_wrong_issuer", None)
        .with_issuer("https://wrong-issuer.test.invalid");
    assert_eq!(
        authenticator()
            .verify_access_token(&signed_claims(claims))
            .await,
        Err(WorkosAuthError::InvalidToken)
    );
}

#[tokio::test]
async fn rejects_wrong_client_id() {
    let claims = TestClaims::valid("user_wrong_client", None).with_client_id("client_wrong");
    assert_eq!(
        authenticator()
            .verify_access_token(&signed_claims(claims))
            .await,
        Err(WorkosAuthError::InvalidToken)
    );
}

#[tokio::test]
async fn rejects_invalid_signature() {
    let claims = TestClaims::valid("user_invalid_signature", None);
    assert_eq!(
        authenticator()
            .verify_access_token(&invalidly_signed_claims(claims))
            .await,
        Err(WorkosAuthError::InvalidToken)
    );
}

#[tokio::test]
async fn rotated_kid_triggers_jwks_refresh_and_accepts_new_key() {
    let source = test_support::isolated_source();
    let authenticator = test_support::authenticator_for_source(source.clone());
    // Prime the key cache with the standard key set.
    authenticator
        .verify_access_token(&signed_claims(TestClaims::valid("user_cache_prime", None)))
        .await
        .unwrap();

    // WorkOS rotates to a brand-new key id; the stale cache misses.
    let rotated = test_support::generate_key("test-key-rotated");
    let rotated_token =
        test_support::encode_claims(TestClaims::valid("user_rotated_kid", None), &rotated);
    test_support::install_jwks_keys(&source, vec![rotated]);
    let session = authenticator
        .verify_access_token(&rotated_token)
        .await
        .unwrap();
    assert_eq!(session.subject, "user_rotated_kid");
}

#[tokio::test]
async fn same_kid_rotation_refreshes_once_and_accepts_replacement_key() {
    let source = test_support::isolated_source();
    let authenticator = test_support::authenticator_for_source(source.clone());
    authenticator
        .verify_access_token(&signed_claims(TestClaims::valid(
            "user_same_kid_prime",
            None,
        )))
        .await
        .unwrap();

    // WorkOS replaces the key material but keeps the key id: the cached
    // key matches the kid yet fails the signature, which must trigger one
    // refresh-and-retry instead of a final rejection.
    let replacement = test_support::generate_key(&test_support::standard_kid());
    let replacement_token = test_support::encode_claims(
        TestClaims::valid("user_same_kid_rotated", None),
        &replacement,
    );
    test_support::install_jwks_keys(&source, vec![replacement]);
    let session = authenticator
        .verify_access_token(&replacement_token)
        .await
        .unwrap();
    assert_eq!(session.subject, "user_same_kid_rotated");
}

#[tokio::test]
async fn jwks_refresh_failure_fails_closed_and_keeps_last_good_keys() {
    let source = test_support::isolated_source();
    let authenticator = test_support::authenticator_for_source(source.clone());
    let cached_session = signed_claims(TestClaims::valid("user_last_good", None));
    authenticator
        .verify_access_token(&cached_session)
        .await
        .unwrap();

    // A degraded WorkOS serves an empty key set. An unknown-kid token
    // must surface Unavailable (503), not InvalidToken (401).
    test_support::install_jwks_keys(&source, Vec::new());
    let unknown_kid = test_support::generate_key("test-key-unknown-during-outage");
    assert_eq!(
        authenticator
            .verify_access_token(&test_support::encode_claims(
                TestClaims::valid("user_during_outage", None),
                &unknown_kid
            ))
            .await,
        Err(WorkosAuthError::Unavailable)
    );

    // The failed refresh must not have dropped the last-good key set:
    // the previously cached key still verifies without another fetch.
    assert!(
        authenticator
            .verify_access_token(&cached_session)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn rejects_empty_subject() {
    let claims = TestClaims::valid("", None);
    assert_eq!(
        authenticator()
            .verify_access_token(&signed_claims(claims))
            .await,
        Err(WorkosAuthError::InvalidToken)
    );
}

#[tokio::test]
async fn user_lookup_fails_closed_for_unknown_or_mismatched_records() {
    let authenticator = authenticator();
    let unknown = authenticator
        .verify_access_token(&signed_claims(TestClaims::valid("user_unknown", None)))
        .await
        .unwrap();
    assert_eq!(
        authenticator.verified_user(&unknown.subject).await,
        Err(WorkosAuthError::UnknownUser)
    );

    register_user_record(
        "user_expected",
        "user_different",
        "expected@finite.vip",
        true,
    );
    let mismatched = authenticator
        .verify_access_token(&signed_claims(TestClaims::valid("user_expected", None)))
        .await
        .unwrap();
    assert_eq!(
        authenticator.verified_user(&mismatched.subject).await,
        Err(WorkosAuthError::InvalidToken)
    );
}

#[test]
fn service_route_credentials_must_be_distinct() {
    let result = CoreAuth::new(authenticator(), "same", "same", "usage");
    assert_eq!(
        result.err(),
        Some(AuthConfigError::ServiceCredentialsMustBeDistinct)
    );
}

fn runner_credential(
    credential_id: &str,
    token: &str,
    runner_id: &str,
    runner_classes: Vec<RunnerClass>,
    source_host_id: &str,
    revoked: bool,
) -> RunnerCredentialConfig {
    RunnerCredentialConfig {
        credential_id: credential_id.to_string(),
        token: token.to_string(),
        runner_id: runner_id.to_string(),
        runner_classes,
        source_host_id: source_host_id.to_string(),
        revoked,
    }
}

#[test]
fn targeted_creation_requires_an_active_host_bound_kata_credential() {
    let auth = CoreAuth::new_with_runner_credentials(
        authenticator(),
        "service",
        vec![
            runner_credential(
                "live",
                "live-token",
                "live-runner",
                vec![RunnerClass::Kata],
                "live-host",
                false,
            ),
            runner_credential(
                "revoked",
                "revoked-token",
                "revoked-runner",
                vec![RunnerClass::Kata],
                "revoked-host",
                true,
            ),
            runner_credential(
                "phala",
                "phala-token",
                "phala-runner",
                vec![RunnerClass::Phala],
                "phala-host",
                false,
            ),
        ],
        "usage",
    )
    .unwrap();
    assert!(auth.has_kata_host("live-host"));
    for host in ["revoked-host", "phala-host", "missing-host"] {
        assert!(!auth.has_kata_host(host));
    }
}

#[test]
fn runner_keyring_accepts_overlap_and_rejects_only_revoked_credentials() {
    let auth = CoreAuth::new_with_runner_credentials(
        authenticator(),
        "service-token",
        vec![
            runner_credential(
                "phala-current",
                "phala-current-token",
                "phala-worker-1",
                vec![RunnerClass::Phala],
                "phala-host-1",
                false,
            ),
            runner_credential(
                "phala-next",
                "phala-next-token",
                "phala-worker-1",
                vec![RunnerClass::Phala],
                "phala-host-1",
                false,
            ),
            runner_credential(
                "phala-revoked",
                "phala-revoked-token",
                "phala-worker-1",
                vec![RunnerClass::Phala],
                "phala-host-1",
                true,
            ),
        ],
        "usage-token",
    )
    .unwrap();

    for (token, credential_id) in [
        ("phala-current-token", "phala-current"),
        ("phala-next-token", "phala-next"),
    ] {
        let verified = auth.verify_runner_credential(token).unwrap();
        assert_eq!(verified.credential_id, credential_id);
        assert_eq!(verified.runner_id, "phala-worker-1");
        assert_eq!(verified.runner_classes, vec![RunnerClass::Phala]);
        assert_eq!(verified.source_host_id, "phala-host-1");
        assert!(!verified.legacy_kata_compatibility);
    }
    assert!(
        auth.verify_runner_credential("phala-revoked-token")
            .is_none()
    );
    assert!(auth.verify_runner_credential("unknown-token").is_none());
}

#[test]
fn runner_keyring_rejects_empty_or_duplicate_class_sets() {
    for classes in [Vec::new(), vec![RunnerClass::Phala, RunnerClass::Phala]] {
        let result = CoreAuth::new_with_runner_credentials(
            authenticator(),
            "service-token",
            vec![runner_credential(
                "phala-current",
                "phala-current-token",
                "phala-worker-1",
                classes,
                "phala-host-1",
                false,
            )],
            "usage-token",
        );
        assert_eq!(
            result.err(),
            Some(AuthConfigError::InvalidRunnerCredentialKeyring)
        );
    }
}

#[test]
fn runner_keyring_metadata_resolves_only_named_secret_environment_variables() {
    let metadata = r#"[{"credentialId":"phala-current","tokenEnv":"FC_CORE_RUNNER_CREDENTIAL_TOKEN_PHALA_CURRENT","runnerId":"phala-worker-1","runnerClasses":["phala"],"sourceHostId":"phala-host-1"}]"#;
    let credentials = runner_credential_configs_from_metadata(metadata, |name| {
        (name == "FC_CORE_RUNNER_CREDENTIAL_TOKEN_PHALA_CURRENT")
            .then(|| "resolved-secret-material".to_string())
    })
    .unwrap();
    assert_eq!(credentials.len(), 1);
    assert_eq!(credentials[0].credential_id, "phala-current");
    assert_eq!(credentials[0].token, "resolved-secret-material");
    assert_eq!(credentials[0].runner_classes, vec![RunnerClass::Phala]);

    let invalid_metadata = r#"[{"credentialId":"phala-current","tokenEnv":"UNSCOPED_TOKEN","runnerId":"phala-worker-1","runnerClasses":["phala"],"sourceHostId":"phala-host-1"}]"#;
    assert_eq!(
        runner_credential_configs_from_metadata(invalid_metadata, |_| Some("secret".into())).err(),
        Some(AuthConfigError::InvalidRunnerCredentialKeyring)
    );
}

#[test]
fn legacy_runner_token_is_narrowly_bound_to_deployed_kata_worker() {
    let auth = CoreAuth::new(
        authenticator(),
        "service-token",
        "legacy-runner-token",
        "usage-token",
    )
    .unwrap();
    let verified = auth
        .verify_runner_credential("legacy-runner-token")
        .unwrap();
    assert_eq!(verified.runner_id, LEGACY_KATA_RUNNER_ID);
    assert_eq!(verified.runner_classes, vec![RunnerClass::Kata]);
    assert_eq!(verified.source_host_id, LEGACY_KATA_SOURCE_HOST_ID);
    assert!(verified.legacy_kata_compatibility);
}
