//! The site-serving plane: everything under `{name}.{base_domain}`.
//!
//! Visibility gate first, then path lookup in the active version, then the
//! blob. Magic-link auth lives here too, on the site's own host, so the
//! viewer cookie is naturally host-scoped.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Form, Query, State};
use axum::http::header::{
    AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, COOKIE, ETAG, HOST, IF_NONE_MATCH, LOCATION,
    SET_COOKIE,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;

use finitesites_blob::BlobStore;
use finitesites_engine::{EngineError, ViewAccess};
use finitesites_proto::dto::NativeViewerSessionRequest;
use finitesites_proto::limits::{MAX_NATIVE_VIEWER_AUTH_BODY_BYTES, VIEWER_COOKIE_TTL_SECONDS};
use finitesites_proto::nip98;
use finitesites_store::{SiteKind, SiteRecord, SiteStatus};

use crate::content_type::content_type_for_path;
use crate::pages;
use crate::proxy;
use crate::server::{AppState, now_unix, site_label};

const VIEWER_COOKIE_NAME: &str = "finite_site_auth";
const PARTITIONED_VIEWER_COOKIE_NAME: &str = "__Host-finite_site_auth_partitioned";

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/_finite/auth", get(redeem_link))
        .route("/_finite/auth/native-session", post(native_session))
        .route("/_finite/request-link", post(request_link))
        .route("/_finite/logout", get(logout))
        // Any method: app sites proxy POST/PUT/etc.; static handling
        // rejects non-GET itself.
        .fallback(serve_path)
        .with_state(state)
}

// ---- request context ---------------------------------------------------------

/// Resolve the site for this request's Host header. `Ok(None)` means the
/// label is unclaimed or invalid (render the unknown-site page).
async fn resolve_request_site(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<SiteRecord>, String> {
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let output_label = site_label(host, &state.base_domain)
        .map(|label| ("site", label))
        .or_else(|| site_label(host, &state.document_base_domain).map(|label| ("document", label)));
    let Some((output_kind, label)) = output_label else {
        // The dispatcher only routes here for site hosts; a missing label
        // means the Host header changed between routing and handling.
        return Ok(None);
    };
    state
        .serving_engines
        .run(move |engine| engine.resolve_output(output_kind, &label))
        .await?
        .map_err(|error| error.to_string())
}

fn viewer_cookie_value(headers: &HeaderMap) -> Option<String> {
    let cookie_header = headers.get(COOKIE)?.to_str().ok()?;
    cookie_value_by_name(cookie_header, VIEWER_COOKIE_NAME)
        .or_else(|| cookie_value_by_name(cookie_header, PARTITIONED_VIEWER_COOKIE_NAME))
}

fn cookie_value_by_name(cookie_header: &str, name: &str) -> Option<String> {
    // Bounded: header size is bounded by the HTTP server's limits.
    for pair in cookie_header.split(';') {
        let trimmed = pair.trim();
        if let Some(value) = trimmed.strip_prefix(name)
            && let Some(value) = value.strip_prefix('=')
        {
            return Some(value.to_string());
        }
    }
    None
}

fn html_response(status: StatusCode, body: String) -> Response {
    // Platform pages (placeholder, login, 404, unknown-site) must never be
    // edge-cached: Cloudflare default-caches by extension when no header is
    // present, which would freeze a pre-publish placeholder over real
    // content at asset URLs.
    (status, [(CACHE_CONTROL, "no-store")], Html(body)).into_response()
}

fn internal_page() -> Response {
    html_response(StatusCode::INTERNAL_SERVER_ERROR, pages::not_found())
}

fn bad_request_page() -> Response {
    html_response(StatusCode::BAD_REQUEST, pages::link_invalid())
}

fn generated_llms_response(body: String, method: &Method) -> Response {
    let response_body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(body)
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(CACHE_CONTROL, "no-store")
        .body(response_body)
        .expect("static response builds")
}

// ---- content serving ------------------------------------------------------------

async fn serve_path(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
) -> Response {
    let headers = request.headers().clone();
    let method = request.method().clone();
    let uri = request.uri().clone();
    let site = match resolve_request_site(&state, &headers).await {
        Ok(Some(site)) => site,
        Ok(None) => return html_response(StatusCode::NOT_FOUND, pages::unknown_site()),
        Err(error) => {
            eprintln!("finitesitesd serve error: {error}");
            return internal_page();
        }
    };

    if site.status == SiteStatus::Deleted {
        return html_response(StatusCode::NOT_FOUND, pages::not_found());
    }
    if site.status != SiteStatus::Published {
        return html_response(StatusCode::OK, pages::placeholder(&site.name));
    }

    let llms_request_path = if matches!(
        site.kind,
        SiteKind::Static | SiteKind::Document | SiteKind::App
    ) && (method == Method::GET || method == Method::HEAD)
    {
        decode_request_path(uri.path())
    } else {
        None
    };
    if llms_request_path.as_deref() == Some("/llms.txt") {
        let llms_site = site.clone();
        let git_base_url = state.git_base_url.clone();
        let api_url = state.api_url.clone();
        let generated =
            state
                .serving_engines
                .run(move |engine| -> Result<Option<String>, EngineError> {
                    if !engine.should_generate_llms_txt(&llms_site)? {
                        return Ok(None);
                    }
                    let (project, output) = engine.project_output_for_site(&llms_site)?.ok_or(
                        EngineError::Conflict("published project output has no output record"),
                    )?;
                    let git_remote_url = format!("{git_base_url}/{}.git", project.slug);
                    Ok(Some(crate::llms::generated_project_llms_txt(
                        &llms_site.name,
                        &engine.output_url_for_site(&llms_site),
                        &api_url,
                        &project.slug,
                        &git_remote_url,
                        &output.output_id,
                        &output.kind,
                        &output.branch,
                        &output.path,
                        output.start_command.as_deref(),
                    )))
                })
                .await;
        let generated = match generated {
            Ok(Ok(generated)) => generated,
            Ok(Err(error)) => {
                eprintln!("finitesitesd project llms.txt error: {error}");
                return internal_page();
            }
            Err(error) => {
                eprintln!("finitesitesd project llms.txt task error: {error}");
                return internal_page();
            }
        };
        if let Some(body) = generated {
            return generated_llms_response(body, &method);
        }
    }

    let access_site = site.clone();
    let viewer_cookie = viewer_cookie_value(&headers);
    let access = state
        .serving_engines
        .run(move |engine| engine.view_access(&access_site, viewer_cookie.as_deref(), now_unix()))
        .await;
    match access {
        Ok(Ok(ViewAccess::Allowed)) => {}
        Ok(Ok(ViewAccess::NeedsLogin)) => {
            return html_response(StatusCode::UNAUTHORIZED, pages::login(&site.name));
        }
        Ok(Err(error)) => {
            eprintln!("finitesitesd access error: {error}");
            return internal_page();
        }
        Err(error) => {
            eprintln!("finitesitesd access task error: {error}");
            return internal_page();
        }
    }

    // App sites: wake the app (start it if idle-reaped), then hand the
    // whole request to it — behind the same visibility gate static sites
    // get. Wake is the density mechanism: idle apps are stopped and cost
    // ~0 memory until the first request brings them back.
    if site.kind == SiteKind::App {
        let app_site_id = site.id.clone();
        let deploy = state
            .serving_engines
            .run(move |engine| engine.app_deploy_for(&app_site_id))
            .await;
        let deploy = match deploy {
            Ok(Ok(Some(deploy))) => deploy,
            Ok(Ok(None)) => {
                eprintln!("finitesitesd: app site {} is not deployable", site.id);
                return internal_page();
            }
            Ok(Err(error)) => {
                eprintln!("finitesitesd: cannot load app {}: {error}", site.id);
                return internal_page();
            }
            Err(error) => {
                eprintln!(
                    "finitesitesd: app metadata task failed for {}: {error}",
                    site.id
                );
                return internal_page();
            }
        };
        // Runner calls are blocking; keep them off the async reactor.
        let supervisor_state = state.clone();
        let woken = tokio::task::spawn_blocking(move || {
            supervisor_state
                .apps
                .note_request_and_start(&deploy, now_unix())
        })
        .await;
        let target = match woken {
            Ok(Ok(addr)) => addr,
            Ok(Err(error)) => {
                eprintln!("finitesitesd: cannot wake app {}: {error}", site.id);
                return crate::proxy::app_unavailable_response();
            }
            Err(_join) => return internal_page(),
        };
        return match proxy::forward(request, target).await {
            Ok(response) => response,
            Err(_unreachable) => {
                // Stale cache (crashed or externally stopped app): drop the
                // endpoint so the next request re-wakes it.
                state.apps.invalidate(&site.id);
                eprintln!(
                    "finitesitesd: app {} unreachable; cache invalidated",
                    site.id
                );
                crate::proxy::app_unavailable_response()
            }
        };
    }

    if site.kind == SiteKind::Document {
        let Some(request_path) = decode_request_path(uri.path()) else {
            return html_response(StatusCode::NOT_FOUND, pages::not_found());
        };
        let document_site = site.clone();
        let prepared = state
            .serving_engines
            .run(
                move |engine| -> Result<_, finitesites_engine::EngineError> {
                    let files = engine.active_version_files(&document_site)?;
                    let entry = engine
                        .project_output_for_site(&document_site)?
                        .and_then(|(_, output)| output.entry);
                    Ok((files, entry))
                },
            )
            .await;
        let prepared = match prepared {
            Ok(Ok(prepared)) => prepared,
            Ok(Err(error)) => {
                eprintln!("finitesitesd document metadata error: {error}");
                return internal_page();
            }
            Err(error) => {
                eprintln!("finitesitesd document metadata task failed: {error}");
                return internal_page();
            }
        };
        let blobs = state.blobs.clone();
        return match tokio::task::spawn_blocking(move || {
            crate::documents::serve_document(
                &blobs,
                &site,
                prepared.0,
                prepared.1,
                &request_path,
                &headers,
                &method,
            )
        })
        .await
        {
            Ok(response) => response,
            Err(error) => {
                eprintln!("finitesitesd document render task failed: {error}");
                internal_page()
            }
        };
    }

    if method != Method::GET && method != Method::HEAD {
        return html_response(StatusCode::METHOD_NOT_ALLOWED, pages::not_found());
    }

    let Some(request_path) = decode_request_path(uri.path()) else {
        return html_response(StatusCode::NOT_FOUND, pages::not_found());
    };

    let lookup_site = site.clone();
    let found = state
        .serving_engines
        .run(
            move |engine| match engine.lookup_file(&lookup_site, &request_path) {
                Ok(Some(file)) => Ok(Some((file, StatusCode::OK))),
                Ok(None) => engine
                    .lookup_not_found_page(&lookup_site)
                    .map(|file| file.map(|file| (file, StatusCode::NOT_FOUND))),
                Err(error) => Err(error),
            },
        )
        .await;
    match found {
        Ok(Ok(Some((file, status)))) => {
            blob_response(
                &state.blobs,
                &site,
                &file.sha256,
                &file.path,
                &headers,
                status,
            )
            .await
        }
        Ok(Ok(None)) => html_response(StatusCode::NOT_FOUND, pages::not_found()),
        Ok(Err(error)) => {
            eprintln!("finitesitesd lookup error: {error}");
            internal_page()
        }
        Err(error) => {
            eprintln!("finitesitesd lookup task error: {error}");
            internal_page()
        }
    }
}

async fn blob_response(
    blobs: &BlobStore,
    site: &SiteRecord,
    sha256: &str,
    served_path: &str,
    request_headers: &HeaderMap,
    status: StatusCode,
) -> Response {
    let etag = format!("\"{sha256}\"");
    // Output URLs are mutable across publishes. Cloudflare's default Browser
    // Cache TTL can replace a shorter origin max-age for cacheable assets, so
    // validators alone cannot keep an ordinary browser reload fresh.
    let cache_control = if site.visibility == finitesites_store::Visibility::Public {
        "no-store"
    } else {
        "private, no-store"
    };
    let client_etag = request_headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok());
    // Content-addressed ETags make revalidation exact: same hash, same body.
    if status == StatusCode::OK && client_etag == Some(etag.as_str()) {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(ETAG, etag)
            .header(CACHE_CONTROL, cache_control)
            .body(Body::empty())
            .expect("static response builds");
    }

    let blobs = blobs.clone();
    let sha256 = sha256.to_string();
    let bytes = match tokio::task::spawn_blocking(move || blobs.get(&sha256)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(error)) => {
            eprintln!("finitesitesd blob read error: {error}");
            return internal_page();
        }
        Err(error) => {
            eprintln!("finitesitesd blob read task failed: {error}");
            return internal_page();
        }
    };
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type_for_path(served_path))
        .header(ETAG, etag)
        .header(CACHE_CONTROL, cache_control)
        .body(Body::from(bytes))
        .expect("static response builds")
}

/// Percent-decode and sanity-check a request path. Returns `None` for
/// anything a manifest could never contain (traversal, encoded NUL, …).
fn decode_request_path(raw_path: &str) -> Option<String> {
    if raw_path.len() > 1024 {
        return None;
    }
    let mut decoded: Vec<u8> = Vec::with_capacity(raw_path.len());
    let bytes = raw_path.as_bytes();
    let mut index: usize = 0;
    // Bounded by the length check above.
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = bytes.get(index + 1)?;
            let low = bytes.get(index + 2)?;
            let value = (hex_nibble(*high)? << 4) | hex_nibble(*low)?;
            decoded.push(value);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let path = String::from_utf8(decoded).ok()?;
    if !path.starts_with('/') {
        return None;
    }
    let has_control_bytes = path.bytes().any(|b| b.is_ascii_control());
    if has_control_bytes {
        return None;
    }
    // Bounded: segment count bounded by path length.
    for segment in path[1..].split('/') {
        if segment == "." || segment == ".." {
            return None;
        }
    }
    Some(path)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

// ---- magic-link auth -------------------------------------------------------------

#[derive(Deserialize)]
struct RequestLinkForm {
    email: String,
}

/// Best-effort client identity for rate limiting. CF-Connecting-IP is
/// trustworthy when Cloudflare proxies the wildcard (the deploy doc pins
/// this); X-Forwarded-For covers a local reverse proxy. A spoofable header
/// only weakens the per-IP budget — the per-email budget binds regardless.
fn client_key(headers: &HeaderMap) -> String {
    let from_header = headers
        .get("cf-connecting-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 64);
    from_header.unwrap_or("direct").to_string()
}

async fn request_link(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<RequestLinkForm>,
) -> Response {
    let site = match resolve_request_site(&state, &headers).await {
        Ok(Some(site)) => site,
        Ok(None) => return html_response(StatusCode::NOT_FOUND, pages::unknown_site()),
        Err(error) => {
            eprintln!("finitesitesd request-link error: {error}");
            return internal_page();
        }
    };

    // Rate limits render the same generic page as success so the limiter
    // cannot be used to probe the share list.
    let now = now_unix();
    let ip_key = format!("ip:{}", client_key(&headers));
    let email_key = format!(
        "email:{}:{}",
        site.id,
        form.email.trim().to_ascii_lowercase()
    );
    let ip_allowed =
        state
            .login_limiter
            .check_and_record(&ip_key, crate::limiter::MAX_LINKS_PER_IP, now);
    let email_allowed =
        state
            .login_limiter
            .check_and_record(&email_key, crate::limiter::MAX_LINKS_PER_EMAIL, now);
    if !ip_allowed || !email_allowed {
        return html_response(StatusCode::OK, pages::link_sent());
    }

    let link = {
        let mut engine = state.engine.lock().expect("engine mutex never poisoned");
        engine.request_login_for_site(&site, &form.email, now_unix())
    };
    match link {
        Ok(Some(link)) => {
            if let Err(error) =
                state
                    .mailer
                    .send_login_link(&link.email, &link.site_name, &link.url)
            {
                eprintln!("finitesitesd mail error: {error}");
                return internal_page();
            }
        }
        Ok(None) => {
            // Same response whether or not the email has access: no
            // share-list enumeration through this endpoint.
        }
        Err(error) => {
            eprintln!("finitesitesd login error: {error}");
            return internal_page();
        }
    }
    html_response(StatusCode::OK, pages::link_sent())
}

async fn redeem_link(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let site = match resolve_request_site(&state, &headers).await {
        Ok(Some(site)) => site,
        Ok(None) => return html_response(StatusCode::NOT_FOUND, pages::unknown_site()),
        Err(error) => {
            eprintln!("finitesitesd redeem error: {error}");
            return internal_page();
        }
    };
    let email_token = params.get("token");
    let native_token = params.get("native_token");
    if email_token.is_some() == native_token.is_some() {
        return html_response(StatusCode::BAD_REQUEST, pages::link_invalid());
    }
    let return_to = match params.get("return_to") {
        Some(path) if crate::api::valid_return_to(path) => path.as_str(),
        Some(_) => return html_response(StatusCode::BAD_REQUEST, pages::link_invalid()),
        None => "/",
    };

    let redeemed = {
        let mut engine = state.engine.lock().expect("engine mutex never poisoned");
        match (email_token, native_token) {
            (Some(token), None) => engine.redeem_login(token, now_unix()),
            (None, Some(token)) => engine.redeem_native_viewer_link(token, now_unix()),
            _ => unreachable!("token shape validated above"),
        }
    };
    match redeemed {
        Ok((token_site, cookie_value)) => {
            // A link minted for one site must not set a cookie on another.
            if token_site.id != site.id {
                return html_response(StatusCode::BAD_REQUEST, pages::link_invalid());
            }
            let mut response = Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header(LOCATION, return_to)
                .body(Body::empty())
                .expect("static response builds");
            for cookie in viewer_cookie_headers(
                &cookie_value,
                VIEWER_COOKIE_TTL_SECONDS,
                &state.api_url,
                &state.base_domain,
            ) {
                response.headers_mut().append(
                    SET_COOKIE,
                    HeaderValue::from_str(&cookie).expect("generated cookie is a valid header"),
                );
            }
            response
        }
        Err(EngineError::Validation(_)) => {
            html_response(StatusCode::BAD_REQUEST, pages::link_invalid())
        }
        Err(error) => {
            eprintln!("finitesitesd redeem error: {error}");
            internal_page()
        }
    }
}

async fn native_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Response {
    if body.len() as u64 > MAX_NATIVE_VIEWER_AUTH_BODY_BYTES {
        return bad_request_page();
    }
    let site = match resolve_request_site(&state, &headers).await {
        Ok(Some(site)) => site,
        Ok(None) => return html_response(StatusCode::NOT_FOUND, pages::unknown_site()),
        Err(error) => {
            eprintln!("finitesitesd native auth site error: {error}");
            return internal_page();
        }
    };
    let Some(expected_url) = absolute_output_request_url(&state, &headers, &uri) else {
        return bad_request_page();
    };
    let Some(auth_header) = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    else {
        return html_response(StatusCode::UNAUTHORIZED, pages::link_invalid());
    };
    let now = now_unix();
    let signer_pubkey =
        match nip98::verify_auth_header(auth_header, &expected_url, "POST", Some(&body), now) {
            Ok(pubkey) => pubkey,
            Err(_) => return html_response(StatusCode::UNAUTHORIZED, pages::link_invalid()),
        };
    let request: NativeViewerSessionRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => return bad_request_page(),
    };
    if !crate::api::valid_native_viewer_session_request(&request) {
        return bad_request_page();
    }
    let limiter_key = format!("native-viewer-ip:{}", client_key(&headers));
    if !state
        .login_limiter
        .check_and_record(&limiter_key, crate::limiter::MAX_LINKS_PER_IP, now)
    {
        return html_response(StatusCode::TOO_MANY_REQUESTS, pages::link_invalid());
    }
    let result = {
        let mut engine = state.engine.lock().expect("engine mutex never poisoned");
        engine.native_viewer_session(&site, &signer_pubkey, &request.nonce, now)
    };
    match result {
        Ok(cookie_value) => {
            let mut response = Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header(LOCATION, request.return_to)
                .body(Body::empty())
                .expect("static response builds");
            for cookie in viewer_cookie_headers(
                &cookie_value,
                VIEWER_COOKIE_TTL_SECONDS,
                &state.api_url,
                &state.base_domain,
            ) {
                response.headers_mut().append(
                    SET_COOKIE,
                    HeaderValue::from_str(&cookie).expect("generated cookie is a valid header"),
                );
            }
            response
        }
        Err(EngineError::NotAuthorized) => {
            html_response(StatusCode::UNAUTHORIZED, pages::link_invalid())
        }
        Err(EngineError::Conflict("native viewer nonce replay"))
        | Err(EngineError::Validation(_)) => bad_request_page(),
        Err(error) => {
            eprintln!("finitesitesd native auth error: {error}");
            internal_page()
        }
    }
}

fn absolute_output_request_url(state: &AppState, headers: &HeaderMap, uri: &Uri) -> Option<String> {
    let host = headers.get(HOST)?.to_str().ok()?;
    let is_output_host = site_label(host, &state.base_domain).is_some()
        || site_label(host, &state.document_base_domain).is_some();
    if !is_output_host {
        return None;
    }
    let scheme = {
        let engine = state.engine.lock().expect("engine mutex never poisoned");
        engine.config().site_url_scheme.clone()
    };
    Some(format!(
        "{scheme}://{host}{}",
        uri.path_and_query()?.as_str()
    ))
}

async fn logout(State(state): State<Arc<AppState>>) -> Response {
    let mut response = Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(LOCATION, "/")
        .body(Body::empty())
        .expect("static response builds");
    for cookie in viewer_cookie_headers("", 0, &state.api_url, &state.base_domain) {
        response.headers_mut().append(
            SET_COOKIE,
            HeaderValue::from_str(&cookie).expect("generated cookie is a valid header"),
        );
    }
    response
}

fn viewer_cookie_headers(
    cookie_value: &str,
    max_age: u64,
    api_url: &str,
    base_domain: &str,
) -> Vec<String> {
    let secure_context = secure_viewer_cookie_context(api_url, base_domain);
    let ordinary_policy = if secure_context {
        "SameSite=Lax; Secure"
    } else {
        "SameSite=Lax"
    };
    let mut cookies = vec![format!(
        "{VIEWER_COOKIE_NAME}={cookie_value}; Path=/; Max-Age={max_age}; HttpOnly; {ordinary_policy}"
    )];
    if secure_context {
        cookies.push(format!(
            "{PARTITIONED_VIEWER_COOKIE_NAME}={cookie_value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=None; Secure; Partitioned"
        ));
    }
    cookies
}

fn secure_viewer_cookie_context(api_url: &str, base_domain: &str) -> bool {
    api_url.starts_with("https://")
        || base_domain == "localhost"
        || base_domain.ends_with(".localhost")
}

#[cfg(test)]
mod tests {
    use super::{
        PARTITIONED_VIEWER_COOKIE_NAME, decode_request_path, secure_viewer_cookie_context,
        viewer_cookie_headers,
    };

    #[test]
    fn serving_hot_path_never_takes_the_control_engine_mutex() {
        let source = include_str!("sites.rs");
        let start = source.find("async fn serve_path(").unwrap();
        let end = source[start..].find("async fn blob_response(").unwrap() + start;
        let serve_path = &source[start..end];
        assert!(!serve_path.contains("state.engine.lock"));
        assert!(serve_path.contains("serving_engines"));

        let documents = include_str!("documents.rs");
        assert!(!documents.contains("engine.read_blob"));
        assert!(!documents.contains("use finitesites_engine::Engine"));
    }

    #[test]
    fn decode_request_path_rules() {
        assert_eq!(decode_request_path("/"), Some("/".into()));
        assert_eq!(decode_request_path("/a%20b.html"), Some("/a b.html".into()));
        assert_eq!(
            decode_request_path("/caf%C3%A9.html"),
            Some("/café.html".into())
        );
        assert_eq!(decode_request_path("/../etc/passwd"), None);
        assert_eq!(decode_request_path("/%2e%2e/escape"), None);
        assert_eq!(decode_request_path("/bad%zz"), None);
        assert_eq!(decode_request_path("/nul%00byte"), None);
        assert_eq!(decode_request_path("no-slash"), None);
    }

    #[test]
    fn viewer_cookies_split_top_level_and_partitioned_preview_access() {
        assert!(secure_viewer_cookie_context(
            "https://api.finite.chat",
            "finite.chat"
        ));
        assert!(secure_viewer_cookie_context(
            "http://127.0.0.1:8787",
            "sites.localhost"
        ));
        assert!(!secure_viewer_cookie_context(
            "http://10.0.0.4:8787",
            "sites.internal"
        ));

        let secure =
            viewer_cookie_headers("signed-value", 60, "https://api.finite.chat", "finite.chat");
        assert_eq!(secure.len(), 2);
        assert_eq!(
            secure[0],
            "finite_site_auth=signed-value; Path=/; Max-Age=60; HttpOnly; SameSite=Lax; Secure"
        );
        assert_eq!(
            secure[1],
            format!(
                "{PARTITIONED_VIEWER_COOKIE_NAME}=signed-value; Path=/; Max-Age=60; HttpOnly; SameSite=None; Secure; Partitioned"
            )
        );

        let internal =
            viewer_cookie_headers("signed-value", 60, "http://10.0.0.4:8787", "sites.internal");
        assert_eq!(
            internal,
            vec!["finite_site_auth=signed-value; Path=/; Max-Age=60; HttpOnly; SameSite=Lax"]
        );
    }
}
