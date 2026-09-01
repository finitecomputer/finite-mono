//! Router-level tests for the gate's dev-mode flow: /authorize renders the
//! loudly-labeled confirmation, /dev/confirm mints an origin-bound vouch
//! and redirects to the output's `/_finite/auth` consumer, and a gate
//! session makes later authorizes silent.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use finite_authn::{AuthPolicy, verify_vouch};
use tower::util::ServiceExt;

use finite_gated::config::{DEFAULT_DEV_EMAIL, GateConfig};
use finite_gated::state::GateState;

fn dev_config() -> GateConfig {
    GateConfig {
        listen: "127.0.0.1:8792".parse().unwrap(),
        public_url: "http://auth.sites.localhost:8792".to_string(),
        signing_key: [0x11; 32],
        workos_client_id: None,
        workos_api_key: None,
        dev_email: DEFAULT_DEV_EMAIL.to_string(),
    }
}

fn app() -> (axum::Router, [u8; 32]) {
    let config = dev_config();
    let key = config.signing_key;
    let router = finite_gated::router(Arc::new(GateState::new(config))).layer(axum::Extension(
        axum::extract::connect_info::ConnectInfo::<std::net::SocketAddr>(
            "127.0.0.1:44000".parse().unwrap(),
        ),
    ));
    (router, key)
}

async fn get(app: &axum::Router, target: &str) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(target)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn post_form(
    app: &axum::Router,
    target: &str,
    body: &str,
    cookie: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method("POST")
        .uri(target)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    app.clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn query_param(location: &str, key: &str) -> String {
    let query = location.split_once('?').unwrap().1;
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap();
        if name == key {
            return percent_decode(value);
        }
    }
    panic!("query parameter {key} missing from {location}");
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = (bytes[index + 1] as char).to_digit(16).unwrap() as u8;
            let low = (bytes[index + 2] as char).to_digit(16).unwrap() as u8;
            out.push((high << 4) | low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

#[tokio::test]
async fn authorize_in_dev_mode_renders_a_loud_confirmation_page() {
    let (app, _) = app();
    let response = get(
        &app,
        "/authorize?output=http%3A%2F%2Fhello.sites.localhost%3A18789%2F&return_to=%2Fdocs%2F",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("DEV MODE"), "{body}");
    assert!(body.contains(DEFAULT_DEV_EMAIL));
    assert!(body.contains("hello.sites.localhost:18789"));
}

#[tokio::test]
async fn authorize_rejects_non_origin_and_offsite_outputs() {
    let (app, _) = app();
    for bad in [
        "output=not-a-url",
        "output=ftp%3A%2F%2Fhello.sites.localhost%2F",
        "output=http%3A%2F%2Fhello.sites.localhost%2Fpage.html",
        "output=http%3A%2F%2Fuser%3Apass%40hello.sites.localhost%2F",
        "output=",
    ] {
        let response = get(&app, &format!("/authorize?{bad}")).await;
        assert_eq!(response.status(), StatusCode::OK, "{bad}");
        let body = body_text(response).await;
        assert!(body.contains("not valid"), "{bad}: {body}");
    }
}

#[tokio::test]
async fn dev_confirm_mints_an_origin_bound_vouch_and_redirects() {
    let (app, signing_key) = app();
    let response = post_form(
        &app,
        "/dev/confirm",
        "output=http%3A%2F%2Fhello.sites.localhost%3A18789&return_to=%2Fgallery%3Fx%3D1",
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .unwrap()
        .to_string();
    assert!(
        location.starts_with("http://hello.sites.localhost:18789/_finite/auth?gate_code="),
        "{location}"
    );
    assert_eq!(query_param(&location, "return_to"), "/gallery?x=1");

    // The minted vouch verifies offline against the gate's public key and
    // names the dev identity for exactly this origin.
    let vouch = query_param(&location, "gate_code");
    let pubkey = finite_authn::gate_pubkey_for_secret(&signing_key).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = verify_vouch(
        &vouch,
        &pubkey,
        "http://hello.sites.localhost:18789",
        now,
        &AuthPolicy::default(),
    )
    .unwrap_or_else(|error| panic!("minted vouch must verify: {error}"));
    assert_eq!(claims.email, DEFAULT_DEV_EMAIL);
    assert_eq!(
        verify_vouch(
            &vouch,
            &pubkey,
            "http://other.sites.localhost:18789",
            now,
            &AuthPolicy::default()
        )
        .unwrap_err()
        .to_string(),
        "invalid vouch: audience mismatch"
    );

    // The gate sets its own session cookie; it never touches site cookies.
    let set_cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .unwrap();
    assert!(
        set_cookie.starts_with("finite_gate_session="),
        "{set_cookie}"
    );
}

#[tokio::test]
async fn a_gate_session_makes_later_authorizes_silent_and_per_origin() {
    let (app, signing_key) = app();
    let confirmed = post_form(
        &app,
        "/dev/confirm",
        "output=http%3A%2F%2Fa.sites.localhost%3A18789&return_to=%2F",
        None,
    )
    .await;
    let cookie = confirmed
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    // A different origin, with the gate session: straight to a vouch.
    let response = get(
        &app,
        "/authorize?output=http%3A%2F%2Fb.sites.localhost%3A18789%2F&return_to=%2F",
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "no session cookie sent: expected the dev confirmation page"
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/authorize?output=http%3A%2F%2Fb.sites.localhost%3A18789%2F&return_to=%2F")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .unwrap()
        .to_string();
    let vouch = query_param(&location, "gate_code");
    let pubkey = finite_authn::gate_pubkey_for_secret(&signing_key).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = verify_vouch(
        &vouch,
        &pubkey,
        "http://b.sites.localhost:18789",
        now,
        &AuthPolicy::default(),
    )
    .expect("silent vouch verifies for the requested origin");
    assert_eq!(claims.email, DEFAULT_DEV_EMAIL);
}

#[tokio::test]
async fn dev_confirm_rejects_bad_return_paths_and_forms() {
    let (app, _) = app();
    for bad_return in [
        "%2F%2Fevil.com",
        "%5C%2Fevil.com",
        "javascript%3Aalert(1)",
        "%2Fwith%20space",
    ] {
        let response = post_form(
            &app,
            "/dev/confirm",
            &format!("output=http%3A%2F%2Fhello.sites.localhost&return_to={bad_return}"),
            None,
        )
        .await;
        let body = body_text(response).await;
        assert!(
            body.contains("not valid"),
            "return_to {bad_return} must be rejected: {body}"
        );
    }
}
