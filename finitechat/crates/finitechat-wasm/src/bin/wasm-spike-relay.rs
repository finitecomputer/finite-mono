#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use axum::{Json, http::StatusCode, routing::get};
    use finitechat_client::generate_account_secret;
    use finitechat_server::{HttpServerState, http_router};
    use nostr::ToBech32;
    use std::{net::SocketAddr, path::PathBuf};
    use tower_http::cors::CorsLayer;

    let state_dir = PathBuf::from(std::env::args().nth(1).ok_or_else(|| {
        anyhow::anyhow!(
            "usage: wasm-spike-relay EMPTY_STATE_DIR PORT DASHBOARD_ORIGIN AGENT_INFO_JSON"
        )
    })?);
    let port: u16 = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "28789".into())
        .parse()?;
    let origin = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "http://127.0.0.1:13010".into());
    let agent_info = PathBuf::from(
        std::env::args()
            .nth(4)
            .ok_or_else(|| anyhow::anyhow!("missing agent info path"))?,
    );
    let agent_url = std::env::args()
        .nth(5)
        .ok_or_else(|| anyhow::anyhow!("missing agent service URL"))?;
    std::fs::create_dir(&state_dir)?;
    let secret = generate_account_secret()?;
    let user_id = hex::encode(secret.public_key().as_bytes());
    let nsec = nostr::SecretKey::from_slice(secret.as_bytes())?.to_bech32()?;
    let server_url = format!("http://127.0.0.1:{port}");
    let state = HttpServerState::from_sqlite_path(state_dir.join("relay.sqlite3"))?
        .with_public_url(&server_url)?
        .with_require_signed_requests(true);
    let app = http_router(state)
        // This is the disposable Core key handoff, not a chat endpoint.
        .route(
            "/spike/bootstrap",
            get(move || {
                let (nsec, server_url, agent_info) =
                    (nsec.clone(), server_url.clone(), agent_info.clone());
                let agent_url = agent_url.clone();
                async move {
                    let agent: serde_json::Value = std::fs::read(&agent_info)
                        .ok()
                        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
                    Ok::<_, StatusCode>((
                        [("Cache-Control", "no-store")],
                        Json(serde_json::json!({
                            "nsec": nsec, "serverUrl": server_url,
                            "agentAccountId": agent["account_id"], "agentNpub": agent["npub"],
                            "agentUrl": agent_url, "room": "wasm-hermes-room",
                        })),
                    ))
                }
            }),
        )
        // Only the public identity is supplied to the real agent for admission.
        .route(
            "/spike/user",
            get(move || {
                let user_id = user_id.clone();
                async move { Json(serde_json::json!({"account_id": user_id})) }
            }),
        )
        .layer(
            CorsLayer::new()
                .allow_origin(origin.parse::<axum::http::HeaderValue>()?)
                .allow_methods([axum::http::Method::POST])
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::AUTHORIZATION,
                ]),
        );
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    println!("FiniteChat relay on 127.0.0.1:{port}; signed requests required; no echo worker");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}
