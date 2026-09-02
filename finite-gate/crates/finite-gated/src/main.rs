//! `finite-gated` — the Finite Auth Gate daemon. See `src/lib.rs` for the
//! protocol and the environment variable contract.

use std::sync::Arc;

use anyhow::{Context, Result};

fn main() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new().context("cannot start runtime")?;
    runtime.block_on(run())
}

async fn run() -> Result<()> {
    let config = finite_gated::config::GateConfig::from_env()?;
    let listen = config.listen;
    let dev_mode = config.dev_mode();
    let public_url = config.public_url.clone();
    let state = Arc::new(finite_gated::state::GateState::new(config));
    // The public key is printed so operators can pin it as
    // FINITE_SITES_AUTH_GATE_PUBKEY; the secret itself never appears.
    let public_key = state.config.public_key_hex()?;
    let app =
        finite_gated::router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("cannot bind {listen}"))?;
    eprintln!(
        "finite-gated listening on {listen} (public url: {public_url}, mode: {}) gate pubkey: {public_key}",
        if dev_mode { "DEV" } else { "WORKOS" },
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("finite-gated shutting down");
        })
        .await
        .context("server error")
}
