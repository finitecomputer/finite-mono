use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use finitechat_hosted_device::{HostedDeviceConfig, app};

const DEFAULT_BIND: &str = "127.0.0.1:38918";
const DEFAULT_SERVER_URL: &str = "https://chat.finite.computer";

#[derive(Debug, Parser)]
#[command(name = "finitechat-hosted-device")]
#[command(about = "WorkOS-gated Hosted Web Devices for Finite Chat")]
struct Args {
    #[arg(long, env = "FINITECHAT_HOSTED_BIND", default_value = DEFAULT_BIND)]
    bind: SocketAddr,
    #[arg(long, env = "FINITECHAT_HOSTED_DATA_ROOT")]
    data_root: PathBuf,
    #[arg(long, env = "FINITECHAT_SERVER_URL", default_value = DEFAULT_SERVER_URL)]
    server_url: String,
    #[arg(long, env = "FINITECHAT_PUBLIC_URL", default_value = DEFAULT_SERVER_URL)]
    public_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let api_token = env::var("FINITECHAT_HOSTED_API_TOKEN")
        .context("FINITECHAT_HOSTED_API_TOKEN is required")?;
    if api_token.trim().is_empty() {
        anyhow::bail!("FINITECHAT_HOSTED_API_TOKEN must not be empty");
    }
    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("failed to bind {}", args.bind))?;
    axum::serve(
        listener,
        app(HostedDeviceConfig {
            data_root: args.data_root,
            server_url: args.server_url,
            public_url: args.public_url,
            api_token,
        }),
    )
    .await
    .context("hosted device server failed")
}
