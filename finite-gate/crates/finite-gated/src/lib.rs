//! A single origin-bound vouch signer. The trusted account application owns
//! all login/session behavior; Sites retains its own access grants and cookies.
pub mod config;
pub mod limiter;
pub mod state;

use axum::{
    Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use finite_authn::{AuthPolicy, mint_vouch};
use serde::{Deserialize, Serialize};
use state::GateState;
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use subtle::ConstantTimeEq;

pub fn router(state: Arc<GateState>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/authorize", get(authorize))
        .route("/vouch", post(vouch))
        .layer(DefaultBodyLimit::max(4096))
        .with_state(state)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthorizeParams {
    output: String,
    return_to: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VouchRequest {
    output: String,
    return_to: String,
    email: String,
}
#[derive(Serialize)]
struct VouchResponse {
    redeem_url: String,
}

pub fn parse_output_origin(raw: &str, domain: &str) -> Option<String> {
    if raw.len() > 2048 {
        return None;
    }
    let url = config::secure_url(raw).ok()?;
    if url.path() != "/" {
        return None;
    }
    let label = url.host_str()?.strip_suffix(&format!(".{domain}"))?;
    if !config::valid_label(label) || matches!(label, "api" | "git" | "auth" | "www") {
        return None;
    }
    // Production origins have one canonical HTTPS port.
    if url.port().is_some() && !(domain == "localhost" || domain.ends_with(".localhost")) {
        return None;
    }
    Some(url.origin().ascii_serialization())
}
pub fn parse_return_to(raw: &str) -> Option<String> {
    if raw.is_empty()
        || raw.len() > 1024
        || !raw.starts_with('/')
        || raw.starts_with("//")
        || raw.contains('\\')
        || !raw.bytes().all(|b| (0x21..=0x7e).contains(&b))
    {
        return None;
    }
    let parsed = url::Url::parse(&format!("https://return.invalid{raw}")).ok()?;
    if parsed.path().starts_with("/_finite/") {
        return None;
    }
    Some(raw.to_string())
}
fn target(state: &GateState, output: &str, path: &str) -> Option<(String, String)> {
    Some((
        parse_output_origin(output, &state.config.site_base_domain)?,
        parse_return_to(path)?,
    ))
}
fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    response
}
async fn authorize(
    State(state): State<Arc<GateState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(params): Query<AuthorizeParams>,
) -> Response {
    if !state.limiter.check(client_ip(&headers, peer)) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let Some((output, path)) = target(
        &state,
        &params.output,
        params.return_to.as_deref().unwrap_or("/"),
    ) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let mut url = url::Url::parse(&state.config.account_url).expect("validated account URL");
    url.query_pairs_mut()
        .append_pair("output", &output)
        .append_pair("return_to", &path);
    no_store(Redirect::to(url.as_str()).into_response())
}
async fn vouch(
    State(state): State<Arc<GateState>>,
    headers: HeaderMap,
    Json(request): Json<VouchRequest>,
) -> Response {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or("");
    if !bool::from(
        token
            .as_bytes()
            .ct_eq(state.config.account_token.as_bytes()),
    ) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some((output, path)) = target(&state, &request.output, &request.return_to) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let email = request.email.trim().to_ascii_lowercase();
    let Some((local, domain)) = email.split_once('@') else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if email.len() > 254
        || local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || email.bytes().any(|b| b <= 32 || b >= 127)
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let mut nonce = [0; 32];
    if getrandom::fill(&mut nonce).is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs();
    let Ok(proof) = mint_vouch(
        &state.config.signing_key,
        &output,
        &email,
        now,
        &AuthPolicy::default(),
        nonce,
    ) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut redeem = url::Url::parse(&format!("{output}/_finite/auth")).expect("validated output");
    redeem
        .query_pairs_mut()
        .append_pair("gate_code", &proof)
        .append_pair("return_to", &path);
    no_store(
        Json(VouchResponse {
            redeem_url: redeem.into(),
        })
        .into_response(),
    )
}
fn client_ip(headers: &HeaderMap, peer: SocketAddr) -> IpAddr {
    if peer.ip().is_loopback()
        && let Some(ip) = headers
            .get(header::HeaderName::from_static("x-forwarded-for"))
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.rsplit(',').next())
            .and_then(|h| h.trim().parse().ok())
    {
        return ip;
    }
    peer.ip()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forwarding_only_trusted_from_loopback_and_last_hop() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "192.0.2.9, 192.0.2.10".parse().unwrap());
        headers.insert("cf-connecting-ip", "192.0.2.11".parse().unwrap());
        assert_eq!(
            client_ip(&headers, "192.0.2.12:80".parse().unwrap()).to_string(),
            "192.0.2.12"
        );
        assert_eq!(
            client_ip(&headers, "127.0.0.1:80".parse().unwrap()).to_string(),
            "192.0.2.10"
        );
        headers.remove("x-forwarded-for");
        assert_eq!(
            client_ip(&headers, "127.0.0.1:80".parse().unwrap()).to_string(),
            "127.0.0.1"
        );
    }
}
