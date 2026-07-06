use anyhow::{Context, Result};
use clap::Parser;
use finite_private_limiter::{LimiterConfig, app};
use std::env;
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[derive(Debug, Parser)]
#[command(name = "finite-private-limiter")]
struct Args {
    #[arg(long, env = "LISTEN_ADDR", default_value = "0.0.0.0:8002")]
    listen_addr: SocketAddr,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let app = app(LimiterConfig {
        finite_usage_api_url: required_env("FINITE_USAGE_API_URL")?,
        finite_usage_api_service_key: required_env("FINITE_USAGE_API_SERVICE_KEY")?,
        upstream_base_url: required_env("UPSTREAM_BASE_URL")?,
        vllm_internal_api_key: required_env("VLLM_INTERNAL_API_KEY")?,
        dashboard_url: env::var("DASHBOARD_URL")
            .unwrap_or_else(|_| "https://finite.computer/dashboard".to_string()),
    })?;
    let listener = TcpListener::bind(args.listen_addr).await?;
    println!(
        "finite-private-limiter listening on http://{}",
        args.listen_addr
    );
    axum::serve(listener, app).await?;
    Ok(())
}

fn required_env(name: &'static str) -> Result<String> {
    let value = env::var(name).with_context(|| format!("{name} is required"))?;
    if value.trim().is_empty() {
        anyhow::bail!("{name} is required");
    }
    Ok(value)
}
