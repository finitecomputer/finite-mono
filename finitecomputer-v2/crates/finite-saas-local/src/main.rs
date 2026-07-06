use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use finite_saas_local::{
    ChainedLimiterInputs, DEFAULT_DEPLOYED_LIMITER_BASE_URL, DEFAULT_FINITE_PRIVATE_MODEL,
    DEFAULT_LIMITER_LISTEN_ADDR, UPSTREAM_KEY_ENV, agent_base_url, chained_limiter_config,
    client_host, health_url, parse_listen_addr, wait_until_ready,
};
use std::time::Duration;

/// Local development harness for finitecomputer-v2 (PRD Phase 4).
#[derive(Debug, Parser)]
#[command(name = "finite-saas-local")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the in-tree Finite Private limiter chained in front of the
    /// deployed limiter, so locally provisioned (local-Core) keys work for
    /// real inference: agents -> this limiter (local admission/metering) ->
    /// deployed limiter (one operator key from
    /// FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY) -> glm-5-2.
    #[command(name = "finite-private-limiter-up")]
    FinitePrivateLimiterUp {
        /// Address the chained limiter listens on. Port 0 picks a free port.
        #[arg(
            long,
            env = "FC_LOCAL_LIMITER_LISTEN_ADDR",
            default_value = DEFAULT_LIMITER_LISTEN_ADDR
        )]
        listen_addr: String,
        /// Local Core base URL (the limiter's usage/admission API).
        #[arg(long, env = "FC_CORE_URL", default_value = "http://127.0.0.1:14200")]
        core_url: String,
        /// Local Core service token.
        #[arg(long, env = "FC_CORE_API_TOKEN")]
        core_api_token: String,
        /// Deployed Finite Private limiter, as agents address it (a trailing
        /// /v1 is expected and stripped for upstream proxying).
        #[arg(
            long,
            env = "FC_LOCAL_FINITE_PRIVATE_UPSTREAM_BASE_URL",
            default_value = DEFAULT_DEPLOYED_LIMITER_BASE_URL
        )]
        upstream_base_url: String,
        /// Dashboard URL surfaced in limit-denied error messages.
        #[arg(
            long,
            env = "FC_LOCAL_LIMITER_DASHBOARD_URL",
            default_value = "http://127.0.0.1:13002/dashboard"
        )]
        dashboard_url: String,
        /// Hostname agents use to reach this limiter from inside Docker.
        #[arg(
            long,
            env = "FC_LOCAL_LIMITER_AGENT_HOST",
            default_value = "host.docker.internal"
        )]
        agent_host: String,
        /// Seconds to wait for the limiter's /health readiness check.
        #[arg(long, default_value_t = 30)]
        ready_timeout_secs: u64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::FinitePrivateLimiterUp {
            listen_addr,
            core_url,
            core_api_token,
            upstream_base_url,
            dashboard_url,
            agent_host,
            ready_timeout_secs,
        } => {
            finite_private_limiter_up(
                &listen_addr,
                ChainedLimiterInputs {
                    core_url,
                    core_api_token,
                    upstream_base_url,
                    // Secret comes from env only, never argv.
                    upstream_api_key: std::env::var(UPSTREAM_KEY_ENV)
                        .ok()
                        .filter(|value| !value.trim().is_empty()),
                    dashboard_url,
                },
                &agent_host,
                Duration::from_secs(ready_timeout_secs),
            )
            .await
        }
    }
}

async fn finite_private_limiter_up(
    listen_addr: &str,
    inputs: ChainedLimiterInputs,
    agent_host: &str,
    ready_timeout: Duration,
) -> Result<()> {
    let requested_addr = parse_listen_addr(listen_addr)?;
    let config = chained_limiter_config(&inputs)?;
    let upstream_root = config.upstream_base_url.clone();
    let core_url = config.finite_usage_api_url.clone();
    let router = finite_private_limiter::app(config)
        .context("failed to build the chained finite-private-limiter")?;

    let listener = tokio::net::TcpListener::bind(requested_addr)
        .await
        .with_context(|| format!("failed to bind chained limiter on {requested_addr}"))?;
    let bound_addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .context("chained limiter server exited")
    });

    let ready_url = health_url(&bound_addr);
    let client = reqwest::Client::new();
    let probe = || {
        let client = client.clone();
        let ready_url = ready_url.clone();
        async move {
            matches!(
                client.get(&ready_url).send().await,
                Ok(response) if response.status().is_success()
            )
        }
    };
    wait_until_ready(probe, ready_timeout, Duration::from_millis(250))
        .await
        .with_context(|| format!("chained limiter never became healthy at {ready_url}"))?;

    println!("finite-saas-local: chained Finite Private limiter is ready");
    println!("  listen:          http://{bound_addr}");
    println!("  health:          {ready_url}");
    println!("  local Core:      {core_url}");
    println!("  upstream:        {upstream_root} (serves {DEFAULT_FINITE_PRIVATE_MODEL})");
    println!(
        "  host clients:    {}",
        agent_base_url(&client_host(&bound_addr), bound_addr.port())
    );
    println!(
        "  agents (Docker): FC_RUNNER_FINITE_PRIVATE_BASE_URL={}",
        agent_base_url(agent_host, bound_addr.port())
    );
    println!("Press Ctrl-C to stop.");

    server.await??;
    Ok(())
}
