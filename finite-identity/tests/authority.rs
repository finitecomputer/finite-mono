//! Identity Directory contract tests. These exercise the public HTTP seam
//! against temporary SQLite storage so product integrations can depend on the
//! behavior rather than the implementation layout.
//!
//! The Directory is the shrunken Identity Authority: NIP-05 name serving,
//! name claiming (Email Challenge + NIP-98), managed-agent name registration,
//! and operator inspect/disable. Grant resolution, mailbox proofs, email-only
//! principals, WorkOS account bindings, and the sites-notification relay were
//! deleted; the retired-route tests pin their absence.

use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use finite_identity::authority::{
    AuthorityConfig, AuthorityState, FixedClock, IdentityStore, Mailer, public_router, router,
};
use finite_identity::client::{AuthorityErrorKind, IdentityClient, LocalIdentityKey};
use finite_identity::{FiniteIdentity, IdentityPaths, hex, nip98, npub};
use tower::ServiceExt as _;

const NOW: u64 = 1_788_000_000;
const BASE_URL: &str = "https://identity.test";
const OPERATOR_TOKEN: &str = "operator-secret";
const ALICE_EMAIL: &str = "alice@finite.vip";
const ALICE_LOCALPART: &str = "alice";
const ALICE_SECRET: [u8; 32] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3,
];
const BOB_SECRET: [u8; 32] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4,
];
const ALICE_PUBKEY: &str = "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9";

#[derive(Default)]
struct RecordingMailer {
    deliveries: Mutex<Vec<(String, String)>>,
}

impl RecordingMailer {
    fn last_token_for(&self, email: &str) -> String {
        self.deliveries
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|(delivered_email, token)| (delivered_email == email).then(|| token.clone()))
            .expect("token delivered")
    }
}

impl Mailer for RecordingMailer {
    fn send_email_challenge(&self, email: &str, token: &str) -> Result<(), String> {
        self.deliveries
            .lock()
            .unwrap()
            .push((email.to_owned(), token.to_owned()));
        Ok(())
    }
}

fn fixture() -> (
    axum::Router,
    IdentityStore,
    Arc<RecordingMailer>,
    FixedClock,
) {
    let store = IdentityStore::open_memory().expect("open memory store");
    let mailer = Arc::new(RecordingMailer::default());
    let clock = FixedClock::new(NOW);
    let state = AuthorityState::new(
        store.clone(),
        Arc::clone(&mailer) as Arc<dyn Mailer>,
        clock.clone(),
        AuthorityConfig {
            external_base_url: BASE_URL.to_owned(),
            finite_vip_domain: "finite.vip".to_owned(),
            email_challenge_ttl_seconds: 600,
            operator_token: Some(OPERATOR_TOKEN.to_owned()),
        },
    );
    (router(state), store, mailer, clock)
}

#[tokio::test]
async fn health_reports_identity_authority_ready() {
    let (app, _, _, _) = fixture();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({ "service": "finite-identity", "status": "ok" })
    );
}

async fn json_request(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: serde_json::Value,
    auth: Option<String>,
) -> (StatusCode, serde_json::Value) {
    let mut headers = Vec::new();
    if let Some(auth) = auth.as_deref() {
        headers.push(("authorization", auth));
    }
    json_request_with_headers(app, method, uri, body, &headers).await
}

async fn json_request_with_headers(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: serde_json::Value,
    headers: &[(&str, &str)],
) -> (StatusCode, serde_json::Value) {
    let bytes = serde_json::to_vec(&body).expect("json serializes");
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = app
        .oneshot(builder.body(Body::from(bytes)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&body).expect("response is json")
    };
    (status, value)
}

async fn signed_json_request(
    app: axum::Router,
    request: finite_identity::client::SignedJsonRequest,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(request.method.as_str())
                .uri(request.path.as_str())
                .header("content-type", "application/json")
                .header("authorization", request.authorization.as_str())
                .body(Body::from(request.body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&body).expect("response is json")
    };
    (status, value)
}

async fn get_json(app: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&body).expect("response is json"),
    )
}

#[tokio::test]
async fn nip05_endpoint_serves_persisted_vip_name() {
    let (app, store, _mailer, _clock) = fixture();
    store
        .bind_vip_email(ALICE_EMAIL, ALICE_PUBKEY, NOW)
        .expect("persist binding");

    let (status, body) = get_json(app.clone(), "/.well-known/nostr.json?name=alice").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        serde_json::json!({
            "names": {
                ALICE_LOCALPART: ALICE_PUBKEY,
            }
        })
    );

    let (status, body) = get_json(app, "/.well-known/nostr.json?name=unknown").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!({ "names": {} }));
}

#[tokio::test]
async fn operator_can_idempotently_bind_managed_agent_email_to_agent_principal() {
    let (app, _store, _mailer, _clock) = fixture();
    let agent_npub = npub::encode(&hex::decode32(ALICE_PUBKEY).unwrap());
    let operator_headers = [("x-finite-operator-token", OPERATOR_TOKEN)];
    let request = serde_json::json!({
        "email": "cheater@finite.vip",
        "agent_npub": agent_npub,
    });

    let (status, bound) = json_request_with_headers(
        app.clone(),
        "POST",
        "/api/v1/operator/agent-email-bindings",
        request.clone(),
        &operator_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bound["email"], "cheater@finite.vip");
    assert_eq!(bound["agent_npub"], agent_npub);
    assert_eq!(bound["nip05"], "cheater@finite.vip");

    let (status, nip05) = get_json(app.clone(), "/.well-known/nostr.json?name=cheater").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(nip05["names"]["cheater"], ALICE_PUBKEY);

    let (status, replayed) = json_request_with_headers(
        app,
        "POST",
        "/api/v1/operator/agent-email-bindings",
        request,
        &operator_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replayed, bound);
}

#[tokio::test]
async fn nip05_resolution_distinguishes_mailboxes_from_managed_agent_names() {
    let (app, store, _mailer, _clock) = fixture();
    store
        .bind_vip_email(ALICE_EMAIL, ALICE_PUBKEY, NOW)
        .expect("persist mailbox-backed name");
    let agent_npub = npub::encode(&hex::decode32(ALICE_PUBKEY).unwrap());
    let operator_headers = [("x-finite-operator-token", OPERATOR_TOKEN)];
    let (status, _) = json_request_with_headers(
        app.clone(),
        "POST",
        "/api/v1/operator/agent-email-bindings",
        serde_json::json!({
            "email": "cheater@finite.vip",
            "agent_npub": agent_npub,
        }),
        &operator_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, mailbox) = json_request(
        app.clone(),
        "POST",
        "/api/v1/nip05-resolution",
        serde_json::json!({ "name": ALICE_EMAIL }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mailbox["name"], ALICE_EMAIL);
    assert_eq!(mailbox["pubkey"], ALICE_PUBKEY);
    assert_eq!(mailbox["npub"], agent_npub);
    assert_eq!(mailbox["kind"], "mailbox");

    let (status, managed) = json_request(
        app.clone(),
        "POST",
        "/api/v1/nip05-resolution",
        serde_json::json!({ "name": "cheater@finite.vip" }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(managed["name"], "cheater@finite.vip");
    assert_eq!(managed["pubkey"], ALICE_PUBKEY);
    assert_eq!(managed["npub"], agent_npub);
    assert_eq!(managed["kind"], "managed_agent");

    let (status, missing) = json_request(
        app,
        "POST",
        "/api/v1/nip05-resolution",
        serde_json::json!({ "name": "missing@finite.vip" }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(missing["error"], "nip05_name_not_found");
}

#[tokio::test]
async fn managed_agent_email_binding_requires_operator_and_never_reassigns() {
    let (app, _store, _mailer, _clock) = fixture();
    let alice_npub = npub::encode(&hex::decode32(ALICE_PUBKEY).unwrap());
    let bob = LocalIdentityKey::from_secret(BOB_SECRET).unwrap();
    let bob_npub = npub::encode(&hex::decode32(bob.pubkey()).unwrap());
    let request = serde_json::json!({
        "email": "cheater@finite.vip",
        "agent_npub": alice_npub,
    });

    let (status, body) = json_request(
        app.clone(),
        "POST",
        "/api/v1/operator/agent-email-bindings",
        request,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "missing_operator_token");

    let operator_headers = [("x-finite-operator-token", OPERATOR_TOKEN)];
    let (status, _) = json_request_with_headers(
        app.clone(),
        "POST",
        "/api/v1/operator/agent-email-bindings",
        serde_json::json!({
            "email": "cheater@finite.vip",
            "agent_npub": alice_npub,
        }),
        &operator_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = json_request_with_headers(
        app,
        "POST",
        "/api/v1/operator/agent-email-bindings",
        serde_json::json!({
            "email": "cheater@finite.vip",
            "agent_npub": bob_npub,
        }),
        &operator_headers,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "vip_email_already_bound");
}

#[tokio::test]
async fn binding_vip_email_requires_email_challenge_and_nip98() {
    let (app, _store, mailer, _clock) = fixture();
    let (status, challenge) = json_request(
        app.clone(),
        "POST",
        "/api/v1/email-challenges",
        serde_json::json!({ "email": ALICE_EMAIL }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(challenge["email"], ALICE_EMAIL);
    let token = mailer.last_token_for(ALICE_EMAIL);

    let redeem_body = serde_json::json!({ "email": ALICE_EMAIL, "token": token });
    let redeem_bytes = serde_json::to_vec(&redeem_body).unwrap();
    let auth = nip98::build_auth_header(
        &ALICE_SECRET,
        &format!("{BASE_URL}/api/v1/vip-email-bindings/redeem"),
        "POST",
        Some(&redeem_bytes),
        NOW,
    )
    .expect("build auth");
    let (status, redeemed) = json_request(
        app.clone(),
        "POST",
        "/api/v1/vip-email-bindings/redeem",
        redeem_body.clone(),
        Some(auth),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(redeemed["email"], ALICE_EMAIL);
    assert_eq!(redeemed["pubkey"], ALICE_PUBKEY);
    assert_eq!(redeemed["nip05"], ALICE_EMAIL);

    let (status, body) = get_json(app.clone(), "/.well-known/nostr.json?name=alice").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["names"][ALICE_LOCALPART], ALICE_PUBKEY);

    let replay_auth = nip98::build_auth_header(
        &ALICE_SECRET,
        &format!("{BASE_URL}/api/v1/vip-email-bindings/redeem"),
        "POST",
        Some(&redeem_bytes),
        NOW,
    )
    .unwrap();
    let (status, _body) = json_request(
        app,
        "POST",
        "/api/v1/vip-email-bindings/redeem",
        redeem_body,
        Some(replay_auth),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn binding_vip_email_rejects_missing_auth_and_expired_challenge() {
    let (app, _store, mailer, clock) = fixture();
    let (status, _challenge) = json_request(
        app.clone(),
        "POST",
        "/api/v1/email-challenges",
        serde_json::json!({ "email": ALICE_EMAIL }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = mailer.last_token_for(ALICE_EMAIL);
    let body = serde_json::json!({ "email": ALICE_EMAIL, "token": token });

    let (status, missing_auth) = json_request(
        app.clone(),
        "POST",
        "/api/v1/vip-email-bindings/redeem",
        body.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(missing_auth["error"], "missing_authorization");

    let wrong_body = serde_json::json!({ "email": ALICE_EMAIL, "token": "wrong-token" });
    let wrong_body_bytes = serde_json::to_vec(&wrong_body).unwrap();
    let auth_for_wrong_body = nip98::build_auth_header(
        &ALICE_SECRET,
        &format!("{BASE_URL}/api/v1/vip-email-bindings/redeem"),
        "POST",
        Some(&wrong_body_bytes),
        NOW,
    )
    .unwrap();
    let (status, tampered_body) = json_request(
        app.clone(),
        "POST",
        "/api/v1/vip-email-bindings/redeem",
        body.clone(),
        Some(auth_for_wrong_body),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(tampered_body["error"], "nip98_rejected");

    clock.set(NOW + 601);
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let auth = nip98::build_auth_header(
        &ALICE_SECRET,
        &format!("{BASE_URL}/api/v1/vip-email-bindings/redeem"),
        "POST",
        Some(&body_bytes),
        NOW + 601,
    )
    .unwrap();
    let (status, expired) = json_request(
        app,
        "POST",
        "/api/v1/vip-email-bindings/redeem",
        body,
        Some(auth),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(expired["error"], "unknown_or_expired_email_challenge");
}

#[tokio::test]
async fn binding_same_vip_email_to_same_pubkey_is_idempotent() {
    let (app, _store, mailer, _clock) = fixture();
    request_and_redeem(app.clone(), &mailer, ALICE_EMAIL, &ALICE_SECRET).await;
    request_and_redeem(app.clone(), &mailer, ALICE_EMAIL, &ALICE_SECRET).await;

    let (status, body) = get_json(app, "/.well-known/nostr.json?name=alice").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["names"][ALICE_LOCALPART], ALICE_PUBKEY);
}

#[tokio::test]
async fn binding_vip_email_to_different_pubkey_is_rejected() {
    let (app, _store, mailer, _clock) = fixture();
    request_and_redeem(app.clone(), &mailer, ALICE_EMAIL, &ALICE_SECRET).await;

    let (status, _challenge) = json_request(
        app.clone(),
        "POST",
        "/api/v1/email-challenges",
        serde_json::json!({ "email": ALICE_EMAIL }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = mailer.last_token_for(ALICE_EMAIL);
    let body = serde_json::json!({ "email": ALICE_EMAIL, "token": token });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let auth = nip98::build_auth_header(
        &BOB_SECRET,
        &format!("{BASE_URL}/api/v1/vip-email-bindings/redeem"),
        "POST",
        Some(&body_bytes),
        NOW,
    )
    .unwrap();

    let (status, response) = json_request(
        app,
        "POST",
        "/api/v1/vip-email-bindings/redeem",
        body,
        Some(auth),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(response["error"], "vip_email_already_bound");
}

#[tokio::test]
async fn disabled_vip_binding_is_not_served_as_nip05() {
    let (app, store, _mailer, _clock) = fixture();
    store
        .bind_vip_email(ALICE_EMAIL, ALICE_PUBKEY, NOW)
        .expect("persist binding");
    store.disable_vip_email(ALICE_EMAIL, NOW + 1).unwrap();

    let (status, body) = get_json(app, "/.well-known/nostr.json?name=alice").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!({ "names": {} }));
}

#[tokio::test]
async fn managed_agent_directory_responses_never_expose_key_material() {
    let (app, _store, _mailer, _clock) = fixture();
    let agent_npub = npub::encode(&hex::decode32(ALICE_PUBKEY).unwrap());
    let operator_headers = [("x-finite-operator-token", OPERATOR_TOKEN)];

    let (status, binding) = json_request_with_headers(
        app.clone(),
        "POST",
        "/api/v1/operator/agent-email-bindings",
        serde_json::json!({
            "email": ALICE_EMAIL,
            "agent_npub": agent_npub,
        }),
        &operator_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(binding["email"], ALICE_EMAIL);
    assert_eq!(binding["agent_npub"], agent_npub);

    let (status, nip05) = get_json(
        app.clone(),
        &format!("/.well-known/nostr.json?name={ALICE_LOCALPART}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(nip05["names"][ALICE_LOCALPART], ALICE_PUBKEY);

    let (status, resolution) = json_request(
        app.clone(),
        "POST",
        "/api/v1/nip05-resolution",
        serde_json::json!({ "name": ALICE_EMAIL }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resolution["pubkey"], ALICE_PUBKEY);
    assert_eq!(resolution["kind"], "managed_agent");

    let (status, inspected) = json_request_with_headers(
        app,
        "POST",
        "/api/v1/operator/inspect",
        serde_json::json!({ "identifier": ALICE_EMAIL }),
        &operator_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(inspected["kind"], "vip_email");
    assert_eq!(inspected["pubkey"], ALICE_PUBKEY);

    for response in [binding, nip05, resolution, inspected] {
        let serialized = response.to_string().to_ascii_lowercase();
        for forbidden in ["operator_token", "private_key", "secret", "nsec"] {
            assert!(
                !serialized.contains(forbidden),
                "directory response exposed forbidden key material marker {forbidden}"
            );
        }
    }
}

#[tokio::test]
async fn product_cli_client_helpers_sign_the_vip_binding_flow() {
    let (app, _store, mailer, _clock) = fixture();
    let dir = tempfile::tempdir().unwrap();
    let paths = IdentityPaths::with_finite_home(dir.path());
    FiniteIdentity::import(&paths, ALICE_SECRET.into(), "seed/1.0.0").unwrap();
    let key = LocalIdentityKey::load_or_generate(&paths, "fsite/0.1.0").unwrap();
    let client = IdentityClient::new(BASE_URL);

    assert_eq!(key.pubkey(), ALICE_PUBKEY);

    let (status, challenge) = json_request(
        app.clone(),
        "POST",
        "/api/v1/email-challenges",
        client.email_challenge_body(ALICE_EMAIL).unwrap(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(challenge["email"], ALICE_EMAIL);

    let token = mailer.last_token_for(ALICE_EMAIL);
    let signed = client
        .vip_email_binding_redeem(&key, ALICE_EMAIL, &token, NOW)
        .unwrap();
    let (status, redeemed) = signed_json_request(app.clone(), signed).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(redeemed["pubkey"], ALICE_PUBKEY);

    // The shared client library still carries helpers for retired routes; the
    // Directory must not serve them. Stacked product PRs remove the callers
    // and then the helpers.
    let signed = client
        .email_only_redeem(&key, "invitee@example.com", "any-token", NOW)
        .unwrap();
    let (status, _) = signed_json_request(app, signed).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let conflict =
        IdentityClient::classify_authority_error(409, "vip_email_already_bound").unwrap();
    assert_eq!(
        conflict.kind(),
        AuthorityErrorKind::AlreadyBoundToDifferentKey
    );
    let bad_token =
        IdentityClient::classify_authority_error(400, "unknown_or_expired_email_challenge")
            .unwrap();
    assert_eq!(bad_token.kind(), AuthorityErrorKind::ExpiredOrReusedToken);
    let recovery = IdentityClient::classify_authority_error(501, "unsupported_recovery").unwrap();
    assert_eq!(recovery.kind(), AuthorityErrorKind::UnsupportedRecovery);
}

#[tokio::test]
async fn operator_can_inspect_and_disable_vip_binding_without_reassignment() {
    let (app, store, _mailer, _clock) = fixture();
    let (status, _challenge) = json_request(
        app.clone(),
        "POST",
        "/api/v1/email-challenges",
        serde_json::json!({ "email": ALICE_EMAIL }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    store
        .bind_vip_email(ALICE_EMAIL, ALICE_PUBKEY, NOW)
        .expect("persist binding");

    let operator_headers = [("x-finite-operator-token", OPERATOR_TOKEN)];
    let (status, inspected) = json_request_with_headers(
        app.clone(),
        "POST",
        "/api/v1/operator/inspect",
        serde_json::json!({ "identifier": ALICE_EMAIL }),
        &operator_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(inspected["kind"], "vip_email");
    assert_eq!(inspected["email"], ALICE_EMAIL);
    assert_eq!(inspected["pubkey"], ALICE_PUBKEY);
    assert_eq!(inspected["disabled"], false);
    assert_eq!(inspected["nip05"], ALICE_EMAIL);
    assert!(inspected.get("principal_link").is_none());
    assert_eq!(inspected["email_challenges"][0]["email"], ALICE_EMAIL);
    assert_eq!(inspected["email_challenges"][0]["created_at"], NOW);
    assert_eq!(inspected["email_challenges"][0]["expires_at"], NOW + 600);
    assert_eq!(
        inspected["email_challenges"][0]["used_at"],
        serde_json::Value::Null
    );

    let (status, disabled) = json_request_with_headers(
        app.clone(),
        "POST",
        "/api/v1/operator/disable-binding",
        serde_json::json!({ "email": ALICE_EMAIL }),
        &operator_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(disabled["email"], ALICE_EMAIL);
    assert_eq!(disabled["disabled"], true);

    let (status, nip05) = get_json(app.clone(), "/.well-known/nostr.json?name=alice").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(nip05, serde_json::json!({ "names": {} }));

    let bob_pubkey = nip98::pubkey_for_secret(&BOB_SECRET).unwrap();
    assert!(matches!(
        store.bind_vip_email(ALICE_EMAIL, &bob_pubkey, NOW + 2),
        Err(finite_identity::authority::StoreError::Conflict(
            "vip_email_already_bound"
        ))
    ));

    let (status, inspected) = json_request_with_headers(
        app,
        "POST",
        "/api/v1/operator/inspect",
        serde_json::json!({ "identifier": ALICE_PUBKEY }),
        &operator_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(inspected["kind"], "native");
    assert_eq!(inspected["pubkey"], ALICE_PUBKEY);
    assert_eq!(inspected["vip_emails"][0]["email"], ALICE_EMAIL);
    assert_eq!(inspected["vip_emails"][0]["disabled"], true);
    assert!(inspected.get("principal_links").is_none());
    assert!(inspected.get("email_only_emails").is_none());
}

#[tokio::test]
async fn operator_actions_reject_missing_token() {
    let (app, store, _mailer, _clock) = fixture();
    store
        .bind_vip_email(ALICE_EMAIL, ALICE_PUBKEY, NOW)
        .expect("persist binding");

    let (status, body) = json_request(
        app,
        "POST",
        "/api/v1/operator/inspect",
        serde_json::json!({ "identifier": ALICE_EMAIL }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "missing_operator_token");
}

#[tokio::test]
async fn retired_endpoints_are_gone_from_the_loopback_router() {
    let (app, _store, _mailer, _clock) = fixture();

    // Grant resolution, mailbox proofs, email-only principals, WorkOS account
    // bindings, brain resolution, and the sites-notification relay were
    // deleted with the directory shrink; the full router must 404 them.
    for path in [
        "/api/v1/principal-resolution/satisfies-grant",
        "/api/v1/mailbox-proofs/redeem",
        "/api/v1/mailbox-proofs/consume",
        "/api/v1/email-only-principals/redeem",
        "/internal/v1/sites-notifications",
        "/api/v1/operator/account-principal-bindings",
        "/api/v1/operator/brain/agent-resolution",
        "/api/v1/operator/brain/user-resolution",
    ] {
        let status = post_status(app.clone(), path).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{path} was deleted with the directory shrink"
        );
    }
}

async fn request_and_redeem(
    app: axum::Router,
    mailer: &RecordingMailer,
    email: &str,
    secret: &[u8; 32],
) {
    let (status, _challenge) = json_request(
        app.clone(),
        "POST",
        "/api/v1/email-challenges",
        serde_json::json!({ "email": email }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = mailer.last_token_for(email);
    let body = serde_json::json!({ "email": email, "token": token });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let auth = nip98::build_auth_header(
        secret,
        &format!("{BASE_URL}/api/v1/vip-email-bindings/redeem"),
        "POST",
        Some(&body_bytes),
        NOW,
    )
    .unwrap();
    let (status, _redeemed) = json_request(
        app,
        "POST",
        "/api/v1/vip-email-bindings/redeem",
        body,
        Some(auth),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

fn public_fixture() -> (
    axum::Router,
    IdentityStore,
    Arc<RecordingMailer>,
    FixedClock,
) {
    let store = IdentityStore::open_memory().expect("open memory store");
    let mailer = Arc::new(RecordingMailer::default());
    let clock = FixedClock::new(NOW);
    let state = AuthorityState::new(
        store.clone(),
        Arc::clone(&mailer) as Arc<dyn Mailer>,
        clock.clone(),
        AuthorityConfig {
            external_base_url: BASE_URL.to_owned(),
            finite_vip_domain: "finite.vip".to_owned(),
            email_challenge_ttl_seconds: 600,
            operator_token: Some(OPERATOR_TOKEN.to_owned()),
        },
    );
    (public_router(state), store, mailer, clock)
}

/// POST an empty JSON object and report only the status: route-mounted probes
/// must not depend on error body shapes (extractor rejections are plain text).
async fn post_status(app: axum::Router, path: &str) -> StatusCode {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await
    .unwrap()
    .status()
}

#[tokio::test]
async fn public_router_serves_the_complete_public_surface() {
    let (app, _store, _mailer, _clock) = public_fixture();

    for uri in ["/health", "/.well-known/nostr.json?name=nobody"] {
        let (status, _) = get_json(app.clone(), uri).await;
        assert_eq!(status, StatusCode::OK, "{uri} must be served publicly");
    }

    // Badly authenticated or empty requests must still reach the route
    // (4xx), proving the path is mounted on the public listener.
    for path in [
        "/api/v1/email-challenges",
        "/api/v1/vip-email-bindings/redeem",
        "/api/v1/nip05-resolution",
    ] {
        let status = post_status(app.clone(), path).await;
        assert_ne!(
            status,
            StatusCode::NOT_FOUND,
            "{path} must be served publicly"
        );
    }

    // Retired public routes must stay gone.
    for path in [
        "/api/v1/principal-resolution/satisfies-grant",
        "/api/v1/email-only-principals/redeem",
        "/api/v1/mailbox-proofs/redeem",
    ] {
        let status = post_status(app.clone(), path).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{path} was deleted with the directory shrink"
        );
    }
}

#[tokio::test]
async fn public_router_never_serves_private_routes() {
    let (app, _store, _mailer, _clock) = public_fixture();

    for path in [
        "/api/v1/operator/inspect",
        "/api/v1/operator/agent-email-bindings",
        "/api/v1/operator/disable-binding",
        // Retired loopback routes also stay off the public surface.
        "/api/v1/mailbox-proofs/consume",
        "/internal/v1/sites-notifications",
        "/api/v1/operator/account-principal-bindings",
        "/api/v1/operator/brain/agent-resolution",
        "/api/v1/operator/brain/user-resolution",
    ] {
        let status = post_status(app.clone(), path).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{path} must stay loopback-only"
        );
    }
}

#[tokio::test]
async fn nip05_allows_cross_origin_reads_served_by_the_service() {
    let (app, _store, _mailer, _clock) = fixture();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/.well-known/nostr.json?name=nobody")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|value| value.to_str().ok()),
        Some("*")
    );
}
