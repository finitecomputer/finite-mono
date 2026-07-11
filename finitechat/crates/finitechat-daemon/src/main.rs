use std::env;
use std::io::{self, Write};
use std::net::SocketAddr;

use clap::Parser;
use finitechat_core::{FiniteChatRuntime, OpenOptions};
use finitechat_daemon::{
    DEFAULT_BIND, DEFAULT_SERVER_URL, DaemonError, app, read_startup_secrets,
    validate_loopback_bind,
};
use serde_json::json;

#[derive(Debug, Parser)]
#[command(name = "finitechatd")]
#[command(about = "Finite Chat local daemon for thin native and Electron clients")]
struct Args {
    #[arg(long, default_value = DEFAULT_BIND)]
    bind: SocketAddr,
    #[arg(long)]
    data_dir: Option<String>,
    #[arg(long)]
    server_url: Option<String>,
    #[arg(long)]
    device_id: Option<String>,
    #[arg(long)]
    now_unix_seconds: Option<u64>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[tokio::main]
async fn run() -> Result<(), DaemonError> {
    let args = Args::parse();
    validate_loopback_bind(args.bind)?;
    let startup = read_startup_secrets(io::stdin().lock())?;
    let data_dir = args
        .data_dir
        .or_else(|| env::var("FINITECHAT_HOME").ok())
        .ok_or(DaemonError::MissingOption("--data-dir or FINITECHAT_HOME"))?;
    let server_url = args
        .server_url
        .or_else(|| env::var("FINITECHAT_SERVER_URL").ok())
        .unwrap_or_else(|| DEFAULT_SERVER_URL.to_owned());
    let device_id = args
        .device_id
        .or_else(|| env::var("FINITECHAT_DEVICE_ID").ok())
        .unwrap_or_else(|| "electron".to_owned());
    let now_unix_seconds = args.now_unix_seconds.or_else(|| {
        env::var("FINITECHAT_FIXED_NOW_UNIX_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
    });
    let runtime = FiniteChatRuntime::open(OpenOptions {
        data_dir,
        server_url,
        device_id,
        account_secret_hex: startup.account_secret_hex,
        now_unix_seconds,
    })?;
    let router = app(runtime, startup.auth_token)?;
    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .map_err(|error| DaemonError::Task(error.to_string()))?;
    let address = listener
        .local_addr()
        .map_err(|error| DaemonError::Task(error.to_string()))?;
    println!(
        "{}",
        json!({
            "event": "ready",
            "url": format!("http://{address}"),
        })
    );
    io::stdout()
        .flush()
        .map_err(|error| DaemonError::Task(error.to_string()))?;
    axum::serve(listener, router)
        .await
        .map_err(|error| DaemonError::Task(error.to_string()))
}
