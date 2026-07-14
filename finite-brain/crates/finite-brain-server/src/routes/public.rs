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

const PRODUCT_CLIENT_APP_SCRIPT_TAG: &str =
    r#"<script src="/client/app.js?v=20260712-settings-consistency"></script>"#;

pub(crate) async fn product_client_handler(State(state): State<ServerState>) -> impl IntoResponse {
    let mut html = include_str!("../product-client.html").to_owned();
    if state.smoke_nip07_signer_secret.is_some() {
        html = html.replace(
            PRODUCT_CLIENT_APP_SCRIPT_TAG,
            r#"<script>window.__FINITE_BRAIN_DISABLE_AUTOSTART__ = true;</script>
    <script src="/client/app.js?v=20260712-settings-consistency"></script>
    <script src="/client/smoke-nip07.js?v=20260712-settings-consistency"></script>"#,
        );
    }
    ([(CACHE_CONTROL, PRODUCT_CLIENT_CACHE_CONTROL)], Html(html))
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

fn product_client_font_response(bytes: &'static [u8]) -> Response {
    (
        [
            (CONTENT_TYPE, "font/ttf"),
            (CACHE_CONTROL, PRODUCT_CLIENT_CACHE_CONTROL),
        ],
        bytes,
    )
        .into_response()
}

pub(crate) async fn product_client_funnel_display_500_font_handler() -> Response {
    product_client_font_response(include_bytes!("../fonts/funnel-display-500.ttf"))
}

pub(crate) async fn product_client_funnel_display_600_font_handler() -> Response {
    product_client_font_response(include_bytes!("../fonts/funnel-display-600.ttf"))
}

pub(crate) async fn product_client_funnel_display_700_font_handler() -> Response {
    product_client_font_response(include_bytes!("../fonts/funnel-display-700.ttf"))
}

pub(crate) async fn product_client_funnel_sans_400_font_handler() -> Response {
    product_client_font_response(include_bytes!("../fonts/funnel-sans-400.ttf"))
}

pub(crate) async fn product_client_funnel_sans_500_font_handler() -> Response {
    product_client_font_response(include_bytes!("../fonts/funnel-sans-500.ttf"))
}

pub(crate) async fn product_client_funnel_sans_600_font_handler() -> Response {
    product_client_font_response(include_bytes!("../fonts/funnel-sans-600.ttf"))
}

pub(crate) async fn product_client_funnel_sans_700_font_handler() -> Response {
    product_client_font_response(include_bytes!("../fonts/funnel-sans-700.ttf"))
}

pub(crate) async fn product_client_jetbrains_mono_400_font_handler() -> Response {
    product_client_font_response(include_bytes!("../fonts/jetbrains-mono-400.ttf"))
}

pub(crate) async fn product_client_jetbrains_mono_500_font_handler() -> Response {
    product_client_font_response(include_bytes!("../fonts/jetbrains-mono-500.ttf"))
}

pub(crate) async fn product_client_jetbrains_mono_600_font_handler() -> Response {
    product_client_font_response(include_bytes!("../fonts/jetbrains-mono-600.ttf"))
}

pub(crate) async fn product_client_smoke_nip07_js_handler(
    State(state): State<ServerState>,
) -> Response {
    let Some(secret) = state.smoke_nip07_signer_secret.as_deref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    (
        [
            (CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (CACHE_CONTROL, PRODUCT_CLIENT_CACHE_CONTROL),
        ],
        smoke_nip07_signer_script(secret),
    )
        .into_response()
}

fn smoke_nip07_signer_script(secret_hex: &str) -> String {
    let secret_json = serde_json::to_string(secret_hex).expect("secret serializes");
    format!(
        r##"(() => {{
  const client = window.FiniteBrainProductClient;
  const defaultSecretHex = {secret_json};
  if (!client) throw new Error("FiniteBrain Product Client did not load before smoke signer");
  const fragmentSecretHex = (() => {{
    try {{
      const params = new URLSearchParams(String(window.location.hash || "").replace(/^#/, ""));
      const fragmentSecret = params.get("smokeNip07Secret");
      if (!fragmentSecret) return null;
      params.delete("smokeNip07Secret");
      const remaining = params.toString();
      const cleanUrl = String(window.location.pathname || "") + String(window.location.search || "") + (remaining ? "#" + remaining : "");
      if (typeof window.history?.replaceState !== "function") return null;
      window.history.replaceState(null, "", cleanUrl || window.location.href.split("#")[0]);
      return fragmentSecret;
    }} catch (_) {{
      return null;
    }}
  }})();
  const configuredSecretHex = fragmentSecretHex || defaultSecretHex;
  const installSmokeSigner = (secretHex) => {{
    const nip07Provider = client.createLocalNip07ProviderFromSecret(secretHex);
    client.configureBrainIdentityProvider(client.createNip07BrainIdentityProvider(nip07Provider));
    const keypair = client.inviteUnwrapKeypairFromSecret(secretHex);
    window.__FINITE_BRAIN_SMOKE_NIP07__ = {{
      publicKeyHex: keypair.publicKeyHex,
      npub: keypair.npub,
      source: secretHex === defaultSecretHex ? "server" : "override"
    }};
    return window.__FINITE_BRAIN_SMOKE_NIP07__;
  }};
  window.__FINITE_BRAIN_SET_SMOKE_NIP07_SECRET__ = (secretHex) => {{
    return installSmokeSigner(secretHex);
  }};
  installSmokeSigner(configuredSecretHex);
  client.start();
}})();
"##
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
