//! Private Brain identity listener: credential separation, router isolation
//! and the account-authenticated observation → scoped description path.

use super::*;
use crate::brain_identity::{
    BrainIdentityConfig, BrainIdentityConfigError, DESCRIPTION_CREDENTIAL_HEADER,
    DESCRIPTIONS_PATH, DESCRIPTIONS_VERSION, DESCRIPTIONS_VERSION_V2, OBSERVATION_CREDENTIAL_HEADER,
    OBSERVATION_PATH,
    OBSERVATION_VERSION,
};

const SERVER: &str = "https://brain.test";
const OBSERVATION_TOKEN: &str = "brain-observation-test-token";
const DESCRIPTION_TOKEN: &str = "brain-description-test-token";

fn config(auth: &CoreAuth) -> BrainIdentityConfig {
    BrainIdentityConfig::from_values(
        Some("127.0.0.1:0".to_string()),
        Some(SERVER.to_string()),
        Some(OBSERVATION_TOKEN.to_string()),
        Some(DESCRIPTION_TOKEN.to_string()),
        auth,
    )
    .unwrap()
    .unwrap()
}

fn identity_app(store: CoreStore) -> Router {
    let auth = scoped_test_auth();
    let config = config(&auth);
    brain_identity_router(store, auth, config)
}

fn key(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn observation_body(operation: &str, human: &str) -> serde_json::Value {
    serde_json::json!({
        "version": OBSERVATION_VERSION,
        "operationId": format!("operation_{operation}_0000"),
        "brainServer": SERVER,
        "brainId": "brain_alpha",
        "observedAt": crate::current_time_iso().unwrap(),
        "actionKind": "humanHostedAction",
        "observedHumanPublicKeyHex": human,
        "participatingPublicKeyHex": human,
    })
}

fn description_body(keys: &[String]) -> serde_json::Value {
    serde_json::json!({
        "version": DESCRIPTIONS_VERSION,
        "brainServer": SERVER,
        "brainId": "brain_alpha",
        "requestedByPublicKeyHex": key(0xee),
        "publicKeysHex": keys,
    })
}

async fn post(
    app: &Router,
    path: &str,
    headers: &[(&str, String)],
    body: Vec<u8>,
) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder().method("POST").uri(path);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let response = app
        .clone()
        .oneshot(
            request
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

fn bearer(email: &str) -> String {
    format!("Bearer {}", access_token(email, true, None))
}

#[test]
fn configuration_is_all_or_nothing_and_never_reuses_a_credential() {
    let auth = scoped_test_auth();
    let build = |observation: &str, description: &str, server: &str| {
        BrainIdentityConfig::from_values(
            Some("127.0.0.1:0".to_string()),
            Some(server.to_string()),
            Some(observation.to_string()),
            Some(description.to_string()),
            &auth,
        )
    };
    assert!(
        BrainIdentityConfig::from_values(None, None, None, None, &auth)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        BrainIdentityConfig::from_values(
            Some("127.0.0.1:0".to_string()),
            None,
            Some(OBSERVATION_TOKEN.to_string()),
            None,
            &auth,
        )
        .unwrap_err(),
        BrainIdentityConfigError::Incomplete
    );
    let usage = scoped_token("finite-private-usage");
    let runner = scoped_token("runner");
    for (observation, description) in [
        (OBSERVATION_TOKEN, OBSERVATION_TOKEN),
        (TOKEN, DESCRIPTION_TOKEN),
        (OBSERVATION_TOKEN, usage.as_str()),
        (runner.as_str(), DESCRIPTION_TOKEN),
        (OBSERVATION_TOKEN, BOUNDARY_RUNNER_TOKEN),
    ] {
        assert_eq!(
            build(observation, description, SERVER).unwrap_err(),
            BrainIdentityConfigError::CredentialsMustBeDistinct
        );
    }
    assert_eq!(
        build(OBSERVATION_TOKEN, DESCRIPTION_TOKEN, "https://brain.test/").unwrap_err(),
        BrainIdentityConfigError::InvalidBrainServer
    );
}

#[tokio::test]
async fn routes_exist_only_on_the_private_identity_router() {
    with_isolated_postgres(|db| async move {
        let account_router = router(db.store.clone(), scoped_test_auth());
        let runtime = runtime_router(db.store.clone(), Default::default());
        let description = serde_json::to_vec(&description_body(&[key(1)])).unwrap();
        for app in [&account_router, &runtime] {
            for path in [OBSERVATION_PATH, DESCRIPTIONS_PATH] {
                let (status, _) = post(
                    app,
                    path,
                    &[
                        (DESCRIPTION_CREDENTIAL_HEADER, DESCRIPTION_TOKEN.to_string()),
                        ("authorization", format!("Bearer {TOKEN}")),
                    ],
                    description.clone(),
                )
                .await;
                assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
            }
        }
        let identity = identity_app(db.store.clone());
        let (status, _) = post(&identity, "/api/core/v1/me", &[], Vec::new()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await;
}

#[tokio::test]
async fn each_route_accepts_only_its_own_credentials() {
    with_isolated_postgres(|db| async move {
        let app = identity_app(db.store.clone());
        let observation = serde_json::to_vec(&observation_body("auth", &key(1))).unwrap();
        let account = bearer("sam@example.org");
        let refused = [
            vec![("authorization", account.clone())],
            vec![(OBSERVATION_CREDENTIAL_HEADER, OBSERVATION_TOKEN.to_string())],
            vec![
                (OBSERVATION_CREDENTIAL_HEADER, TOKEN.to_string()),
                ("authorization", account.clone()),
            ],
            vec![
                (OBSERVATION_CREDENTIAL_HEADER, DESCRIPTION_TOKEN.to_string()),
                ("authorization", account.clone()),
            ],
            vec![
                (OBSERVATION_CREDENTIAL_HEADER, OBSERVATION_TOKEN.to_string()),
                ("authorization", format!("Bearer {TOKEN}")),
            ],
            vec![
                (OBSERVATION_CREDENTIAL_HEADER, OBSERVATION_TOKEN.to_string()),
                ("authorization", account.clone()),
                (WORKOS_EMAIL_HEADER, "spoofed@example.org".to_string()),
            ],
        ];
        for headers in &refused {
            let (status, _) = post(&app, OBSERVATION_PATH, headers, observation.clone()).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{headers:?}");
        }
        assert_eq!(db.all("brain_account_observation_receipts").await.len(), 0);

        let description = serde_json::to_vec(&description_body(&[key(1)])).unwrap();
        for headers in [
            vec![],
            vec![("authorization", format!("Bearer {DESCRIPTION_TOKEN}"))],
            vec![(DESCRIPTION_CREDENTIAL_HEADER, OBSERVATION_TOKEN.to_string())],
            vec![(DESCRIPTION_CREDENTIAL_HEADER, TOKEN.to_string())],
            vec![(DESCRIPTION_CREDENTIAL_HEADER, scoped_token("runner"))],
        ] {
            let (status, _) = post(&app, DESCRIPTIONS_PATH, &headers, description.clone()).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{headers:?}");
        }
    })
    .await;
}

#[tokio::test]
async fn account_observation_then_scoped_description_over_http() {
    with_isolated_postgres(|db| async move {
        let app = identity_app(db.store.clone());
        let observe = [
            (OBSERVATION_CREDENTIAL_HEADER, OBSERVATION_TOKEN.to_string()),
            ("authorization", bearer("dana@acme.example")),
        ];
        let body = serde_json::to_vec(&observation_body("dana", &key(1))).unwrap();
        // Not yet enrolled: refused, and the route enrolls nobody.
        let (status, refused) = post(&app, OBSERVATION_PATH, &observe, body.clone()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
        assert!(db.all("users").await.is_empty());
        assert!(db.all("customer_orgs").await.is_empty());
        // Normal first contact enrolls the account through the account router.
        let me = router(db.store.clone(), scoped_test_auth())
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me")
                    .header("authorization", bearer("dana@acme.example"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(me.status(), StatusCode::OK);
        let (status, first) = post(&app, OBSERVATION_PATH, &observe, body.clone()).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["outcome"], "recorded");
        let (status, retry) = post(&app, OBSERVATION_PATH, &observe, body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(retry, first);

        let describe = [(DESCRIPTION_CREDENTIAL_HEADER, DESCRIPTION_TOKEN.to_string())];
        let request = serde_json::to_vec(&description_body(&[key(1), key(2)])).unwrap();
        let (status, response) = post(&app, DESCRIPTIONS_PATH, &describe, request).await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_eq!(response["version"], DESCRIPTIONS_VERSION);
        assert_eq!(response["results"][0]["state"], "resolved");
        assert_eq!(response["results"][0]["kind"], "human");
        assert_eq!(response["results"][0]["accountEmail"], "dana@acme.example");
        assert_eq!(
            response["results"][1],
            serde_json::json!({"publicKeyHex": key(2), "state": "notShared"})
        );
        // v2 answers in v2. An unknown version gets the exact error Brain
        // treats as "fall back to v1"; nothing else downgrades.
        let mut v2 = description_body(&[key(1)]);
        v2["version"] = serde_json::json!(DESCRIPTIONS_VERSION_V2);
        let (status, response) =
            post(&app, DESCRIPTIONS_PATH, &describe, serde_json::to_vec(&v2).unwrap()).await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_eq!(response["version"], DESCRIPTIONS_VERSION_V2);
        assert_eq!(response["results"][0]["accountEmail"], "dana@acme.example");
        v2["version"] = serde_json::json!("finite-core-brain-identity-descriptions-v3");
        let (status, response) =
            post(&app, DESCRIPTIONS_PATH, &describe, serde_json::to_vec(&v2).unwrap()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            response,
            serde_json::json!({"error": "unsupported descriptions version"})
        );

        // A browser-style claim of another key for the same account in a
        // human action is refused before any write.
        let mut mismatched = observation_body("mismatch", &key(3));
        mismatched["participatingPublicKeyHex"] = serde_json::json!(key(4));
        let (status, _) = post(
            &app,
            OBSERVATION_PATH,
            &observe,
            serde_json::to_vec(&mismatched).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    })
    .await;
}

#[tokio::test]
async fn description_inputs_are_bounded_and_strict() {
    with_isolated_postgres(|db| async move {
        let app = identity_app(db.store.clone());
        let describe = [(DESCRIPTION_CREDENTIAL_HEADER, DESCRIPTION_TOKEN.to_string())];
        let too_many = (0..=100_u16)
            .map(|n| format!("{n:064x}"))
            .collect::<Vec<_>>();
        let mut wrong_server = description_body(&[key(1)]);
        wrong_server["brainServer"] = serde_json::json!("https://brain.test/");
        let mut email_search = description_body(&[key(1)]);
        email_search["email"] = serde_json::json!("sam@example.org");
        let mut npub_key = description_body(&[key(1)]);
        npub_key["publicKeysHex"] = serde_json::json!(["npub1qqqq"]);
        for body in [
            description_body(&too_many),
            description_body(&[key(1), key(1)]),
            description_body(&[]),
            wrong_server,
            email_search,
            npub_key,
        ] {
            let (status, _) = post(
                &app,
                DESCRIPTIONS_PATH,
                &describe,
                serde_json::to_vec(&body).unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        }
        let (status, _) = post(
            &app,
            DESCRIPTIONS_PATH,
            &describe,
            vec![b' '; crate::brain_identity::MAX_DESCRIPTION_BODY_BYTES + 1],
        )
        .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    })
    .await;
}
