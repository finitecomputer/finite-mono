//! Protected-route request handling.

use std::collections::BTreeSet;

use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_MAX_AGE, AUTHORIZATION, ORIGIN, VARY,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use finite_nostr::{
    HttpAuthValidation, NostrPrimitiveError, NostrPublicKey, decode_http_auth_header,
};
use nostr::Event;

use crate::{
    ApiError, FINITEBRAIN_NOSTR_HEADER, NOSTR_AUTHORIZATION_HEADER, ServerState, lock_error,
};

pub(crate) fn cors_allowed_origins_from_public_base_url(public_base_url: &str) -> BTreeSet<String> {
    let mut origins = BTreeSet::from([public_base_url.to_owned()]);
    if let Some((scheme, rest)) = public_base_url.split_once("://") {
        let host = rest.split('/').next().unwrap_or(rest);
        if !host.is_empty() {
            origins.insert(format!("{scheme}://{host}"));
        }
    }
    origins
}

pub(crate) async fn cors_allowlist_middleware(
    State(state): State<ServerState>,
    request: Request,
    next: Next,
) -> Response {
    let origin = request
        .headers()
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let allowed_origin = origin
        .as_deref()
        .filter(|origin| state.cors_origin_allowed(origin));

    if request.method() == Method::OPTIONS && origin.is_some() {
        let mut response = if allowed_origin.is_some() {
            StatusCode::NO_CONTENT.into_response()
        } else {
            ApiError::new(StatusCode::FORBIDDEN, "CORS origin is not allowed").into_response()
        };
        if let Some(origin) = allowed_origin {
            add_cors_headers(response.headers_mut(), origin);
        }
        return response;
    }

    let mut response = next.run(request).await;
    if let Some(origin) = allowed_origin {
        add_cors_headers(response.headers_mut(), origin);
    }
    response
}

pub(crate) fn validate_request_auth(
    state: &ServerState,
    headers: &HeaderMap,
    method: &Method,
    uri: &Uri,
    body: Option<&Bytes>,
) -> Result<String, ApiError> {
    let authorization = headers
        .get(AUTHORIZATION)
        .or_else(|| headers.get(NOSTR_AUTHORIZATION_HEADER))
        .or_else(|| headers.get(FINITEBRAIN_NOSTR_HEADER))
        .ok_or_else(|| auth_error_message("valid Nostr authorization is required"))?
        .to_str()
        .map_err(|_| auth_error_message("valid Nostr authorization is required"))?;
    let event = decode_http_auth_header(authorization)
        .map_err(|_| auth_error_message("valid Nostr authorization is required"))?;

    let expected_url = absolute_url(&state.public_base_url, uri);
    let mut expected = HttpAuthValidation::new(
        method.as_str(),
        expected_url,
        state.auth_now_unix_seconds(),
        state.max_auth_skew_seconds,
    );
    if let Some(body) = body {
        expected = expected.with_body(body.to_vec());
    }

    let signer = finite_nostr::validate_http_auth_event(&event, &expected).map_err(auth_error)?;
    enforce_auth_replay_cache(state, &event)?;
    enforce_rate_limit(state, method, uri, &signer)?;
    signer.to_npub().map_err(auth_error)
}

fn add_cors_headers(headers: &mut HeaderMap, origin: &str) {
    if let Ok(origin) = HeaderValue::from_str(origin) {
        headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    }
    headers.insert(VARY, HeaderValue::from_static("Origin"));
    headers.insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET,POST,PUT,DELETE,PATCH,OPTIONS"),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static(
            "authorization,content-type,x-nostr-authorization,x-finitebrain-nostr",
        ),
    );
    headers.insert(ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
}

fn enforce_auth_replay_cache(state: &ServerState, event: &Event) -> Result<(), ApiError> {
    let now = state.auth_now_unix_seconds();
    let expires_at = now.saturating_add(state.max_auth_skew_seconds);
    let event_id = event.id.to_string();
    let mut cache = state.auth_replay_cache.lock().map_err(lock_error)?;
    cache.retain(|_, expiry| *expiry >= now);
    if cache.contains_key(&event_id) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "replayed Nostr authorization event",
        ));
    }
    cache.insert(event_id, expires_at);
    Ok(())
}

fn enforce_rate_limit(
    state: &ServerState,
    method: &Method,
    uri: &Uri,
    signer: &NostrPublicKey,
) -> Result<(), ApiError> {
    let now = state.auth_now_unix_seconds();
    let window = state.rate_limit.window_seconds.max(1);
    let floor = now.saturating_sub(window);
    let actor = signer
        .to_npub()
        .unwrap_or_else(|_| "unknown-nostr-public-key".to_owned());
    let key = format!("{actor}:{}:{}", method.as_str(), uri.path());
    let mut hits = state.rate_limit_hits.lock().map_err(lock_error)?;
    let entries = hits.entry(key).or_default();
    entries.retain(|timestamp| *timestamp > floor);
    if entries.len() as u32 >= state.rate_limit.max_requests {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "protected route rate limit exceeded",
        ));
    }
    entries.push(now);
    Ok(())
}

fn auth_error(error: NostrPrimitiveError) -> ApiError {
    ApiError::new(StatusCode::FORBIDDEN, error.to_string())
}

fn auth_error_message(message: &'static str) -> ApiError {
    ApiError::new(StatusCode::FORBIDDEN, message)
}

fn absolute_url(public_base_url: &str, uri: &Uri) -> String {
    let path_and_query = uri
        .path_and_query()
        .map_or(uri.path(), |path_and_query| path_and_query.as_str());
    format!(
        "{}{}",
        public_base_url.trim_end_matches('/'),
        path_and_query
    )
}
