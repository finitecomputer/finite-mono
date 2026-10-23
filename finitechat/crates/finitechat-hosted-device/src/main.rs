use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use finitechat_hosted_device::{HostedDeviceConfig, app, digest_all_users};

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
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print one JSON line per hosted user with hashes of what they see in
    /// each room (no message text). Read-only; safe beside the service.
    Digest,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if let Some(Command::Digest) = args.command {
        let digests = tokio::task::spawn_blocking(move || {
            digest_all_users(&args.data_root, &args.server_url)
        })
        .await
        .context("digest task failed")?
        .map_err(|error| anyhow::anyhow!("digest failed: {error}"))?;
        for digest in digests {
            println!("{digest}");
        }
        return Ok(());
    }
    let api_token = env::var("FINITECHAT_HOSTED_API_TOKEN")
        .context("FINITECHAT_HOSTED_API_TOKEN is required")?;
    if api_token.trim().is_empty() {
        anyhow::bail!("FINITECHAT_HOSTED_API_TOKEN must not be empty");
    }
    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("failed to bind {}", args.bind))?;
    let config = HostedDeviceConfig {
        data_root: args.data_root,
        server_url: args.server_url,
        public_url: args.public_url,
        api_token,
    };
    axum::serve(listener, app(config))
        .await
        .context("hosted device server failed")
}
