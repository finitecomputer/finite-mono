//! Only pre-join Device admission is HTTP. No transcript or metadata HTTP route.
use super::*;
use axum::{
    body::Bytes,
    http::{HeaderMap, Method, StatusCode, header},
};
use finitechat_core::browser_spike::{Command, ROOM};
use finitechat_proto::DeviceRef;
use tower_http::cors::CorsLayer;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Enrollment {
    room: String,
    device: DeviceRef,
    key_package_id: String,
    key_package_hash: String,
}

pub(super) fn router(state: HermesServiceState) -> Router {
    let Ok(origin) = std::env::var("FINITECHAT_BROWSER_SPIKE_ORIGIN") else {
        return Router::new();
    };
    Router::new()
        .route("/spike/enroll", post(enroll))
        .layer(
            CorsLayer::new()
                .allow_origin(
                    origin
                        .parse::<axum::http::HeaderValue>()
                        .expect("local spike origin"),
                )
                .allow_methods([Method::POST])
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]),
        )
        .with_state(state)
}
async fn enroll(
    State(state): State<HermesServiceState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let event =
        finite_nostr::decode_http_auth_header(auth).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let url = format!(
        "{}/spike/enroll",
        std::env::var("FINITECHAT_HERMES_SERVICE_URL").map_err(|_| StatusCode::NOT_FOUND)?
    );
    let signer = finite_nostr::validate_http_auth_event(
        &event,
        &finite_nostr::HttpAuthValidation::new("POST", url, now_ms() / 1000, 60)
            .with_body(body.to_vec()),
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let request: Enrollment = serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let allowed =
        std::env::var("FINITECHAT_BROWSER_SPIKE_USER").map_err(|_| StatusCode::NOT_FOUND)?;
    if signer.to_hex() != allowed || request.device.account_id != allowed || request.room != ROOM {
        return Err(StatusCode::FORBIDDEN);
    }
    let result = tokio::task::spawn_blocking(move || {
        state.runtime.browser_spike(Command::Enroll {
            device: request.device,
            key_package_id: request.key_package_id,
            key_package_hash: request.key_package_hash,
        })
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    result.map(Json).map_err(|e| {
        eprintln!("browser enrollment: {e}");
        StatusCode::CONFLICT
    })
}
