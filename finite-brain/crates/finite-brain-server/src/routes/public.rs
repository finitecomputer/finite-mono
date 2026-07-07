use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};

use crate::*;

const PRODUCT_CLIENT_CACHE_CONTROL: &str = "no-store, max-age=0";

pub(crate) async fn root_handler() -> &'static str {
    "FiniteBrain Rust smoke server"
}

pub(crate) async fn health_handler() -> Json<HealthStatus> {
    Json(health_status())
}

pub(crate) async fn bootstrap_smoke_handler() -> Result<Json<BootstrapSmokeSummary>, ApiError> {
    finite_brain_core::smoke_bootstrap_summary()
        .map(Json)
        .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

pub(crate) async fn smoke_ui_handler() -> Html<&'static str> {
    Html(include_str!("../smoke-ui.html"))
}

pub(crate) async fn smoke_ui_css_handler() -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../smoke-ui.css"),
    )
}

pub(crate) async fn smoke_ui_js_handler() -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../smoke-ui.js"),
    )
}

pub(crate) async fn product_client_handler() -> impl IntoResponse {
    (
        [(CACHE_CONTROL, PRODUCT_CLIENT_CACHE_CONTROL)],
        Html(include_str!("../product-client.html")),
    )
}

pub(crate) async fn product_client_css_handler() -> impl IntoResponse {
    (
        [
            (CONTENT_TYPE, "text/css; charset=utf-8"),
            (CACHE_CONTROL, PRODUCT_CLIENT_CACHE_CONTROL),
        ],
        include_str!("../product-client.css"),
    )
}

pub(crate) async fn product_client_js_handler() -> impl IntoResponse {
    (
        [
            (CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (CACHE_CONTROL, PRODUCT_CLIENT_CACHE_CONTROL),
        ],
        include_str!("../product-client.js"),
    )
}

pub(crate) async fn product_client_config_handler(
    State(state): State<ServerState>,
) -> impl IntoResponse {
    (
        [(CACHE_CONTROL, PRODUCT_CLIENT_CACHE_CONTROL)],
        Json(ProductClientConfigResponse {
            public_base_url: state.public_base_url.to_string(),
            auth_scheme: "Nostr".to_owned(),
            http_auth_kind: 27_235,
            default_vault_id: "personal".to_owned(),
        }),
    )
}
