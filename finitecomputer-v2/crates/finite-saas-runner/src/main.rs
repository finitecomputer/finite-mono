use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use finite_saas_runner::phala::PhalaApiClient;
use finite_saas_runner::{
    AgentCreationRunner, AppleContainerConfig, AppleContainerLauncher, CoreHttpAgentCreationQueue,
    DEFAULT_FINITE_AGENT_PICTURE_URL, DEFAULT_FINITE_PRIVATE_BASE_URL,
    DEFAULT_FINITE_PRIVATE_MODEL, DEFAULT_FINITECHAT_SERVER_URL, DockerConfig, DockerLauncher,
    EnclaviaConfig, EnclaviaLauncher, FinitePrivateRuntimeDefaults, KataConfig, KataLauncher,
    PhalaConfig, PhalaLauncher, RandomLeaseTokenSource, RunOnceOutcome, RuntimeLauncher,
};
use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;
use std::thread;
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
    /// Continuously process generic runtime lifecycle work.
    #[command(name = "serve")]
    Serve,
    /// Run authenticated, read-only Phala contract and inventory checks.
    #[command(name = "phala-preflight")]
    PhalaPreflight,
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command.unwrap_or(Command::RunOnce) {
        Command::RunOnce => run_once(),
        Command::Serve => serve(),
        Command::PhalaPreflight => phala_preflight(),
    }
}

fn phala_preflight() -> Result<()> {
    let api_key = required_env_any(&["FC_RUNNER_PHALA_API_KEY", "PHALA_CLOUD_API_KEY"])?;
    let expected_workspace_id = required_env("FC_RUNNER_PHALA_EXPECTED_WORKSPACE_ID")?;
    let expected_workspace_slug = required_env("FC_RUNNER_PHALA_EXPECTED_WORKSPACE_SLUG")?;
    let summary = PhalaApiClient::new(api_key)?
        .preflight_summary(&expected_workspace_id, &expected_workspace_slug)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn run_once() -> Result<()> {
    let outcome = run_cycle()?;
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(())
}

fn run_cycle() -> Result<RunOnceOutcome> {
    let queue = CoreHttpAgentCreationQueue::new(
        required_env("FC_CORE_URL")?,
        required_env("FC_CORE_RUNNER_API_TOKEN")?,
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
    let runtime_environment = optional_runtime_environment()?;
    let runtime_secret_environment = optional_runtime_secret_environment()?;
    // This identifies the adapter offered by this worker. Placement remains
    // project-selected in Core; product code never toggles a process-global
    // backend to change an existing agent's runtime.
    let runner_class = optional_env("FC_RUNNER_CLASS", "local_docker")
        .to_ascii_lowercase()
        .replace('-', "_");
    let outcome = match runner_class.as_str() {
        "local_docker" => {
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
                    runtime_environment,
                    runtime_secret_environment,
                },
            )?
        }
        "apple_container" => {
            let expected_image_descriptor_digest =
                immutable_oci_descriptor_digest(&runtime_artifact.reference)?;
            let launcher = AppleContainerLauncher::new(AppleContainerConfig {
                container_bin: optional_path("FC_RUNNER_APPLE_CONTAINER_BIN", "container"),
                source_host_id: required_env("FC_RUNNER_SOURCE_HOST_ID")?,
                image: optional_env_value("FC_RUNNER_APPLE_CONTAINER_LOCAL_IMAGE_REFERENCE")
                    .unwrap_or_else(|| runtime_artifact.reference.clone()),
                expected_image_descriptor_digest: Some(expected_image_descriptor_digest),
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
                name_prefix: optional_env("FC_RUNNER_APPLE_CONTAINER_NAME_PREFIX", "finite-apple"),
                host_port: optional_u16("FC_RUNNER_APPLE_CONTAINER_HOST_PORT", 18080)?,
                container_port: optional_u16("FC_RUNNER_APPLE_CONTAINER_CONTAINER_PORT", 8080)?,
                public_base_url: optional_env_value("FC_RUNNER_APPLE_CONTAINER_PUBLIC_BASE_URL"),
                platform: optional_env_value("FC_RUNNER_APPLE_CONTAINER_PLATFORM")
                    .or_else(|| Some("linux/arm64".to_string())),
                rosetta: optional_bool("FC_RUNNER_APPLE_CONTAINER_ROSETTA", false)?,
                cpus: optional_u32_value("FC_RUNNER_APPLE_CONTAINER_CPUS")?.or(Some(4)),
                memory: optional_env_value("FC_RUNNER_APPLE_CONTAINER_MEMORY")
                    .or_else(|| Some("4G".to_string())),
                max_container_count: optional_u32_value("FC_RUNNER_MAX_SANDBOXES")?.or(Some(1)),
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
                stop_timeout_secs: optional_u64("FC_RUNNER_APPLE_CONTAINER_STOP_TIMEOUT_SECS", 30)?,
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
                    runtime_environment,
                    runtime_secret_environment,
                },
            )?
        }
        "kata" => {
            let launcher = KataLauncher::new(KataConfig {
                nerdctl_bin: optional_path("FC_RUNNER_KATA_NERDCTL_BIN", "nerdctl"),
                kata_runtime_bin: optional_path("FC_RUNNER_KATA_RUNTIME_BIN", "kata-runtime"),
                namespace: optional_env("FC_RUNNER_KATA_NAMESPACE", "finite"),
                runtime: optional_env("FC_RUNNER_KATA_OCI_RUNTIME", "io.containerd.kata.v2"),
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
                name_prefix: optional_env("FC_RUNNER_KATA_NAME_PREFIX", "finite-kata"),
                container_port: optional_u16("FC_RUNNER_KATA_CONTAINER_PORT", 8080)?,
                cpus: optional_u32_value("FC_RUNNER_KATA_CPUS")?.or(Some(4)),
                memory: optional_env_value("FC_RUNNER_KATA_MEMORY")
                    .or_else(|| Some("8G".to_string())),
                pull_policy: optional_env_value("FC_RUNNER_KATA_PULL_POLICY")
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
                    900,
                )?),
                readiness_timeout: runtime_ready_timeout,
                readiness_interval: runtime_ready_interval,
                stop_timeout_secs: optional_u64("FC_RUNNER_KATA_STOP_TIMEOUT_SECS", 30)?,
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
                    runtime_environment,
                    runtime_secret_environment,
                },
            )?
        }
        "phala" => {
            let launcher = PhalaLauncher::new(PhalaConfig {
                expected_workspace_id: required_env("FC_RUNNER_PHALA_EXPECTED_WORKSPACE_ID")?,
                expected_workspace_slug: required_env("FC_RUNNER_PHALA_EXPECTED_WORKSPACE_SLUG")?,
                api_key: required_env_any(&["FC_RUNNER_PHALA_API_KEY", "PHALA_CLOUD_API_KEY"])?,
                source_host_id: required_env("FC_RUNNER_SOURCE_HOST_ID")?,
                image: runtime_artifact.reference,
                runtime_artifact_id: Some(runtime_artifact.id),
                runtime_artifact_kind: Some(runtime_artifact.kind),
                runtime_state_schema_version: Some(runtime_artifact.state_schema_version),
                finitechat_server_url: optional_env(
                    "FC_RUNNER_FINITECHAT_SERVER_URL",
                    DEFAULT_FINITECHAT_SERVER_URL,
                ),
                agent_picture_url: optional_env(
                    "FC_RUNNER_AGENT_PICTURE_URL",
                    DEFAULT_FINITE_AGENT_PICTURE_URL,
                ),
                max_cvm_count: optional_u32_value("FC_RUNNER_MAX_SANDBOXES")?.or(Some(1)),
                drain_new_leases: optional_bool("FC_RUNNER_DRAIN", false)?,
                available_memory_bytes: host_available_memory_bytes(),
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
                    runtime_environment,
                    runtime_secret_environment,
                },
            )?
        }
        "enclavia" => {
            let launcher = EnclaviaLauncher::new(EnclaviaConfig {
                enclavia_bin: optional_path("FC_RUNNER_ENCLAVIA_BIN", "enclavia"),
                docker_bin: optional_path("FC_RUNNER_DOCKER_BIN", "docker"),
                source_host_id: required_env("FC_RUNNER_SOURCE_HOST_ID")?,
                image: runtime_artifact.reference,
                runtime_artifact_id: Some(runtime_artifact.id),
                runtime_artifact_kind: Some(runtime_artifact.kind),
                runtime_state_schema_version: Some(runtime_artifact.state_schema_version),
                finitechat_server_url: optional_env(
                    "FC_RUNNER_FINITECHAT_SERVER_URL",
                    DEFAULT_FINITECHAT_SERVER_URL,
                ),
                agent_picture_url: optional_env(
                    "FC_RUNNER_AGENT_PICTURE_URL",
                    DEFAULT_FINITE_AGENT_PICTURE_URL,
                ),
                enclave_id: required_env("FC_RUNNER_ENCLAVIA_ENCLAVE_ID")?,
                pull_policy: optional_env_value("FC_RUNNER_ENCLAVIA_PULL_POLICY")
                    .or_else(|| Some("missing".to_string())),
                max_enclave_count: optional_u32_value("FC_RUNNER_MAX_SANDBOXES")?.or(Some(1)),
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
                    runtime_environment,
                    runtime_secret_environment,
                },
            )?
        }
        other => bail!(
            "FC_RUNNER_CLASS must be local_docker, apple_container, kata, phala, or enclavia, got {other:?}"
        ),
    };
    Ok(outcome)
}

fn serve() -> Result<()> {
    let idle_interval =
        Duration::from_millis(optional_u64("FC_RUNNER_IDLE_INTERVAL_MS", 1_000)?.max(100));
    let max_error_backoff =
        Duration::from_millis(optional_u64("FC_RUNNER_MAX_ERROR_BACKOFF_MS", 30_000)?.max(1_000));
    let mut error_backoff = idle_interval;
    let mut last_error: Option<String> = None;

    // Fail the supervised process for static configuration or Core artifact
    // lookup errors. Provider inventory/preflight failures are represented as
    // unavailable creation capacity so persisted runtime controls remain
    // serviceable; later cycle failures are retried as transient outages.
    match run_cycle()? {
        RunOnceOutcome::Idle | RunOnceOutcome::CapacityUnavailable { .. } => {}
        outcome => println!("{}", serde_json::to_string(&outcome)?),
    }

    loop {
        match run_cycle() {
            Ok(RunOnceOutcome::Idle | RunOnceOutcome::CapacityUnavailable { .. }) => {
                last_error = None;
                error_backoff = idle_interval;
                thread::sleep(idle_interval);
            }
            Ok(outcome) => {
                last_error = None;
                error_backoff = idle_interval;
                println!("{}", serde_json::to_string(&outcome)?);
                thread::sleep(idle_interval);
            }
            Err(error) => {
                let message = error.to_string();
                if last_error.as_deref() != Some(&message) {
                    eprintln!("runner cycle failed: {message}");
                    last_error = Some(message);
                }
                thread::sleep(error_backoff);
                error_backoff = error_backoff
                    .checked_mul(2)
                    .unwrap_or(max_error_backoff)
                    .min(max_error_backoff);
            }
        }
    }
}

struct RunOnceConfig {
    runner_id: String,
    lease_seconds: i64,
    runtime_ready_timeout: Duration,
    runtime_ready_interval: Duration,
    finite_private_base_url: String,
    finite_private_model: String,
    finite_private_api_key_override: Option<String>,
    runtime_environment: BTreeMap<String, String>,
    runtime_secret_environment: BTreeMap<String, String>,
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
    .with_runtime_environment(config.runtime_environment)?
    .with_runtime_secret_environment(config.runtime_secret_environment)?
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

fn optional_runtime_environment() -> Result<BTreeMap<String, String>> {
    let Some(raw) = optional_env_value("FC_RUNNER_RUNTIME_ENV_JSON") else {
        return Ok(BTreeMap::new());
    };
    serde_json::from_str(&raw)
        .context("FC_RUNNER_RUNTIME_ENV_JSON must be a JSON object of string values")
}

fn optional_runtime_secret_environment() -> Result<BTreeMap<String, String>> {
    let Some(path) = optional_env_value("FC_RUNNER_RUNTIME_SECRET_ENV_FILE") else {
        return Ok(BTreeMap::new());
    };
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read FC_RUNNER_RUNTIME_SECRET_ENV_FILE {path}"))?;
    parse_runtime_secret_environment(&contents)
        .with_context(|| format!("invalid FC_RUNNER_RUNTIME_SECRET_ENV_FILE {path}"))
}

fn parse_runtime_secret_environment(contents: &str) -> Result<BTreeMap<String, String>> {
    let mut environment = BTreeMap::new();
    for (index, raw_line) in contents.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (raw_key, raw_value) = line
            .split_once('=')
            .with_context(|| format!("line {} must be KEY=VALUE", index + 1))?;
        let key = raw_key.trim();
        let value = raw_value.trim();
        if key.is_empty() || value.is_empty() {
            bail!("line {} must contain a non-empty key and value", index + 1);
        }
        if environment
            .insert(key.to_string(), value.to_string())
            .is_some()
        {
            bail!("line {} repeats {key}", index + 1);
        }
    }
    Ok(environment)
}

fn optional_env_value(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn immutable_oci_descriptor_digest(reference: &str) -> Result<String> {
    let Some((repository, hex)) = reference.trim().rsplit_once("@sha256:") else {
        bail!("Core runtime artifact reference is not immutable: {reference}");
    };
    if repository.is_empty() || hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("Core runtime artifact reference has an invalid sha256 digest: {reference}");
    }
    Ok(format!("sha256:{hex}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_secret_file_parser_is_literal_and_rejects_duplicates() {
        let parsed = parse_runtime_secret_environment(
            "# host-owned runtime secrets\nFAL_KEY=fal_test\nXAI_API_KEY=xai=value\n",
        )
        .unwrap();
        assert_eq!(parsed.get("FAL_KEY").map(String::as_str), Some("fal_test"));
        assert_eq!(
            parsed.get("XAI_API_KEY").map(String::as_str),
            Some("xai=value")
        );
        assert!(parse_runtime_secret_environment("FAL_KEY=one\nFAL_KEY=two\n").is_err());
        assert!(parse_runtime_secret_environment("not-an-assignment\n").is_err());
    }

    #[test]
    fn phala_preflight_is_an_explicit_subcommand() {
        let args = Args::try_parse_from(["finite-saas-runner", "phala-preflight"]).unwrap();
        assert!(matches!(args.command, Some(Command::PhalaPreflight)));
    }

    #[test]
    fn local_image_override_digest_is_derived_from_immutable_core_reference() {
        let hex = "a".repeat(64);
        assert_eq!(
            immutable_oci_descriptor_digest(&format!("runtime:dev@sha256:{hex}")).unwrap(),
            format!("sha256:{hex}")
        );
        assert!(immutable_oci_descriptor_digest("runtime:dev").is_err());
        assert!(immutable_oci_descriptor_digest("runtime:dev@sha256:short").is_err());
    }
}
