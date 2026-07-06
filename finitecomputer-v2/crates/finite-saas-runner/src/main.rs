use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use finite_saas_runner::{
    AgentCreationRunner, CoreHttpAgentCreationQueue, DEFAULT_FINITE_AGENT_PICTURE_URL,
    DEFAULT_FINITE_PRIVATE_BASE_URL, DEFAULT_FINITE_PRIVATE_MODEL, DEFAULT_FINITECHAT_SERVER_URL,
    DockerConfig, DockerLauncher, FinitePrivateRuntimeDefaults, PhalaConfig, PhalaLauncher,
    RandomLeaseTokenSource, RuntimeLauncher,
};
use std::env;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "finite-saas-runner")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Claim at most one Core agent creation request and try to launch it.
    #[command(name = "run-once")]
    RunOnce,
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command.unwrap_or(Command::RunOnce) {
        Command::RunOnce => run_once(),
    }
}

fn run_once() -> Result<()> {
    let queue = CoreHttpAgentCreationQueue::new(
        required_env("FC_CORE_URL")?,
        required_env("FC_CORE_API_TOKEN")?,
    )?;
    let runtime_artifact_id = required_env("FC_RUNNER_RUNTIME_ARTIFACT_ID")?;
    let runtime_artifact = queue.runtime_artifact(&runtime_artifact_id)?;
    let runner_id = required_env("FC_RUNNER_ID")?;
    let lease_seconds = optional_i64("FC_RUNNER_LEASE_SECONDS", 600)?;
    let runtime_ready_timeout =
        Duration::from_secs(optional_u64("FC_RUNNER_RUNTIME_READY_TIMEOUT_SECS", 120)?);
    let runtime_ready_interval =
        Duration::from_millis(optional_u64("FC_RUNNER_RUNTIME_READY_INTERVAL_MS", 2_000)?);
    let finite_private_base_url = optional_env(
        "FC_RUNNER_FINITE_PRIVATE_BASE_URL",
        DEFAULT_FINITE_PRIVATE_BASE_URL,
    );
    let finite_private_model = optional_env(
        "FC_RUNNER_FINITE_PRIVATE_MODEL",
        DEFAULT_FINITE_PRIVATE_MODEL,
    );
    let finite_private_api_key_override =
        optional_env_value("FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE");
    let backend = optional_env("FC_RUNNER_BACKEND", "docker").to_ascii_lowercase();
    let outcome = match backend.as_str() {
        "docker" => {
            let launcher = DockerLauncher::new(DockerConfig {
                docker_bin: optional_path("FC_RUNNER_DOCKER_BIN", "docker"),
                source_host_id: required_env("FC_RUNNER_SOURCE_HOST_ID")?,
                image: runtime_artifact.reference,
                runtime_artifact_id: Some(runtime_artifact.id),
                runtime_artifact_kind: Some(runtime_artifact.kind),
                runtime_state_schema_version: Some(runtime_artifact.state_schema_version),
                work_root: required_path("FC_RUNNER_WORK_ROOT")?,
                finitechat_server_url: optional_env(
                    "FC_RUNNER_FINITECHAT_SERVER_URL",
                    DEFAULT_FINITECHAT_SERVER_URL,
                ),
                agent_picture_url: optional_env(
                    "FC_RUNNER_AGENT_PICTURE_URL",
                    DEFAULT_FINITE_AGENT_PICTURE_URL,
                ),
                host_port: optional_u16("FC_RUNNER_DOCKER_HOST_PORT", 18080)?,
                container_port: optional_u16("FC_RUNNER_DOCKER_CONTAINER_PORT", 8080)?,
                public_base_url: optional_env_value("FC_RUNNER_DOCKER_PUBLIC_BASE_URL"),
                pull_policy: optional_env_value("FC_RUNNER_DOCKER_PULL_POLICY")
                    .or_else(|| Some("missing".to_string())),
                max_container_count: optional_u32_value("FC_RUNNER_MAX_SANDBOXES")?,
                drain_new_leases: optional_bool("FC_RUNNER_DRAIN", false)?,
                available_memory_bytes: host_available_memory_bytes(),
                command_timeout: Duration::from_secs(optional_u64(
                    "FC_RUNNER_COMMAND_TIMEOUT_SECS",
                    15,
                )?),
                launch_timeout: Duration::from_secs(optional_u64(
                    "FC_RUNNER_LAUNCH_TIMEOUT_SECS",
                    300,
                )?),
                readiness_timeout: runtime_ready_timeout,
                readiness_interval: runtime_ready_interval,
            });
            run_once_with_launcher(
                queue,
                launcher,
                RunOnceConfig {
                    runner_id,
                    lease_seconds,
                    runtime_ready_timeout,
                    runtime_ready_interval,
                    finite_private_base_url,
                    finite_private_model,
                    finite_private_api_key_override,
                },
            )?
        }
        "phala" => {
            let launcher = PhalaLauncher::new(PhalaConfig {
                phala_bin: optional_path("FC_RUNNER_PHALA_BIN", "phala"),
                api_key: required_env_any(&["FC_RUNNER_PHALA_API_KEY", "PHALA_CLOUD_API_KEY"])?,
                source_host_id: required_env("FC_RUNNER_SOURCE_HOST_ID")?,
                image: runtime_artifact.reference,
                runtime_artifact_id: Some(runtime_artifact.id),
                runtime_artifact_kind: Some(runtime_artifact.kind),
                runtime_state_schema_version: Some(runtime_artifact.state_schema_version),
                work_root: required_path("FC_RUNNER_WORK_ROOT")?,
                finitechat_server_url: optional_env(
                    "FC_RUNNER_FINITECHAT_SERVER_URL",
                    DEFAULT_FINITECHAT_SERVER_URL,
                ),
                agent_picture_url: optional_env(
                    "FC_RUNNER_AGENT_PICTURE_URL",
                    DEFAULT_FINITE_AGENT_PICTURE_URL,
                ),
                instance_type: optional_env("FC_RUNNER_PHALA_INSTANCE_TYPE", "tdx.small"),
                disk_size: optional_env("FC_RUNNER_PHALA_DISK_SIZE", "40G"),
                region: optional_env_value("FC_RUNNER_PHALA_REGION"),
                kms: optional_env("FC_RUNNER_PHALA_KMS", "phala"),
                public_logs: optional_bool("FC_RUNNER_PHALA_PUBLIC_LOGS", false)?,
                public_sysinfo: optional_bool("FC_RUNNER_PHALA_PUBLIC_SYSINFO", false)?,
                max_cvm_count: optional_u32_value("FC_RUNNER_MAX_SANDBOXES")?,
                drain_new_leases: optional_bool("FC_RUNNER_DRAIN", false)?,
                available_memory_bytes: host_available_memory_bytes(),
                command_timeout: Duration::from_secs(optional_u64(
                    "FC_RUNNER_COMMAND_TIMEOUT_SECS",
                    15,
                )?),
                launch_timeout: Duration::from_secs(optional_u64(
                    "FC_RUNNER_LAUNCH_TIMEOUT_SECS",
                    900,
                )?),
                readiness_timeout: runtime_ready_timeout,
                readiness_interval: runtime_ready_interval,
            });
            run_once_with_launcher(
                queue,
                launcher,
                RunOnceConfig {
                    runner_id,
                    lease_seconds,
                    runtime_ready_timeout,
                    runtime_ready_interval,
                    finite_private_base_url,
                    finite_private_model,
                    finite_private_api_key_override,
                },
            )?
        }
        other => bail!("FC_RUNNER_BACKEND must be docker or phala, got {other:?}"),
    };
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(())
}

struct RunOnceConfig {
    runner_id: String,
    lease_seconds: i64,
    runtime_ready_timeout: Duration,
    runtime_ready_interval: Duration,
    finite_private_base_url: String,
    finite_private_model: String,
    finite_private_api_key_override: Option<String>,
}

fn run_once_with_launcher<L>(
    queue: CoreHttpAgentCreationQueue,
    launcher: L,
    config: RunOnceConfig,
) -> Result<finite_saas_runner::RunOnceOutcome>
where
    L: RuntimeLauncher,
{
    let mut runner = AgentCreationRunner::new(
        queue,
        launcher,
        RandomLeaseTokenSource,
        config.runner_id,
        config.lease_seconds,
    )?
    .with_default_finite_private_inference(FinitePrivateRuntimeDefaults {
        base_url: config.finite_private_base_url,
        model: config.finite_private_model,
        api_key_override: config.finite_private_api_key_override,
    })
    .with_runtime_ready_polling(config.runtime_ready_timeout, config.runtime_ready_interval);
    runner.run_once().map_err(Into::into)
}

fn required_env(name: &str) -> Result<String> {
    let value = env::var(name).with_context(|| format!("{name} is required"))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("{name} must not be empty");
    }
    Ok(trimmed.to_string())
}

fn required_env_any(names: &[&str]) -> Result<String> {
    for name in names {
        if let Some(value) = optional_env_value(name) {
            return Ok(value);
        }
    }
    bail!("one of {} is required", names.join(", "))
}

fn optional_env(name: &str, default: &str) -> String {
    optional_env_value(name).unwrap_or_else(|| default.to_string())
}

fn optional_env_value(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn required_path(name: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(required_env(name)?))
}

fn optional_path(name: &str, default: &str) -> PathBuf {
    PathBuf::from(optional_env(name, default))
}

fn optional_i64(name: &str, default: i64) -> Result<i64> {
    let value = optional_env(name, &default.to_string());
    value
        .parse::<i64>()
        .with_context(|| format!("{name} must be an integer"))
}

fn optional_u64(name: &str, default: u64) -> Result<u64> {
    let value = optional_env(name, &default.to_string());
    value
        .parse::<u64>()
        .with_context(|| format!("{name} must be an integer"))
}

fn optional_u16(name: &str, default: u16) -> Result<u16> {
    let value = optional_env(name, &default.to_string());
    value
        .parse::<u16>()
        .with_context(|| format!("{name} must be an integer between 1 and 65535"))
}

fn optional_u32_value(name: &str) -> Result<Option<u32>> {
    optional_env_value(name)
        .map(|value| {
            value
                .parse::<u32>()
                .with_context(|| format!("{name} must be an integer"))
        })
        .transpose()
}

fn optional_bool(name: &str, default: bool) -> Result<bool> {
    let Some(value) = optional_env_value(name) else {
        return Ok(default);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "enabled" => Ok(true),
        "0" | "false" | "no" | "off" | "disabled" => Ok(false),
        _ => bail!("{name} must be a boolean"),
    }
}

fn host_available_memory_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in meminfo.lines() {
        let Some(rest) = line.strip_prefix("MemAvailable:") else {
            continue;
        };
        let kib = rest.split_whitespace().next()?.parse::<u64>().ok()?;
        return kib.checked_mul(1024);
    }
    None
}
