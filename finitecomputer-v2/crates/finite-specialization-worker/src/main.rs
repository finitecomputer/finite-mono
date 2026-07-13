use anyhow::Context;
use clap::{Parser, Subcommand};
use finite_specialization_worker::{WorkerConfig, app};
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[derive(Debug, Parser)]
#[command(name = "finite-specialization-worker")]
#[command(about = "Shared Finite specialization worker")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve(ServeArgs),
}

#[derive(Debug, Parser)]
struct ServeArgs {
    #[arg(long, default_value = "0.0.0.0")]
    host: String,
    #[arg(long, default_value_t = 18998)]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| {
                "finite_specialization_worker=info,tower_http=warn".to_string()
            }),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Serve(args) => serve(args).await,
    }
}

async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    let config = WorkerConfig::from_env(args.host, args.port);
    let addr: SocketAddr = format!("{}:{}", config.bind_host, config.bind_port)
        .parse()
        .with_context(|| {
            format!(
                "invalid bind address {}:{}",
                config.bind_host, config.bind_port
            )
        })?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    axum::serve(listener, app(config))
        .await
        .context("finite specialization worker failed")
}
