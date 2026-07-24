use std::env;
use std::io::{self, BufRead, Write};
use std::net::SocketAddr;
use std::time::Duration;

use clap::{Parser, Subcommand};
use finitechat_core::{FiniteChatRuntime, OpenOptions};
use finitechat_daemon::device_link::{DeviceLinkBootstrapOptions, create_device_link_session};
use finitechat_daemon::{
    DEFAULT_BIND, DEFAULT_SERVER_URL, DaemonError, app_with_data_dir, read_startup_secrets,
    validate_loopback_bind,
};
use serde_json::json;

#[derive(Debug, Parser)]
#[command(name = "finitechatd")]
#[command(about = "Finite Chat local daemon for thin native and Electron clients")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
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

#[derive(Debug, Subcommand)]
enum Command {
    /// Link this local Device to the signed-in Finite Computer account.
    Link {
        #[arg(long, default_value = DEFAULT_SERVER_URL)]
        server_url: String,
        #[arg(long)]
        device_id: String,
        #[arg(long, default_value_t = 3)]
        result_fd: i32,
        #[arg(long, default_value_t = 4)]
        confirm_fd: i32,
        #[arg(long, default_value_t = 600)]
        timeout_seconds: u64,
    },
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
    if let Some(Command::Link {
        server_url,
        device_id,
        result_fd,
        confirm_fd,
        timeout_seconds,
    }) = args.command
    {
        return run_device_link(
            server_url,
            device_id,
            result_fd,
            confirm_fd,
            timeout_seconds,
        )
        .await;
    }
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
        data_dir: data_dir.clone(),
        server_url,
        device_id,
        account_secret_hex: startup.account_secret_hex,
        now_unix_seconds,
    })?;
    let router = app_with_data_dir(runtime, startup.auth_token, data_dir)?;
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

async fn run_device_link(
    server_url: String,
    device_id: String,
    result_fd: i32,
    confirm_fd: i32,
    timeout_seconds: u64,
) -> Result<(), DaemonError> {
    let (mut result_pipe, confirmation_pipe) = supervisor_pipes(result_fd, confirm_fd)?;
    let mut options = DeviceLinkBootstrapOptions::internal_alpha(server_url, device_id);
    options.timeout = Duration::from_secs(timeout_seconds);
    let session = create_device_link_session(options).await?;
    println!("{}", serde_json::to_string(session.ready())?);
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::Task("device-link status pipe failed".to_owned()))?;

    let claimed = session.wait_for_claim().await?;
    if claimed.write_secret_result(&mut result_pipe).is_err() {
        claimed.release().await;
        return Err(DaemonError::Task(
            "device-link result pipe failed".to_owned(),
        ));
    }
    let confirmed = tokio::task::spawn_blocking(move || {
        let mut confirmation = String::new();
        let read = io::BufReader::new(confirmation_pipe).read_line(&mut confirmation)?;
        Ok::<bool, io::Error>(read > 0 && confirmation.trim() == "stored")
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or(false);
    if !confirmed {
        claimed.release().await;
        return Err(DaemonError::Task(
            "device-link storage confirmation failed".to_owned(),
        ));
    }
    claimed.acknowledge_stored().await?;
    println!("{}", json!({ "event": "linked" }));
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::Task("device-link status pipe failed".to_owned()))
}

#[cfg(unix)]
fn supervisor_pipes(
    result_fd: i32,
    confirm_fd: i32,
) -> Result<(std::fs::File, std::fs::File), DaemonError> {
    use std::os::fd::FromRawFd;

    if result_fd < 3 || confirm_fd < 3 || result_fd == confirm_fd {
        return Err(DaemonError::Task(
            "invalid device-link supervisor pipes".to_owned(),
        ));
    }
    // SAFETY: Electron creates two dedicated child-only pipe descriptors and
    // transfers their ownership to this process for the link command.
    let result = unsafe { std::fs::File::from_raw_fd(result_fd) };
    // SAFETY: same ownership contract as `result_fd`; the descriptors are
    // validated as distinct above.
    let confirmation = unsafe { std::fs::File::from_raw_fd(confirm_fd) };
    Ok((result, confirmation))
}

#[cfg(not(unix))]
fn supervisor_pipes(
    _result_fd: i32,
    _confirm_fd: i32,
) -> Result<(std::fs::File, std::fs::File), DaemonError> {
    Err(DaemonError::Task(
        "device linking is unavailable on this alpha platform".to_owned(),
    ))
}
