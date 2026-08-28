//! NIP-98-style request authentication for account-scoped routes.
//!
//! `SignedJson<T>` buffers the request body, verifies the
//! `Authorization: Nostr <base64>` event against the method, the absolute
//! request URL, and the payload hash, then binds the verified signer to the
//! account id named by the body (`AccountScopedRequest`). The MemberId-keyed
//! routes (/sync/*, /welcomes/*, KeyPackage publish/inventory) stay unsigned
//! for now — binding them to account keys is phase 2.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use finitechat_http::{AccountScopedRequest, ErrorResponse};
use serde::de::DeserializeOwned;

use crate::ServerHttpError;
use crate::state::HttpServerState;

/// Maximum accepted clock skew for HTTP auth events, in seconds. Generous on
/// purpose: first-party clients run on consumer devices and hosted boxes
/// whose clocks drift.
pub(crate) const HTTP_AUTH_MAX_SKEW_SECONDS: u64 = 300;

/// JSON body extractor for account-scoped routes that authenticates the
/// request with a NIP-98-style signed event.
///
/// Mixed-version behavior follows `HttpServerState::require_signed_requests`:
/// when false, a missing header is accepted (old deployed clients keep
/// working) and a present-but-invalid header is logged and treated as
/// unsigned — rejecting it would only break upgraded clients whose dial URL
/// differs from this server's public URL, while buying nothing (an attacker
/// just omits the header). When true, a missing or invalid header is
/// rejected. In both modes, a signature that does validate is binding: the
/// signer must match the account id named by the body.
pub(crate) struct SignedJson<T>(pub T);

impl<T> FromRequest<HttpServerState> for SignedJson<T>
where
    T: AccountScopedRequest + DeserializeOwned,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &HttpServerState) -> Result<Self, Self::Rejection> {
        let method = req.method().as_str().to_owned();
        let headers = req.headers().clone();
        let uri = req.uri().clone();
        let bytes = Bytes::from_request(req, state)
            .await
            .map_err(IntoResponse::into_response)?;

        let signer = match authorization_header(&headers) {
            None => {
                if state.require_signed_requests() {
                    return Err(unauthorized("missing Nostr authorization header"));
                }
                None
            }
            Some(value) => match finite_nostr::decode_http_auth_header(value).and_then(|event| {
                let url = absolute_request_url(state.public_url(), &headers, &uri);
                let validation = finite_nostr::HttpAuthValidation::new(
                    method,
                    url,
                    current_unix_seconds(),
                    HTTP_AUTH_MAX_SKEW_SECONDS,
                )
                .with_body(bytes.to_vec());
                finite_nostr::validate_http_auth_event(&event, &validation)
            }) {
                Ok(signer) => Some(signer.to_hex()),
                // Flag-off mode treats a bad header as advisory: upgraded
                // clients sign their configured dial URL, which can differ
                // from this server's public URL (loopback/alias
                // deployments), so rejecting here would 401 exactly the
                // clients this rollout is meant to carry. With the flag off
                // an attacker simply omits the header anyway — rejecting
                // buys nothing and breaks real deployments.
                Err(error) => {
                    if state.require_signed_requests() {
                        return Err(unauthorized(format!(
                            "invalid Nostr authorization: {error}"
                        )));
                    }
                    eprintln!(
                        "finitechat-server: ignoring invalid Nostr authorization \
                         (signed-requests flag is off): {error}"
                    );
                    None
                }
            },
        };

        let body = serde_json::from_slice::<T>(&bytes).map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    kind: "invalid_json".to_owned(),
                    error: format!("failed to deserialize request body: {error}"),
                }),
            )
                .into_response()
        })?;

        // A signature that validates is binding in every mode: a signer that
        // does not match the named account is never tolerated, flag or no
        // flag. First-party clients always sign as the account they name, so
        // this cannot be a compat false positive.
        if let Some(signer) = signer
            && body.signer_account_id() != signer
        {
            return Err(unauthorized(
                "authorization signer does not match the request account",
            ));
        }

        Ok(Self(body))
    }
}

fn unauthorized(reason: impl Into<String>) -> Response {
    ServerHttpError::Unauthorized {
        reason: reason.into(),
    }
    .into_response()
}

fn authorization_header(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::AUTHORIZATION)?.to_str().ok()
}

/// Reconstruct the absolute URL the client signed: the configured public
/// origin when set, else the forwarded scheme and Host header (the same
/// fallback blob URLs use behind the edge proxy).
fn absolute_request_url(public_url: Option<&str>, headers: &HeaderMap, uri: &Uri) -> String {
    let path = uri
        .path_and_query()
        .map(|path_and_query| path_and_query.as_str())
        .unwrap_or("/");
    if let Some(public_url) = public_url {
        return format!("{public_url}{path}");
    }
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .filter(|value| *value == "http" || *value == "https")
        .unwrap_or("http");
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("localhost");
    format!("{scheme}://{host}{path}")
}

fn current_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_url_prefers_public_origin() {
        let headers = HeaderMap::new();
        let uri: Uri = "http://ignored.local/activities/get".parse().expect("uri");

        assert_eq!(
            absolute_request_url(Some("https://chat.finite.computer"), &headers, &uri),
            "https://chat.finite.computer/activities/get"
        );
    }

    #[test]
    fn absolute_url_falls_back_to_forwarded_scheme_and_host() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", "https".parse().expect("header"));
        headers.insert(
            header::HOST,
            "chat.finite.computer".parse().expect("header"),
        );
        let uri: Uri = "http://127.0.0.1:8787/activities/get".parse().expect("uri");

        assert_eq!(
            absolute_request_url(None, &headers, &uri),
            "https://chat.finite.computer/activities/get"
        );
    }

    #[test]
    fn absolute_url_defaults_to_http_host_without_forwarded_proto() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:8787".parse().expect("header"));
        let uri: Uri = "http://127.0.0.1:8787/activities/get".parse().expect("uri");

        assert_eq!(
            absolute_request_url(None, &headers, &uri),
            "http://127.0.0.1:8787/activities/get"
        );
    }
}
