//! Local development harness for finitecomputer-v2 (PRD Phase 4).
//!
//! Product rule: local orchestration logic lives in real, testable Rust, not
//! in shell scripts. The first slice is `finite-private-limiter-up`, which
//! runs the in-tree Finite Private limiter chained in front of the deployed
//! limiter so the local rung exercises real key provisioning:
//!
//! ```text
//! agent runtime -> local limiter (admission vs local Core, local metering)
//!               -> deployed limiter (prod admission with ONE operator key)
//!               -> confidential vLLM (glm-5-2)
//! ```
//!
//! The orchestration decisions (env/config assembly, upstream path handling,
//! listen address selection, readiness polling) are pure functions here so
//! they can be unit tested without network access.

use finite_private_limiter::LimiterConfig;
use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

/// Deployed Finite Private limiter, as agents address it. The domain keeps
/// the historical kimi-k2-6 name but the endpoint now serves `glm-5-2`
/// (see docs/service-dependencies.md, Finite Private Routing Debt).
pub const DEFAULT_DEPLOYED_LIMITER_BASE_URL: &str =
    "https://kimi-k2-6.finite.containers.tinfoil.dev/v1";
/// Model served by the deployed Finite Private endpoint.
pub const DEFAULT_FINITE_PRIVATE_MODEL: &str = "glm-5-2";
/// Default listen address for the chained local limiter.
pub const DEFAULT_LIMITER_LISTEN_ADDR: &str = "127.0.0.1:18002";
/// Environment variable holding the one operator-held real `fpk_...` key used
/// as the chained limiter's upstream credential. This is the only production
/// secret the local rung needs.
pub const UPSTREAM_KEY_ENV: &str = "FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LocalLimiterConfigError {
    #[error(
        "{UPSTREAM_KEY_ENV} is required.\n\n\
         The chained local limiter admits and meters requests against your local\n\
         Core, then forwards allowed requests to the deployed Finite Private\n\
         limiter, which only accepts real production keys. Set:\n\n\
         \x20 export {UPSTREAM_KEY_ENV}=fpk_...\n\n\
         using one operator-held key issued by production Core. This is the only\n\
         production secret the local rung needs; per-agent keys are provisioned\n\
         by your local Core.\n\n\
         Fallback without a real key: export\n\
         FC_LOCAL_CANARY_FINITE_PRIVATE_API_KEY=fpk_... to have the runner inject\n\
         that key directly into the agent (bypasses local provisioning)."
    )]
    MissingUpstreamKey,
    #[error("{0} must not be empty")]
    MissingField(&'static str),
    #[error("listen address {value:?} is invalid: {message}")]
    InvalidListenAddr { value: String, message: String },
}

/// Inputs for the chained limiter, in operator-facing form.
#[derive(Debug, Clone)]
pub struct ChainedLimiterInputs {
    /// Local Core base URL (`FC_CORE_URL` in the canary).
    pub core_url: String,
    /// Route-scoped Core usage credential (`FC_FINITE_PRIVATE_USAGE_API_TOKEN`).
    pub finite_private_usage_api_token: String,
    /// Deployed limiter base URL as agents address it, i.e. usually ending in
    /// `/v1` (see [`upstream_root_for_chain`] for why that suffix is removed).
    pub upstream_base_url: String,
    /// One real operator-held `fpk_...` key from [`UPSTREAM_KEY_ENV`].
    pub upstream_api_key: Option<String>,
    /// Dashboard URL surfaced in limit-denied errors.
    pub dashboard_url: String,
}

/// Assemble the in-tree limiter's config for the chained-local topology.
///
/// The limiter treats `UPSTREAM_BASE_URL` as a host root and appends the
/// incoming request path (`/v1/chat/completions`) verbatim, so the
/// agent-facing `.../v1` base URL must be reduced to its root here or every
/// upstream call would hit `/v1/v1/...`.
pub fn chained_limiter_config(
    inputs: &ChainedLimiterInputs,
) -> Result<LimiterConfig, LocalLimiterConfigError> {
    let upstream_api_key = inputs
        .upstream_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(LocalLimiterConfigError::MissingUpstreamKey)?;
    let core_url = require_field("core url", &inputs.core_url)?;
    let finite_private_usage_api_token = require_field(
        "finite private usage api token",
        &inputs.finite_private_usage_api_token,
    )?;
    let upstream_base_url = require_field("upstream base url", &inputs.upstream_base_url)?;
    let dashboard_url = require_field("dashboard url", &inputs.dashboard_url)?;
    Ok(LimiterConfig::new(
        core_url.trim_end_matches('/').to_string(),
        finite_private_usage_api_token.to_string(),
        upstream_root_for_chain(upstream_base_url),
        upstream_api_key.to_string(),
        dashboard_url.to_string(),
    ))
}

fn require_field<'a>(
    name: &'static str,
    value: &'a str,
) -> Result<&'a str, LocalLimiterConfigError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(LocalLimiterConfigError::MissingField(name));
    }
    Ok(trimmed)
}

/// Reduce an agent-facing limiter base URL (usually `https://host/v1`) to the
/// upstream root the limiter proxies against. The limiter forwards the full
/// incoming path-and-query (already `/v1/...`) onto this value.
pub fn upstream_root_for_chain(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    trimmed
        .strip_suffix("/v1")
        .unwrap_or(trimmed)
        .trim_end_matches('/')
        .to_string()
}

/// Parse the chained limiter listen address. Port `0` is allowed and means
/// "pick a free port"; callers report the bound address after binding.
pub fn parse_listen_addr(value: &str) -> Result<SocketAddr, LocalLimiterConfigError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(LocalLimiterConfigError::MissingField("listen address"));
    }
    trimmed
        .parse::<SocketAddr>()
        .map_err(|error| LocalLimiterConfigError::InvalidListenAddr {
            value: value.to_string(),
            message: error.to_string(),
        })
}

/// Host clients on this machine should use to reach a bound listen address
/// (unspecified addresses such as `0.0.0.0` are reachable via loopback).
pub fn client_host(addr: &SocketAddr) -> String {
    if addr.ip().is_unspecified() {
        "127.0.0.1".to_string()
    } else {
        addr.ip().to_string()
    }
}

/// Deep-readiness URL for the chained limiter. `/live` is process-only;
/// local startup must also prove both upstream dependencies through `/health`.
pub fn health_url(addr: &SocketAddr) -> String {
    format!("http://{}:{}/health", client_host(addr), addr.port())
}

/// OpenAI-compatible base URL agents and the runner should point at
/// (`FC_RUNNER_FINITE_PRIVATE_BASE_URL` / `FINITE_PRIVATE_BASE_URL`).
pub fn agent_base_url(host: &str, port: u16) -> String {
    format!("http://{host}:{port}/v1")
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("not ready after {attempts} attempts over {timeout:?}")]
pub struct ReadinessTimeout {
    pub attempts: u32,
    pub timeout: Duration,
}

/// Poll `probe` until it reports ready or `timeout` elapses. Returns the
/// number of attempts made on success. The probe is injected so the loop is
/// unit-testable without a network.
pub async fn wait_until_ready<Probe, Fut>(
    mut probe: Probe,
    timeout: Duration,
    interval: Duration,
) -> Result<u32, ReadinessTimeout>
where
    Probe: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let started = tokio::time::Instant::now();
    let mut attempts = 0;
    loop {
        attempts += 1;
        if probe().await {
            return Ok(attempts);
        }
        if started.elapsed() >= timeout {
            return Err(ReadinessTimeout { attempts, timeout });
        }
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> ChainedLimiterInputs {
        ChainedLimiterInputs {
            core_url: "http://127.0.0.1:14200/".to_string(),
            finite_private_usage_api_token: "local-usage-token".to_string(),
            upstream_base_url: DEFAULT_DEPLOYED_LIMITER_BASE_URL.to_string(),
            upstream_api_key: Some("fpk_live_operator".to_string()),
            dashboard_url: "http://127.0.0.1:13002/dashboard".to_string(),
        }
    }

    #[test]
    fn chained_config_targets_local_core_and_deployed_limiter_root() {
        let config = chained_limiter_config(&inputs()).unwrap();

        assert_eq!(config.finite_usage_api_url, "http://127.0.0.1:14200");
        assert_eq!(config.finite_usage_api_service_key, "local-usage-token");
        // The /v1 suffix must be stripped: the limiter appends the incoming
        // /v1/... path onto UPSTREAM_BASE_URL.
        assert_eq!(
            config.upstream_base_url,
            "https://kimi-k2-6.finite.containers.tinfoil.dev"
        );
        assert_eq!(config.vllm_internal_api_key, "fpk_live_operator");
        assert_eq!(config.dashboard_url, "http://127.0.0.1:13002/dashboard");
    }

    #[test]
    fn chained_config_requires_the_operator_upstream_key() {
        for upstream_api_key in [None, Some(String::new()), Some("   ".to_string())] {
            let error = chained_limiter_config(&ChainedLimiterInputs {
                upstream_api_key,
                ..inputs()
            })
            .unwrap_err();
            assert_eq!(error, LocalLimiterConfigError::MissingUpstreamKey);
            let message = error.to_string();
            assert!(message.contains("FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY"));
            assert!(message.contains("FC_LOCAL_CANARY_FINITE_PRIVATE_API_KEY"));
        }
    }

    #[test]
    fn chained_config_rejects_missing_fields() {
        let error = chained_limiter_config(&ChainedLimiterInputs {
            finite_private_usage_api_token: " ".to_string(),
            ..inputs()
        })
        .unwrap_err();
        assert_eq!(
            error,
            LocalLimiterConfigError::MissingField("finite private usage api token")
        );
    }

    #[test]
    fn upstream_root_handles_v1_suffixes_and_slashes() {
        for (input, expected) in [
            ("https://limiter.example/v1", "https://limiter.example"),
            ("https://limiter.example/v1/", "https://limiter.example"),
            ("https://limiter.example", "https://limiter.example"),
            ("https://limiter.example/", "https://limiter.example"),
            (" http://127.0.0.1:9999/v1 ", "http://127.0.0.1:9999"),
            (
                "https://limiter.example/nested/v1",
                "https://limiter.example/nested",
            ),
        ] {
            assert_eq!(upstream_root_for_chain(input), expected, "input {input:?}");
        }
    }

    #[test]
    fn listen_addr_parses_and_allows_ephemeral_port_zero() {
        assert_eq!(
            parse_listen_addr(DEFAULT_LIMITER_LISTEN_ADDR).unwrap(),
            "127.0.0.1:18002".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(parse_listen_addr("0.0.0.0:0").unwrap().port(), 0);
        assert!(matches!(
            parse_listen_addr("not-an-addr"),
            Err(LocalLimiterConfigError::InvalidListenAddr { .. })
        ));
        assert_eq!(
            parse_listen_addr("  "),
            Err(LocalLimiterConfigError::MissingField("listen address"))
        );
    }

    #[test]
    fn client_urls_map_unspecified_hosts_to_loopback() {
        let bound: SocketAddr = "0.0.0.0:18002".parse().unwrap();
        assert_eq!(health_url(&bound), "http://127.0.0.1:18002/health");
        let explicit: SocketAddr = "192.168.1.20:9000".parse().unwrap();
        assert_eq!(health_url(&explicit), "http://192.168.1.20:9000/health");
        assert_eq!(
            agent_base_url("host.docker.internal", 18002),
            "http://host.docker.internal:18002/v1"
        );
    }

    #[tokio::test]
    async fn wait_until_ready_counts_attempts_and_times_out() {
        let attempts = wait_until_ready(
            {
                let mut remaining_failures = 2;
                move || {
                    let ready = remaining_failures == 0;
                    if !ready {
                        remaining_failures -= 1;
                    }
                    async move { ready }
                }
            },
            Duration::from_secs(5),
            Duration::from_millis(1),
        )
        .await
        .unwrap();
        assert_eq!(attempts, 3);

        let error = wait_until_ready(
            || async { false },
            Duration::from_millis(5),
            Duration::from_millis(1),
        )
        .await
        .unwrap_err();
        assert!(error.attempts >= 1);
        assert_eq!(error.timeout, Duration::from_millis(5));
    }
}
