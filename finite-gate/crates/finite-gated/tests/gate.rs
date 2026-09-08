use axum::{
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use finite_gated::{config::GateConfig, router, state::GateState};
use std::sync::Arc;
use tower::ServiceExt;
fn config() -> GateConfig {
    GateConfig {
        listen: "127.0.0.1:8792".parse().unwrap(),
        public_url: "https://auth.finite.computer".into(),
        signing_key: [11; 32],
        account_url: "https://finite.computer/site-auth".into(),
        account_token: "12".repeat(32),
        site_base_domain: "finite.site".into(),
    }
}
fn app() -> axum::Router {
    router(Arc::new(GateState::new(config()))).layer(axum::Extension(ConnectInfo(
        "127.0.0.1:54321".parse::<std::net::SocketAddr>().unwrap(),
    )))
}
fn mint_request(token: Option<&str>, email: &str, output: &str, path: &str) -> Request<Body> {
    let mut builder = Request::post("/vouch").header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder
        .body(Body::from(
            serde_json::json!({"email":email,"output":output,"return_to":path}).to_string(),
        ))
        .unwrap()
}
#[tokio::test]
async fn authorize_forwards_only_valid_target_without_identity_or_cookie() {
    let response = app()
        .oneshot(
            Request::get(
                "/authorize?output=https%3A%2F%2Fhello.finite.site&return_to=%2Fhello%3Fx%3D1",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(response.headers().get("set-cookie").is_none());
    assert_eq!(response.headers()["cache-control"], "no-store");
    let url = url::Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
    assert_eq!(
        url.origin().ascii_serialization(),
        "https://finite.computer"
    );
    assert_eq!(url.path(), "/site-auth");
    assert!(
        url.query_pairs()
            .any(|(k, v)| k == "return_to" && v == "/hello?x=1")
    );
    let response = app()
        .oneshot(
            Request::get(
                "/authorize?output=https%3A%2F%2Fhello.finite.site&email=attacker%40example.com",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
#[tokio::test]
async fn account_attestation_mints_distinct_origin_bound_proofs() {
    let mut previous = None;
    for _ in 0..2 {
        let response = app()
            .oneshot(mint_request(
                Some(&config().account_token),
                "Viewer@Example.com",
                "https://hello.finite.site",
                "/docs?q=ok",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("set-cookie").is_none());
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
        let url = url::Url::parse(body["redeem_url"].as_str().unwrap()).unwrap();
        assert_eq!(url.path(), "/_finite/auth");
        let proof = url
            .query_pairs()
            .find(|(k, _)| k == "gate_code")
            .unwrap()
            .1
            .into_owned();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = finite_authn::verify_vouch(
            &proof,
            &config().public_key_hex().unwrap(),
            "https://hello.finite.site",
            now,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(claims.email, "viewer@example.com");
        assert!(
            finite_authn::verify_vouch(
                &proof,
                &config().public_key_hex().unwrap(),
                "https://other.finite.site",
                now,
                &Default::default()
            )
            .is_err()
        );
        assert_ne!(Some(&claims.jti), previous.as_ref());
        previous = Some(claims.jti);
    }
}
#[tokio::test]
async fn unauthenticated_caller_cannot_assert_email() {
    for token in [None, Some(""), Some("wrong"), Some(&"13".repeat(32))] {
        let response = app()
            .oneshot(mint_request(
                token,
                "viewer@example.com",
                "https://hello.finite.site",
                "/",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
#[tokio::test]
async fn rejects_bad_targets_paths_and_emails() {
    for (output, path, email) in [
        ("https://elsewhere.example", "/", "viewer@example.com"),
        ("https://finite.site", "/", "viewer@example.com"),
        ("https://api.finite.site", "/", "viewer@example.com"),
        ("https://auth.finite.site", "/", "viewer@example.com"),
        (
            "https://nested.hello.finite.site",
            "/",
            "viewer@example.com",
        ),
        ("http://hello.finite.site", "/", "viewer@example.com"),
        ("https://hello.finite.site:8443", "/", "viewer@example.com"),
        ("https://hello.finite.site/path", "/", "viewer@example.com"),
        (
            "https://hello.finite.site",
            "//attacker.example",
            "viewer@example.com",
        ),
        (
            "https://hello.finite.site",
            "/_finite/auth?gate_code=bad",
            "viewer@example.com",
        ),
        ("https://hello.finite.site", "/", "not-email"),
        ("https://hello.finite.site", "/", "a@b@c"),
        ("https://hello.finite.site", "/", "a\nb@example.com"),
    ] {
        let response = app()
            .oneshot(mint_request(
                Some(&config().account_token),
                email,
                output,
                path,
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{output} {path} {email}"
        );
    }
}
#[tokio::test]
async fn removed_auth_paths_are_not_available() {
    for path in ["/callback", "/dev/confirm"] {
        let response = app()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
#[test]
fn configuration_fails_closed() {
    assert!(config().validate().is_ok());
    for token in ["", "bad", &"GG".repeat(32)] {
        let mut c = config();
        c.account_token = token.into();
        assert!(c.validate().is_err());
    }
    for url in [
        "http://finite.computer/site-auth",
        "https://user@finite.computer/site-auth",
        "https://finite.computer/site-auth?email=x",
    ] {
        let mut c = config();
        c.account_url = url.into();
        assert!(c.validate().is_err());
    }
    let mut c = config();
    c.signing_key = [0; 32];
    assert!(c.validate().is_err());
    assert_eq!(
        finite_gated::parse_output_origin("http://hello.sites.localhost:8787", "sites.localhost")
            .as_deref(),
        Some("http://hello.sites.localhost:8787")
    );
}

#[test]
fn local_account_callback_accepts_loopback_but_remote_http_fails_closed() {
    let mut config = config();
    config.account_url = "http://127.0.0.1:13002/site-auth".into();
    assert!(config.validate().is_ok());
    config.account_url = "http://finite.computer/site-auth".into();
    assert!(config.validate().is_err());
}
