//! Loopback exact-key name lookup contract. Trusted products (the Brain access
//! report) send an already-authorized batch of keys and receive only active
//! published names bound to those exact keys, behind a read-only credential
//! that is distinct from the operator token.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use finite_identity::authority::{
    AuthorityConfig, AuthorityState, DevMailer, FixedClock, IdentityStore, MAX_NAME_LOOKUP_KEYS,
    Mailer, NAME_LOOKUP_PATH, NAME_LOOKUP_TOKEN_HEADER, public_router, router,
};
use finite_identity::npub;
use tower::ServiceExt as _;

const NOW: u64 = 1_788_000_000;
const OPERATOR_TOKEN: &str = "synthetic-operator-credential";
const LOOKUP_TOKEN: &str = "synthetic-name-lookup-credential";
const MAILBOX_KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const AGENT_KEY: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const DISABLED_KEY: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const UNKNOWN_KEY: &str = "4444444444444444444444444444444444444444444444444444444444444444";

fn state(store: IdentityStore, lookup_token: Option<&str>) -> AuthorityState {
    AuthorityState::new(
        store,
        Arc::new(DevMailer) as Arc<dyn Mailer>,
        FixedClock::new(NOW),
        AuthorityConfig {
            external_base_url: "https://identity.test".to_owned(),
            finite_vip_domain: "finite.vip".to_owned(),
            email_challenge_ttl_seconds: 600,
            operator_token: Some(OPERATOR_TOKEN.to_owned()),
        },
    )
    .with_name_lookup_token(lookup_token.map(ToOwned::to_owned))
}

async fn post(
    app: axum::Router,
    path: &str,
    body: serde_json::Value,
    headers: &[(&str, &str)],
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = app
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

fn agent_npub() -> String {
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&AGENT_KEY[index * 2..index * 2 + 2], 16).unwrap();
    }
    npub::encode(&bytes)
}

async fn seeded_router(lookup_token: Option<&str>) -> axum::Router {
    let store = IdentityStore::open_memory().unwrap();
    store
        .bind_vip_email("mailbox-one@finite.vip", MAILBOX_KEY, NOW - 10)
        .unwrap();
    store
        .bind_vip_email("disabled-one@finite.vip", DISABLED_KEY, NOW - 10)
        .unwrap();
    store
        .disable_vip_email("disabled-one@finite.vip", NOW - 5)
        .unwrap();
    let app = router(state(store, lookup_token));
    let (status, _) = post(
        app.clone(),
        "/api/v1/operator/agent-email-bindings",
        serde_json::json!({ "email": "agent-one@finite.vip", "agent_npub": agent_npub() }),
        &[("x-finite-operator-token", OPERATOR_TOKEN)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    app
}

#[tokio::test]
async fn name_lookup_returns_only_active_names_for_exact_keys() {
    let app = seeded_router(Some(LOOKUP_TOKEN)).await;
    let (status, body) = post(
        app,
        NAME_LOOKUP_PATH,
        serde_json::json!({ "pubkeys": [MAILBOX_KEY, agent_npub(), DISABLED_KEY, UNKNOWN_KEY, MAILBOX_KEY] }),
        &[(NAME_LOOKUP_TOKEN_HEADER, LOOKUP_TOKEN)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["checked_at"], NOW);
    let results = body["results"].as_array().unwrap();
    // Duplicates collapse; order follows the request.
    assert_eq!(results.len(), 4);
    assert_eq!(results[0]["pubkey"], MAILBOX_KEY);
    assert_eq!(results[0]["status"], "found");
    assert_eq!(results[0]["names"][0]["name"], "mailbox-one@finite.vip");
    assert_eq!(results[0]["names"][0]["kind"], "mailbox");
    assert_eq!(results[0]["names"][0]["source"], "finite_vip_binding");
    assert_eq!(results[0]["more_names"], false);
    assert_eq!(results[1]["pubkey"], AGENT_KEY);
    assert_eq!(results[1]["names"][0]["name"], "agent-one@finite.vip");
    assert_eq!(results[1]["names"][0]["kind"], "managed_agent");
    // Disabled bindings and unrelated keys reveal nothing.
    assert_eq!(results[2]["status"], "not_found");
    assert_eq!(results[2]["names"], serde_json::json!([]));
    assert_eq!(results[3]["status"], "not_found");
    let text = body.to_string();
    assert!(!text.contains("disabled-one"));
    assert!(!text.contains("disabled_at"));
}

#[tokio::test]
async fn name_lookup_requires_its_own_credential() {
    let disabled = seeded_router(None).await;
    let request = serde_json::json!({ "pubkeys": [MAILBOX_KEY] });
    let (status, body) = post(
        disabled,
        NAME_LOOKUP_PATH,
        request.clone(),
        &[(NAME_LOOKUP_TOKEN_HEADER, LOOKUP_TOKEN)],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "name_lookup_disabled");

    let app = seeded_router(Some(LOOKUP_TOKEN)).await;
    for headers in [
        vec![],
        vec![(NAME_LOOKUP_TOKEN_HEADER, OPERATOR_TOKEN)],
        vec![("x-finite-operator-token", OPERATOR_TOKEN)],
        vec![(NAME_LOOKUP_TOKEN_HEADER, "wrong")],
    ] {
        let (status, body) = post(app.clone(), NAME_LOOKUP_PATH, request.clone(), &headers).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{headers:?}");
        assert!(body.get("results").is_none());
    }

    // The lookup credential never unlocks operator endpoints.
    let (status, _) = post(
        app,
        "/api/v1/operator/inspect",
        serde_json::json!({ "identifier": MAILBOX_KEY }),
        &[("x-finite-operator-token", LOOKUP_TOKEN)],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn name_lookup_bounds_batches_and_rejects_malformed_keys() {
    let app = seeded_router(Some(LOOKUP_TOKEN)).await;
    let headers = [(NAME_LOOKUP_TOKEN_HEADER, LOOKUP_TOKEN)];
    let oversized = (0..=MAX_NAME_LOOKUP_KEYS)
        .map(|index| format!("{index:064x}"))
        .collect::<Vec<_>>();
    let (status, body) = post(
        app.clone(),
        NAME_LOOKUP_PATH,
        serde_json::json!({ "pubkeys": oversized }),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"], "too_many_pubkeys");

    for pubkeys in [
        serde_json::json!([]),
        serde_json::json!(["mailbox-one@finite.vip"]),
    ] {
        let (status, _) = post(
            app.clone(),
            NAME_LOOKUP_PATH,
            serde_json::json!({ "pubkeys": pubkeys }),
            &headers,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn name_lookup_authenticates_before_parsing_the_body() {
    let app = seeded_router(Some(LOOKUP_TOKEN)).await;
    let send = |token: Option<&'static str>, body: Vec<u8>| {
        let app = app.clone();
        async move {
            let mut builder = Request::builder()
                .method("POST")
                .uri(NAME_LOOKUP_PATH)
                .header("content-type", "application/json");
            if let Some(token) = token {
                builder = builder.header(NAME_LOOKUP_TOKEN_HEADER, token);
            }
            let response = app
                .oneshot(builder.body(Body::from(body)).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            let value = serde_json::from_slice::<serde_json::Value>(&bytes)
                .unwrap_or(serde_json::Value::Null);
            (status, value)
        }
    };
    // Unauthenticated garbage and oversized bodies are refused as
    // unauthenticated, never parsed.
    for body in [b"not json".to_vec(), vec![b'x'; 64 * 1024]] {
        let (status, body) = send(None, body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "missing_name_lookup_token");
    }
    let (status, body) = send(Some(LOOKUP_TOKEN), b"not json".to_vec()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_request");
    let (status, _) = send(Some(LOOKUP_TOKEN), vec![b' '; 64 * 1024]).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn name_lookup_is_never_served_publicly() {
    let store = IdentityStore::open_memory().unwrap();
    let app = public_router(state(store, Some(LOOKUP_TOKEN)));
    let (status, _) = post(
        app,
        NAME_LOOKUP_PATH,
        serde_json::json!({ "pubkeys": [MAILBOX_KEY] }),
        &[(NAME_LOOKUP_TOKEN_HEADER, LOOKUP_TOKEN)],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
