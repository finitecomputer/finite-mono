use anyhow::{Context, Result};
use clap::Parser;
use finite_private_limiter::{LimiterConfig, WatchdogConfig, app};
use std::env;
use std::net::SocketAddr;
use std::time::Duration;
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
    let app = app(config_from_env()?)?;
    let listener = TcpListener::bind(args.listen_addr).await?;
    println!(
        "finite-private-limiter listening on http://{}",
        args.listen_addr
    );
    axum::serve(listener, app).await?;
    Ok(())
}

fn config_from_env() -> Result<LimiterConfig> {
    let mut config = LimiterConfig::new(
        required_env("FINITE_USAGE_API_URL")?,
        required_env("FINITE_USAGE_API_SERVICE_KEY")?,
        required_env("UPSTREAM_BASE_URL")?,
        required_env("VLLM_INTERNAL_API_KEY")?,
        env::var("DASHBOARD_URL")
            .unwrap_or_else(|_| "https://finite.computer/dashboard".to_string()),
    );
    config.upstream_health_path =
        env::var("UPSTREAM_HEALTH_PATH").unwrap_or_else(|_| "/health".to_string());
    config.usage_api_health_path = env::var("FINITE_USAGE_API_HEALTH_PATH")
        .unwrap_or_else(|_| "/internal/finite-private/v1/health".to_string());
    config.default_model = env::var("FINITE_PRIVATE_MODEL")
        .or_else(|_| env::var("DEFAULT_MODEL"))
        .unwrap_or(config.default_model);
    config.readiness_timeout = duration_env("READINESS_TIMEOUT_SECS", config.readiness_timeout)?;
    config.usage_api_timeout =
        duration_env("FINITE_USAGE_API_TIMEOUT_SECS", config.usage_api_timeout)?;
    config.upstream_first_byte_timeout = duration_env(
        "UPSTREAM_FIRST_BYTE_TIMEOUT_SECS",
        config.upstream_first_byte_timeout,
    )?;
    config.upstream_body_timeout =
        duration_env("UPSTREAM_BODY_TIMEOUT_SECS", config.upstream_body_timeout)?;
    config.upstream_stream_idle_timeout = duration_env(
        "UPSTREAM_STREAM_IDLE_TIMEOUT_SECS",
        config.upstream_stream_idle_timeout,
    )?;
    config.watchdog = WatchdogConfig {
        enabled: bool_env("FINITE_PRIVATE_WATCHDOG_ENABLED")?,
        interval: duration_env(
            "FINITE_PRIVATE_WATCHDOG_INTERVAL_SECS",
            Duration::from_secs(30),
        )?,
        failure_threshold: u32_env("FINITE_PRIVATE_WATCHDOG_FAILURE_THRESHOLD", 3)?,
        restart_command: env::var("FINITE_PRIVATE_WATCHDOG_RESTART_COMMAND")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        exit_after_failures: bool_env("FINITE_PRIVATE_WATCHDOG_EXIT_AFTER_FAILURES")?,
    };
    Ok(config)
}

fn required_env(name: &'static str) -> Result<String> {
    let value = env::var(name).with_context(|| format!("{name} is required"))?;
    if value.trim().is_empty() {
        anyhow::bail!("{name} is required");
    }
    Ok(value)
}

fn duration_env(name: &'static str, default: Duration) -> Result<Duration> {
    let Ok(raw) = env::var(name) else {
        return Ok(default);
    };
    let seconds = raw
        .trim()
        .parse::<u64>()
        .with_context(|| format!("{name} must be an integer number of seconds"))?;
    if seconds == 0 {
        anyhow::bail!("{name} must be greater than zero");
    }
    Ok(Duration::from_secs(seconds))
}

fn u32_env(name: &'static str, default: u32) -> Result<u32> {
    let Ok(raw) = env::var(name) else {
        return Ok(default);
    };
    let value = raw
        .trim()
        .parse::<u32>()
        .with_context(|| format!("{name} must be an integer"))?;
    if value == 0 {
        anyhow::bail!("{name} must be greater than zero");
    }
    Ok(value)
}

fn bool_env(name: &'static str) -> Result<bool> {
    let Ok(raw) = env::var(name) else {
        return Ok(false);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "no" | "off" => Ok(false),
        "1" | "true" | "yes" | "on" => Ok(true),
        _ => anyhow::bail!("{name} must be a boolean"),
    }
}
