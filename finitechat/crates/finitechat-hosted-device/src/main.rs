use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use finitechat_hosted_device::{
    HostedDeviceConfig, HostedIdentityAuthorityConfig, app, app_with_identity_authority,
};

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
    let identity_authority = match (
        env::var("FINITE_IDENTITY_AUTHORITY").ok(),
        env::var("FINITE_IDENTITY_OPERATOR_TOKEN").ok(),
    ) {
        (Some(base_url), Some(operator_token))
            if !base_url.trim().is_empty() && !operator_token.trim().is_empty() =>
        {
            Some(HostedIdentityAuthorityConfig {
                base_url,
                operator_token,
            })
        }
        (None, None) => None,
        _ => anyhow::bail!(
            "FINITE_IDENTITY_AUTHORITY and FINITE_IDENTITY_OPERATOR_TOKEN must be configured together"
        ),
    };
    let config = HostedDeviceConfig {
        data_root: args.data_root,
        server_url: args.server_url,
        public_url: args.public_url,
        api_token,
    };
    let router = if let Some(identity_authority) = identity_authority {
        app_with_identity_authority(config, identity_authority)
    } else {
        app(config)
    };
    axum::serve(listener, router)
        .await
        .context("hosted device server failed")
}
