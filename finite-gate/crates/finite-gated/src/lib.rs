//! `finite-gated` — the Finite Auth Gate.
//!
//! Viewers gate here. A browser that hits a non-public Finite Site output
//! without a session is redirected (top-level) to this daemon; the human
//! authenticates (WorkOS AuthKit in production, a loudly-labeled fixed-email
//! confirmation in dev mode); the gate redirects back to
//! `{output}/_finite/auth?gate_code=<vouch>` with a short-lived,
//! origin-bound vouch. The gate never sets the site's viewer cookie — it
//! vouches, and finitesitesd mints its own session after verifying the vouch
//! offline against the pinned gate public key.
//!
//! Configuration is environment-only (secret NAMES, never values, in git):
//!
//! - `FINITE_GATE_LISTEN` — bind address (default `127.0.0.1:8792`).
//! - `FINITE_GATE_PUBLIC_URL` — the gate's own canonical origin
//!   (e.g. `https://auth.finite.computer`); required in production.
//! - `FINITE_GATE_SIGNING_KEY` — 64 lowercase hex chars; the vouch signing
//!   secret. Its public counterpart is pinned by finitesitesd.
//! - `FINITE_GATE_WORKOS_CLIENT_ID` / `FINITE_GATE_WORKOS_API_KEY` — when
//!   the client id is absent the gate runs in DEV MODE and never calls
//!   WorkOS.
//! - `FINITE_GATE_DEV_EMAIL` — the fixed dev-mode identity (default
//!   `dev@finite.computer`).

pub mod config;
pub mod limiter;
pub mod pages;
pub mod session;
pub mod state;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Form, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use finite_authn::{AuthPolicy, mint_vouch};
use serde::Deserialize;

use crate::state::GateState;

/// How long a pending WorkOS round trip may take before its state entry is
/// dropped (the user simply restarts the flow).
const PENDING_STATE_TTL_SECONDS: u64 = 10 * 60;

pub fn router(state: Arc<GateState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/authorize", get(authorize))
        .route("/callback", get(callback))
        .route("/dev/confirm", post(dev_confirm))
        .fallback(gate_not_found)
        .with_state(state)
}

fn now_unix() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the unix epoch");
    now.as_secs()
}

async fn healthz() -> Response {
    (StatusCode::OK, "ok").into_response()
}

async fn gate_not_found() -> Response {
    Html(pages::error("Unknown gate route.")).into_response()
}

// ---- request validation ------------------------------------------------------

/// A strictly canonical output origin: `scheme://host[:port]`, nothing else.
/// This is the vouch audience — a vouch for one output is not a passport to
/// others — so parsing is exact: a path (even a non-root one), query,
/// fragment, or userinfo disqualifies the value.
pub fn parse_output_origin(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.len() > 2048 {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return None,
    }
    if url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && (url.path() == "/" || url.path().is_empty())
    {
        // Origin serialization keeps non-default ports and strips default
        // ones, matching how browsers and finitesitesd spell output origins.
        let origin = url.origin().ascii_serialization();
        if origin.len() <= 2048 {
            return Some(origin);
        }
    }
    None
}

/// Same-origin return path, exactly as strict as finitesitesd's
/// `valid_return_to`: bounded, ASCII, starts with `/`, never `//`, no
/// backslash, no control or space bytes.
pub fn parse_return_to(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.len() > 1024 {
        return None;
    }
    let bytes = raw.as_bytes();
    if !raw.starts_with('/') || raw.starts_with("//") || raw.contains('\\') {
        return None;
    }
    if !bytes.iter().all(|byte| (0x21..=0x7e).contains(byte)) {
        return None;
    }
    Some(raw.to_string())
}

#[derive(Deserialize)]
struct AuthorizeParams {
    output: String,
    return_to: Option<String>,
}

/// Validate an /authorize request into (audience, return path).
fn authorize_target(params: &AuthorizeParams) -> Option<(String, String)> {
    let audience = parse_output_origin(&params.output)?;
    let return_to = match &params.return_to {
        Some(raw) => parse_return_to(raw)?,
        None => "/".to_string(),
    };
    Some((audience, return_to))
}

// ---- authorize ----------------------------------------------------------------

async fn authorize(
    State(state): State<Arc<GateState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let params: AuthorizeParams = match Query::try_from_uri(&uri) {
        Ok(Query(params)) => params,
        Err(_) => return Html(pages::error("Choose a Finite site to view.")).into_response(),
    };
    let Some((audience, return_to)) = authorize_target(&params) else {
        return Html(pages::error("That output URL is not valid.")).into_response();
    };
    if !state.limiter.check(client_ip(&headers, peer)) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Html(pages::error("Too many attempts; wait a minute and retry.")),
        )
            .into_response();
    }

    // An existing gate session mints the vouch silently: one WorkOS login
    // per browser, not per site view.
    if let Some(email) = session::read_email(&state, &headers, now_unix()) {
        return mint_and_redirect(&state, &audience, &email, &return_to);
    }

    match state.workos.as_ref() {
        None => Html(pages::dev_confirm(
            &state.config.dev_email,
            &audience,
            &return_to,
        ))
        .into_response(),
        Some(client) => {
            let state_nonce = state.remember_pending(&audience, &return_to, now_unix());
            let redirect_uri = format!("{}/callback", state.config.public_url);
            let authorization =
                client
                    .authkit()
                    .authorization_url(workos::AuthKitAuthorizationUrlParams {
                        redirect_uri,
                        state: Some(state_nonce),
                        ..Default::default()
                    });
            match authorization {
                Ok(url) => see_other(&url),
                Err(error) => {
                    eprintln!("finite-gated: cannot build AuthKit URL: {error}");
                    Html(pages::error("Sign-in is temporarily unavailable.")).into_response()
                }
            }
        }
    }
}

// ---- WorkOS callback ----------------------------------------------------------

#[derive(Deserialize)]
struct CallbackParams {
    code: Option<String>,
    state: String,
    error: Option<String>,
}

async fn callback(
    State(state): State<Arc<GateState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let Some(client) = state.workos.clone() else {
        return Html(pages::error("Sign-in is not configured.")).into_response();
    };
    if !state.limiter.check(client_ip(&headers, peer)) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Html(pages::error("Too many attempts; wait a minute and retry.")),
        )
            .into_response();
    };
    let params: CallbackParams = match Query::try_from_uri(&uri) {
        Ok(Query(params)) => params,
        Err(_) => return Html(pages::error("That sign-in link is not valid.")).into_response(),
    };
    if let Some(error) = &params.error {
        eprintln!("finite-gated: AuthKit returned an error: {error}");
        return Html(pages::error("Sign-in was not completed.")).into_response();
    }
    let Some(code) = params
        .code
        .filter(|code| !code.is_empty() && code.len() <= 16 * 1024)
    else {
        return Html(pages::error("That sign-in link is not valid.")).into_response();
    };
    let Some((audience, return_to)) = state.take_pending(&params.state, now_unix()) else {
        return Html(pages::error(
            "That sign-in link expired. Start again from the site.",
        ))
        .into_response();
    };

    let mut request = workos::user_management::AuthenticateWithCodeParams::new(code);
    request.ip_address = None;
    let response = match client
        .user_management()
        .authenticate_with_code(request)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            eprintln!("finite-gated: AuthKit code exchange failed: {error}");
            return Html(pages::error("Sign-in was not completed.")).into_response();
        }
    };
    // The vouch names a VERIFIED email attribute; an unverified mailbox is
    // not a statement the gate is allowed to make.
    if !response.user.email_verified {
        return Html(pages::error("Verify your email address, then try again.")).into_response();
    }
    let email = response.user.email.trim().to_ascii_lowercase();
    if email.is_empty() || email.len() > 254 {
        return Html(pages::error("That account email is not usable.")).into_response();
    }
    let mut response = mint_and_redirect(&state, &audience, &email, &return_to);
    session::set_cookie(&mut response, &state, &email, now_unix());
    response
}

// ---- dev mode -----------------------------------------------------------------

#[derive(Deserialize)]
struct DevConfirmForm {
    output: String,
    return_to: Option<String>,
}

async fn dev_confirm(
    State(state): State<Arc<GateState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<DevConfirmForm>,
) -> Response {
    if state.workos.is_some() {
        // Dev confirm exists only when WorkOS is absent; never reachable in
        // production configuration.
        return (
            StatusCode::NOT_FOUND,
            Html(pages::error("Unknown gate route.")),
        )
            .into_response();
    }
    if !state.limiter.check(client_ip(&headers, peer)) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Html(pages::error("Too many attempts; wait a minute and retry.")),
        )
            .into_response();
    }
    let params = AuthorizeParams {
        output: form.output,
        return_to: form.return_to,
    };
    let Some((audience, return_to)) = authorize_target(&params) else {
        return Html(pages::error("That output URL is not valid.")).into_response();
    };
    let mut response = mint_and_redirect(&state, &audience, &state.config.dev_email, &return_to);
    session::set_cookie(&mut response, &state, &state.config.dev_email, now_unix());
    response
}

// ---- vouch minting --------------------------------------------------------------

fn mint_and_redirect(state: &GateState, audience: &str, email: &str, return_to: &str) -> Response {
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).expect("operating system randomness must be available");
    let vouch = match mint_vouch(
        &state.config.signing_key,
        audience,
        email,
        now_unix(),
        &AuthPolicy::default(),
        nonce,
    ) {
        Ok(vouch) => vouch,
        Err(error) => {
            eprintln!("finite-gated: cannot mint vouch: {error}");
            return Html(pages::error("Sign-in is temporarily unavailable.")).into_response();
        }
    };
    let location = format!(
        "{}/_finite/auth?gate_code={}&return_to={}",
        audience.trim_end_matches('/'),
        encode_query_component(&vouch),
        encode_query_component(return_to)
    );
    see_other(&location)
}

fn see_other(location: &str) -> Response {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::empty())
        .expect("static response builds")
}

/// Percent-encode one query component (RFC 3986 unreserved kept verbatim).
pub fn encode_query_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            write!(&mut encoded, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    encoded
}

/// Best-effort client identity for the limiter. Behind Caddy the trusted
/// header wins; direct connections fall back to the socket address.
fn client_ip(headers: &HeaderMap, peer: SocketAddr) -> std::net::IpAddr {
    if let Some(value) = headers
        .get("cf-connecting-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 64)
        .and_then(|value| value.parse().ok())
    {
        return value;
    }
    peer.ip()
}
