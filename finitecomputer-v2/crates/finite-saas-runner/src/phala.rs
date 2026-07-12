//! Narrow Phala Cloud HTTPS API adapter.
//!
//! This module intentionally contains no CLI fallback and no provider delete
//! primitive. Provision and update commits accept only opaque material from the
//! not-yet-integrated, officially reviewed environment-encryption helper. That
//! keeps this client usable for inventory and ordinary lifecycle work without
//! inventing a cryptographic envelope or staging plaintext environment files.

use super::{
    DEFAULT_DOCKER_CONTAINER_PORT, DEFAULT_FINITE_AGENT_PICTURE_URL, DEFAULT_FINITECHAT_SERVER_URL,
    DEFAULT_RUNTIME_READY_INTERVAL, DEFAULT_RUNTIME_READY_TIMEOUT, DockerEquivalentRuntimeEnv,
    RunnerError, RuntimeLaunchFacts, RuntimeLaunchOptions, RuntimeLauncher, RuntimeRestartOptions,
    RuntimeUpgradeFacts, docker_equivalent_runtime_env, wait_for_http_json_ready,
};
use finite_saas_core::{
    AgentCreationLease, ProviderRuntimeHandleEnvelope, ProviderRuntimeHandleV1, RunnerClass,
    RunnerLeaseCapacity, RuntimeArtifactKind, RuntimeControlLease,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::io::Read;
use std::sync::Mutex;
use std::thread;
use std::time::Instant;
use std::time::{Duration, SystemTime};

pub const API_BASE_URL: &str = "https://cloud-api.phala.com/api/v1";
pub const API_VERSION: &str = "2026-06-23";
pub const FINITE_INSTANCE_TYPE: &str = "tdx.medium";
pub const FINITE_INSTANCE_VCPU: u32 = 2;
pub const FINITE_INSTANCE_MEMORY_MB: u64 = 4096;
pub const FINITE_DISK_SIZE_GB: u32 = 40;
pub const FINITE_HOURLY_PRICE_USD_MICROS: u64 = 116_000;

const CLOUD_KMS: &str = "PHALA";
const FINITE_CVM_NAME_PREFIX: &str = "finite-agent-";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_COMPOSE_BYTES: usize = 200 * 1024;
const MAX_PROVIDER_ID_BYTES: usize = 256;
const MAX_INVENTORY_PAGES: u32 = 1000;
const INVENTORY_PAGE_SIZE: u32 = 100;
const USER_AGENT: &str = concat!("finite-saas-runner/", env!("CARGO_PKG_VERSION"));

#[derive(Clone)]
pub struct PhalaConfig {
    pub api_key: String,
    pub source_host_id: String,
    pub image: String,
    pub runtime_artifact_id: Option<String>,
    pub runtime_artifact_kind: Option<RuntimeArtifactKind>,
    pub runtime_state_schema_version: Option<String>,
    pub finitechat_server_url: String,
    pub agent_picture_url: String,
    pub max_cvm_count: Option<u32>,
    pub drain_new_leases: bool,
    pub available_memory_bytes: Option<u64>,
    pub readiness_timeout: Duration,
    pub readiness_interval: Duration,
}

impl fmt::Debug for PhalaConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PhalaConfig")
            .field("api_key", &"<redacted>")
            .field("source_host_id", &self.source_host_id)
            .field("image", &self.image)
            .field("runtime_artifact_id", &self.runtime_artifact_id)
            .field("runtime_artifact_kind", &self.runtime_artifact_kind)
            .field(
                "runtime_state_schema_version",
                &self.runtime_state_schema_version,
            )
            .field("finitechat_server_url", &self.finitechat_server_url)
            .field("agent_picture_url", &self.agent_picture_url)
            .field("max_cvm_count", &self.max_cvm_count)
            .field("drain_new_leases", &self.drain_new_leases)
            .field("available_memory_bytes", &self.available_memory_bytes)
            .field("readiness_timeout", &self.readiness_timeout)
            .field("readiness_interval", &self.readiness_interval)
            .finish()
    }
}

impl PhalaConfig {
    pub fn validate(&self) -> Result<(), RunnerError> {
        if self.api_key.trim().is_empty() {
            return Err(RunnerError::MissingPhalaApiKey);
        }
        if self.source_host_id.trim().is_empty() {
            return Err(RunnerError::MissingSourceHostId);
        }
        validate_digest_pinned_image(&self.image)?;
        if self.finitechat_server_url.trim().is_empty() {
            return Err(RunnerError::MissingFinitechatServerUrl);
        }
        if let Some(kind) = self.runtime_artifact_kind
            && kind != RuntimeArtifactKind::OciImage
        {
            return Err(RunnerError::RuntimeLaunch(format!(
                "Phala launcher requires an OCI image artifact, got {}",
                kind.as_str()
            )));
        }
        if self.max_cvm_count == Some(0) {
            return Err(RunnerError::RuntimeLaunch(
                "Phala maximum CVM count must be at least one".to_string(),
            ));
        }
        if self.readiness_timeout.is_zero() || self.readiness_interval.is_zero() {
            return Err(RunnerError::RuntimeLaunch(
                "Phala readiness timeouts must be positive".to_string(),
            ));
        }
        Ok(())
    }
}

impl Default for PhalaConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            source_host_id: String::new(),
            image: String::new(),
            runtime_artifact_id: None,
            runtime_artifact_kind: Some(RuntimeArtifactKind::OciImage),
            runtime_state_schema_version: None,
            finitechat_server_url: DEFAULT_FINITECHAT_SERVER_URL.to_string(),
            agent_picture_url: DEFAULT_FINITE_AGENT_PICTURE_URL.to_string(),
            max_cvm_count: Some(1),
            drain_new_leases: false,
            available_memory_bytes: None,
            readiness_timeout: DEFAULT_RUNTIME_READY_TIMEOUT,
            readiness_interval: DEFAULT_RUNTIME_READY_INTERVAL,
        }
    }
}

fn validate_digest_pinned_image(image: &str) -> Result<(), RunnerError> {
    let Some((repository, digest)) = image.trim().rsplit_once("@sha256:") else {
        return Err(RunnerError::RuntimeLaunch(
            "Phala runtime image must be pinned by sha256 digest".to_string(),
        ));
    };
    if repository.is_empty()
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(RunnerError::RuntimeLaunch(
            "Phala runtime image must use an exact sha256 digest".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(tag = "schema", content = "handle")]
enum PhalaRuntimeHandleEnvelope {
    #[serde(rename = "phala_runtime_handle.v1")]
    V1(PhalaRuntimeHandleV1),
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PhalaRuntimeHandleV1 {
    cvm_id: String,
    app_id: String,
}

impl PhalaRuntimeHandleV1 {
    fn validate(&self) -> Result<(), RunnerError> {
        validate_provider_id(&self.cvm_id).map_err(runner_api_error)?;
        validate_provider_id(&self.app_id).map_err(runner_api_error)
    }

    #[cfg(test)]
    fn fixture() -> Self {
        Self {
            cvm_id: "cvm_fixture_01".to_string(),
            app_id: "app_fixture_01".to_string(),
        }
    }

    #[cfg(test)]
    fn core_envelope(&self) -> ProviderRuntimeHandleEnvelope {
        ProviderRuntimeHandleEnvelope::V1(ProviderRuntimeHandleV1 {
            runner_class: RunnerClass::Phala,
            opaque: serde_json::to_value(PhalaRuntimeHandleEnvelope::V1(self.clone())).unwrap(),
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct PreflightSnapshot {
    provider_healthy: bool,
    active_cvm_count: u32,
}

type HealthCheck = fn(&str, &str, Duration, Duration) -> Result<(), RunnerError>;

pub struct PhalaLauncher {
    config: PhalaConfig,
    client: Result<PhalaApiClient, String>,
    preflight: Mutex<PreflightSnapshot>,
    health_check: HealthCheck,
}

impl fmt::Debug for PhalaLauncher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PhalaLauncher")
            .field("config", &self.config)
            .field("client", &self.client)
            .field("preflight", &self.preflight)
            .finish_non_exhaustive()
    }
}

impl PhalaLauncher {
    pub fn new(config: PhalaConfig) -> Self {
        let client = PhalaApiClient::new(config.api_key.clone()).map_err(|error| error.to_string());
        Self {
            config,
            client,
            preflight: Mutex::new(PreflightSnapshot::default()),
            health_check: wait_for_http_json_ready,
        }
    }

    #[cfg(test)]
    fn with_client(config: PhalaConfig, client: PhalaApiClient) -> Self {
        Self {
            config,
            client: Ok(client),
            preflight: Mutex::new(PreflightSnapshot::default()),
            health_check: no_op_health_check,
        }
    }

    fn client(&self) -> Result<&PhalaApiClient, RunnerError> {
        self.client
            .as_ref()
            .map_err(|error| RunnerError::RuntimeLaunch(error.clone()))
    }

    fn refresh_preflight(&self) -> Result<(), RunnerError> {
        let result = (|| {
            let client = self.client()?;
            client.preflight_summary().map_err(runner_api_error)
        })();
        let mut snapshot = self.preflight.lock().map_err(|_| {
            RunnerError::RuntimeLaunch("Phala preflight state lock was poisoned".to_string())
        })?;
        match result {
            Ok(summary) => {
                *snapshot = PreflightSnapshot {
                    provider_healthy: true,
                    active_cvm_count: summary.active_finite_cvm_count,
                };
            }
            Err(error) => {
                *snapshot = PreflightSnapshot::default();
                eprintln!("Phala preflight blocked new creation: {error}");
            }
        }
        Ok(())
    }

    fn runtime_handle(
        &self,
        lease: &RuntimeControlLease,
    ) -> Result<PhalaRuntimeHandleV1, RunnerError> {
        if lease.runtime.source_host_id != self.config.source_host_id {
            return Err(RunnerError::RuntimeLaunch(format!(
                "runtime belongs to source host {}, not {}",
                lease.runtime.source_host_id, self.config.source_host_id
            )));
        }
        let outer = lease
            .runtime
            .provider_runtime_handle
            .as_ref()
            .ok_or_else(|| {
                RunnerError::RuntimeLaunch(
                    "Phala runtime is missing its persisted provider handle".to_string(),
                )
            })?;
        if outer.runner_class() != RunnerClass::Phala {
            return Err(RunnerError::RuntimeLaunch(
                "provider handle does not belong to the Phala runner".to_string(),
            ));
        }
        let ProviderRuntimeHandleEnvelope::V1(ProviderRuntimeHandleV1 { opaque, .. }) = outer;
        let handle: PhalaRuntimeHandleEnvelope =
            serde_json::from_value(opaque.clone()).map_err(|_| {
                RunnerError::RuntimeLaunch(
                    "Phala runtime provider handle was invalid or unsupported".to_string(),
                )
            })?;
        let PhalaRuntimeHandleEnvelope::V1(handle) = handle;
        handle.validate()?;
        Ok(handle)
    }

    fn inspect_verified(&self, handle: &PhalaRuntimeHandleV1) -> Result<CvmInfo, RunnerError> {
        let cvm = self
            .client()?
            .inspect_cvm(&handle.cvm_id)
            .map_err(runner_api_error)?;
        cvm.verify_finite_runtime().map_err(runner_api_error)?;
        if !cvm.is_finite_active() || cvm.app_id.as_deref() != Some(handle.app_id.as_str()) {
            return Err(RunnerError::RuntimeLaunch(
                "Phala CVM did not match its persisted provider handle".to_string(),
            ));
        }
        Ok(cvm)
    }

    fn wait_for_running(&self, handle: &PhalaRuntimeHandleV1) -> Result<CvmInfo, RunnerError> {
        let started = Instant::now();
        loop {
            let cvm = self.inspect_verified(handle)?;
            if status_is_running(&cvm.status) {
                return Ok(cvm);
            }
            if started.elapsed() >= self.config.readiness_timeout {
                return Err(RunnerError::RuntimeLaunch(
                    "Phala CVM did not become running before the readiness deadline".to_string(),
                ));
            }
            thread::sleep(self.config.readiness_interval);
        }
    }

    fn wait_for_stopped(&self, handle: &PhalaRuntimeHandleV1) -> Result<(), RunnerError> {
        let started = Instant::now();
        loop {
            let cvm = self.inspect_verified(handle)?;
            if status_is_stopped(&cvm.status) {
                return Ok(());
            }
            if started.elapsed() >= self.config.readiness_timeout {
                return Err(RunnerError::RuntimeLaunch(
                    "Phala CVM did not stop before the readiness deadline".to_string(),
                ));
            }
            thread::sleep(self.config.readiness_interval);
        }
    }

    fn check_runtime_health(&self, cvm: &CvmInfo) -> Result<(), RunnerError> {
        let base_url = cvm
            .public_application_endpoint()
            .map_err(runner_api_error)?
            .trim_end_matches('/');
        (self.health_check)(
            &format!("{base_url}/healthz"),
            "Phala runtime /healthz",
            self.config.readiness_timeout,
            self.config.readiness_interval,
        )
    }
}

impl RuntimeLauncher for PhalaLauncher {
    fn validate_ready(&self) -> Result<(), RunnerError> {
        self.config.validate()?;
        self.client()?;
        self.refresh_preflight()
    }

    fn runner_class(&self) -> RunnerClass {
        RunnerClass::Phala
    }

    fn uses_core_runtime_heartbeat(&self) -> bool {
        false
    }

    fn runner_capacity(&self) -> RunnerLeaseCapacity {
        let snapshot = self
            .preflight
            .lock()
            .map(|snapshot| *snapshot)
            .unwrap_or_default();
        RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::Phala],
            draining: self.config.drain_new_leases || !snapshot.provider_healthy,
            max_sandbox_count: self.config.max_cvm_count,
            active_sandbox_count: Some(snapshot.active_cvm_count),
            available_memory_bytes: self.config.available_memory_bytes,
        }
    }

    fn source_host_id(&self) -> Option<&str> {
        Some(&self.config.source_host_id)
    }

    fn restart_runtime(
        &mut self,
        lease: &RuntimeControlLease,
        _options: &RuntimeRestartOptions,
    ) -> Result<(), RunnerError> {
        self.config.validate()?;
        let handle = self.runtime_handle(lease)?;
        let cvm = self.inspect_verified(&handle)?;
        if status_is_stopped(&cvm.status) {
            self.client()?
                .start_cvm(&handle.cvm_id)
                .map_err(runner_api_error)?;
        } else {
            self.client()?
                .restart_cvm(&handle.cvm_id)
                .map_err(runner_api_error)?;
        }
        let cvm = self.wait_for_running(&handle)?;
        self.check_runtime_health(&cvm)
    }

    fn recover_known_good_chat_runtime(
        &mut self,
        lease: &RuntimeControlLease,
        options: &RuntimeRestartOptions,
    ) -> Result<(), RunnerError> {
        self.restart_runtime(lease, options)
    }

    fn upgrade_runtime(
        &mut self,
        _lease: &RuntimeControlLease,
        _options: &RuntimeRestartOptions,
    ) -> Result<RuntimeUpgradeFacts, RunnerError> {
        Err(RunnerError::RuntimeLaunch(
            "Phala upgrade is disabled until reviewed encrypted-environment handling and durable Core acknowledgment are wired"
                .to_string(),
        ))
    }

    fn stop_runtime(&mut self, lease: &RuntimeControlLease) -> Result<(), RunnerError> {
        self.config.validate()?;
        let handle = self.runtime_handle(lease)?;
        let cvm = self.inspect_verified(&handle)?;
        if status_is_stopped(&cvm.status) {
            return Ok(());
        }
        self.client()?
            .shutdown_cvm(&handle.cvm_id)
            .map_err(runner_api_error)?;
        self.wait_for_stopped(&handle)
    }

    fn destroy_runtime(&mut self, _lease: &RuntimeControlLease) -> Result<(), RunnerError> {
        Err(RunnerError::RuntimeLaunch(
            "Phala destroy is intentionally unsupported by this runner".to_string(),
        ))
    }

    fn launch(
        &mut self,
        lease: &AgentCreationLease,
        options: &RuntimeLaunchOptions,
    ) -> Result<RuntimeLaunchFacts, RunnerError> {
        self.config.validate()?;
        // Render and validate the reviewed compose contract in memory only.
        // No provider provision call or plaintext environment staging is
        // allowed on this side of the encryption-and-acknowledgment boundary.
        let _compose = phala_compose(&self.config, lease, options)?;
        Err(RunnerError::RuntimeLaunch(
            "Phala creation is disabled until reviewed encrypted-environment handling and durable Core acknowledgment are wired"
                .to_string(),
        ))
    }
}

fn runner_api_error(error: PhalaApiError) -> RunnerError {
    RunnerError::RuntimeLaunch(error.to_string())
}

fn status_is_running(status: &str) -> bool {
    status.trim().eq_ignore_ascii_case("running")
}

fn status_is_stopped(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "stopped" | "shutdown" | "exited" | "powered_off"
    )
}

fn phala_cvm_name_for_request_id(request_id: &str) -> String {
    let suffix = request_id
        .strip_prefix("agent_request_")
        .unwrap_or(request_id);
    let mut result = String::from("finite-agent-");
    for ch in suffix.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
        } else if !result.ends_with('-') {
            result.push('-');
        }
    }
    if result.len() > 63 {
        result.truncate(63);
    }
    result.trim_end_matches('-').to_string()
}

fn phala_compose(
    config: &PhalaConfig,
    lease: &AgentCreationLease,
    options: &RuntimeLaunchOptions,
) -> Result<String, RunnerError> {
    config.validate()?;
    let cvm_name = phala_cvm_name_for_request_id(&lease.request.id);
    let mut environment = docker_equivalent_runtime_env(
        DockerEquivalentRuntimeEnv {
            finitechat_server_url: &config.finitechat_server_url,
            agent_picture_url: &config.agent_picture_url,
            agent_http_port: DEFAULT_DOCKER_CONTAINER_PORT,
            agent_device_id: &cvm_name,
            agent_home: "/data/agent",
            hermes_home: "/data/agent/hermes-home",
            workspace: "/data/workspace",
        },
        lease,
        options,
    );
    for (key, value) in &mut environment {
        if matches!(key.as_str(), "FINITE_PRIVATE_API_KEY" | "OPENAI_API_KEY") {
            *value = "${FINITE_PRIVATE_API_KEY:?FINITE_PRIVATE_API_KEY is required}".to_string();
        } else if options.secret_environment.contains_key(key) {
            *value = format!("${{{key}:?{key} is required}}");
        }
    }

    let mut rendered = String::new();
    rendered.push_str("services:\n  agent:\n    image: ");
    rendered.push_str(&yaml_quote(config.image.trim()));
    rendered.push_str("\n    platform: linux/amd64\n    container_name: ");
    rendered.push_str(&yaml_quote(&cvm_name));
    rendered.push_str(
        "\n    restart: unless-stopped\n    ports:\n      - \"8080:8080\"\n    volumes:\n      - agent_state:/data\n      - /var/run/dstack.sock:/var/run/dstack.sock\n    environment:\n",
    );
    for (key, value) in environment {
        rendered.push_str("      ");
        rendered.push_str(&key);
        rendered.push_str(": ");
        rendered.push_str(&yaml_quote(&value));
        rendered.push('\n');
    }
    rendered.push_str("\nvolumes:\n  agent_state:\n");
    Ok(rendered)
}

fn yaml_quote(value: &str) -> String {
    let mut quoted = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

#[cfg(test)]
fn no_op_health_check(
    _url: &str,
    _name: &str,
    _timeout: Duration,
    _interval: Duration,
) -> Result<(), RunnerError> {
    Ok(())
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    max_retries: u8,
    base_delay: Duration,
    max_delay: Duration,
}

impl RetryPolicy {
    pub fn new(
        max_retries: u8,
        base_delay: Duration,
        max_delay: Duration,
    ) -> Result<Self, PhalaApiError> {
        if max_retries > 8 {
            return Err(PhalaApiError::Configuration(
                "Phala retry count must not exceed 8",
            ));
        }
        if base_delay > max_delay || max_delay > Duration::from_secs(60) {
            return Err(PhalaApiError::Configuration(
                "Phala retry delays must be ordered and bounded to 60 seconds",
            ));
        }
        Ok(Self {
            max_retries,
            base_delay,
            max_delay,
        })
    }

    fn delay_for(&self, attempt: u8, retry_after: Option<Duration>) -> Duration {
        if let Some(delay) = retry_after {
            return delay.min(self.max_delay);
        }
        let multiplier = 1_u32.checked_shl(u32::from(attempt)).unwrap_or(u32::MAX);
        self.base_delay
            .checked_mul(multiplier)
            .unwrap_or(self.max_delay)
            .min(self.max_delay)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PhalaApiConfig {
    base_url: String,
    connect_timeout: Duration,
    read_timeout: Duration,
    write_timeout: Duration,
    retry_policy: RetryPolicy,
}

impl Default for PhalaApiConfig {
    fn default() -> Self {
        Self {
            base_url: API_BASE_URL.to_string(),
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(20),
            write_timeout: Duration::from_secs(20),
            retry_policy: RetryPolicy::default(),
        }
    }
}

impl PhalaApiConfig {
    pub fn with_timeouts(
        mut self,
        connect_timeout: Duration,
        read_timeout: Duration,
        write_timeout: Duration,
    ) -> Result<Self, PhalaApiError> {
        for timeout in [connect_timeout, read_timeout, write_timeout] {
            if timeout.is_zero() || timeout > Duration::from_secs(60) {
                return Err(PhalaApiError::Configuration(
                    "Phala HTTP timeouts must be between 1ns and 60 seconds",
                ));
            }
        }
        self.connect_timeout = connect_timeout;
        self.read_timeout = read_timeout;
        self.write_timeout = write_timeout;
        Ok(self)
    }

    pub fn with_retry_policy(mut self, retry_policy: RetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }

    #[cfg(test)]
    fn for_fake_server(base_url: String) -> Self {
        Self {
            base_url,
            connect_timeout: Duration::from_secs(1),
            read_timeout: Duration::from_secs(1),
            write_timeout: Duration::from_secs(1),
            retry_policy: RetryPolicy {
                max_retries: 2,
                base_delay: Duration::ZERO,
                max_delay: Duration::ZERO,
            },
        }
    }

    fn validate(&self) -> Result<(), PhalaApiError> {
        let is_official = self.base_url == API_BASE_URL;
        let is_test = cfg!(test) && self.base_url.starts_with("http://127.0.0.1:");
        if !is_official && !is_test {
            return Err(PhalaApiError::Configuration(
                "Phala API base URL must be the pinned official endpoint",
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct PhalaApiClient {
    base_url: String,
    api_key: String,
    agent: ureq::Agent,
    retry_policy: RetryPolicy,
}

impl fmt::Debug for PhalaApiClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PhalaApiClient")
            .field("base_url", &self.base_url)
            .field("api_version", &API_VERSION)
            .field("api_key", &"<redacted>")
            .field("retry_policy", &self.retry_policy)
            .finish_non_exhaustive()
    }
}

impl PhalaApiClient {
    pub fn new(api_key: impl Into<String>) -> Result<Self, PhalaApiError> {
        Self::with_config(api_key, PhalaApiConfig::default())
    }

    pub fn with_config(
        api_key: impl Into<String>,
        config: PhalaApiConfig,
    ) -> Result<Self, PhalaApiError> {
        config.validate()?;
        let api_key = api_key.into().trim().to_string();
        if api_key.is_empty() {
            return Err(PhalaApiError::Configuration("Phala API key is required"));
        }
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(config.connect_timeout)
            .timeout_read(config.read_timeout)
            .timeout_write(config.write_timeout)
            .redirects(0)
            .build();
        Ok(Self {
            base_url: config.base_url.trim_end_matches('/').to_string(),
            api_key,
            agent,
            retry_policy: config.retry_policy,
        })
    }

    pub fn list_cpu_instance_types(&self) -> Result<InstanceTypeCatalog, PhalaApiError> {
        self.get_json("list_cpu_instance_types", "/instance-types/cpu")
    }

    pub fn verify_finite_instance_type(&self) -> Result<VerifiedFiniteInstanceType, PhalaApiError> {
        self.list_cpu_instance_types()?
            .verify_finite_instance_type()
    }

    pub fn capacity(&self) -> Result<AvailableCapacity, PhalaApiError> {
        self.get_json("capacity", "/teepods/available")
    }

    /// Authenticated read-only compatibility check used by both worker
    /// startup and the opt-in CI preflight command. The returned projection
    /// deliberately contains no provider identifiers, endpoint URLs, or
    /// credential material.
    pub fn preflight_summary(&self) -> Result<PhalaPreflightSummary, PhalaApiError> {
        let verified = self.verify_finite_instance_type()?;
        let capacity = self.capacity()?;
        let inventory = self.inventory()?;
        let available_node_count = capacity
            .nodes
            .iter()
            .filter(|node| {
                node.listed
                    && node.remaining_vcpu >= f64::from(FINITE_INSTANCE_VCPU)
                    && node.remaining_memory >= FINITE_INSTANCE_MEMORY_MB as f64
                    && node.remaining_cvm_slots >= 1.0
            })
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        Ok(PhalaPreflightSummary {
            api_version: API_VERSION,
            instance_type: FINITE_INSTANCE_TYPE,
            vcpu: FINITE_INSTANCE_VCPU,
            memory_mb: FINITE_INSTANCE_MEMORY_MB,
            disk_size_gb: FINITE_DISK_SIZE_GB,
            hourly_price_usd_micros: verified.hourly_rate.usd_micros(),
            provider_reported_max_instances: capacity.capacity.max_instances,
            available_node_count,
            active_finite_cvm_count: inventory.active_cvm_count(),
        })
    }

    pub fn inspect_cvm(&self, cvm_id: &str) -> Result<CvmInfo, PhalaApiError> {
        let path = cvm_path(cvm_id, None)?;
        self.get_json("inspect_cvm", &path)
    }

    pub fn inventory(&self) -> Result<FiniteInventory, PhalaApiError> {
        let mut page = 1;
        let mut cvms = Vec::new();
        loop {
            if page > MAX_INVENTORY_PAGES {
                return Err(PhalaApiError::Contract(
                    "Phala inventory exceeded the bounded page limit",
                ));
            }
            let path = format!("/cvms/paginated?page={page}&page_size={INVENTORY_PAGE_SIZE}");
            let response: Paginated<CvmInfo> = self.get_json("inventory", &path)?;
            cvms.extend(response.items.into_iter().filter(CvmInfo::is_finite_active));
            if response.pages == 0 || page >= response.pages {
                break;
            }
            page += 1;
        }
        Ok(FiniteInventory { cvms })
    }

    pub fn start_cvm(&self, cvm_id: &str) -> Result<CvmAction, PhalaApiError> {
        let path = cvm_path(cvm_id, Some("start"))?;
        self.post_json("start_cvm", &path, None::<&serde_json::Value>)
    }

    pub fn shutdown_cvm(&self, cvm_id: &str) -> Result<CvmAction, PhalaApiError> {
        let path = cvm_path(cvm_id, Some("shutdown"))?;
        self.post_json("shutdown_cvm", &path, None::<&serde_json::Value>)
    }

    pub fn restart_cvm(&self, cvm_id: &str) -> Result<CvmAction, PhalaApiError> {
        let path = cvm_path(cvm_id, Some("restart"))?;
        self.post_json(
            "restart_cvm",
            &path,
            Some(&RestartCvmRequest { force: false }),
        )
    }

    pub fn provision_cvm(
        &self,
        request: &ProvisionCvmRequest,
    ) -> Result<UnverifiedProvision, PhalaApiError> {
        self.post_json("provision_cvm", "/cvms/provision", Some(request))
    }

    /// Commit is deliberately gated on both Core-persisted identifiers and an
    /// envelope produced by a future reviewed official-helper integration.
    pub fn commit_provision(
        &self,
        provision: &PersistedProvision,
        environment: &VerifiedEncryptedEnvironment,
    ) -> Result<CvmAction, PhalaApiError> {
        let request = CommitProvisionRequest {
            app_id: &provision.app_id,
            compose_hash: &provision.compose_hash,
            encrypted_env: environment.ciphertext(),
            env_keys: environment.keys(),
        };
        self.post_json("commit_provision", "/cvms", Some(&request))
    }

    pub fn provision_update(
        &self,
        cvm_id: &str,
        request: &ProvisionUpdateRequest,
    ) -> Result<UnverifiedProvision, PhalaApiError> {
        let path = cvm_path(cvm_id, Some("compose_file/provision"))?;
        self.post_json("provision_update", &path, Some(request))
    }

    /// Update commit has the same reviewed-helper and durable-persistence
    /// boundary as initial creation. It never accepts plaintext environment.
    pub fn commit_update(
        &self,
        update: &PersistedUpdate,
        environment: &VerifiedEncryptedEnvironment,
    ) -> Result<(), PhalaApiError> {
        let path = cvm_path(&update.cvm_id, Some("compose_file"))?;
        let request = CommitUpdateRequest {
            compose_hash: &update.compose_hash,
            encrypted_env: environment.ciphertext(),
            env_keys: environment.keys(),
            update_env_vars: true,
        };
        self.patch_json_empty("commit_update", &path, &request)
    }

    fn get_json<T: DeserializeOwned>(
        &self,
        operation: &'static str,
        path: &str,
    ) -> Result<T, PhalaApiError> {
        let response = self.request(operation, "GET", path, None, RequestKind::ReadOnly)?;
        decode_json(operation, response)
    }

    fn post_json<T, B>(
        &self,
        operation: &'static str,
        path: &str,
        body: Option<&B>,
    ) -> Result<T, PhalaApiError>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        let body = encode_body(operation, body)?;
        let response = self.request(
            operation,
            "POST",
            path,
            body.as_deref(),
            RequestKind::Mutation,
        )?;
        decode_json(operation, response)
    }

    fn patch_json_empty<B: Serialize + ?Sized>(
        &self,
        operation: &'static str,
        path: &str,
        body: &B,
    ) -> Result<(), PhalaApiError> {
        let body = encode_body(operation, Some(body))?;
        let response = self.request(
            operation,
            "PATCH",
            path,
            body.as_deref(),
            RequestKind::Mutation,
        )?;
        let _ = read_bounded(operation, response)?;
        Ok(())
    }

    fn request(
        &self,
        operation: &'static str,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        request_kind: RequestKind,
    ) -> Result<ureq::Response, PhalaApiError> {
        debug_assert!(path.starts_with('/'));
        let url = format!("{}{}", self.base_url, path);
        let mut attempt = 0_u8;
        loop {
            let mut request = self
                .agent
                .request(method, &url)
                .set("Accept", "application/json")
                .set("X-API-Key", &self.api_key)
                .set("X-Phala-Version", API_VERSION)
                .set("User-Agent", USER_AGENT);
            if body.is_some() {
                request = request.set("Content-Type", "application/json");
            }
            let result = match body {
                Some(body) => request.send_bytes(body),
                None => request.call(),
            };
            match result {
                Ok(response) => return Ok(response),
                Err(ureq::Error::Status(_, response)) => {
                    let error = decode_status_error(operation, response)?;
                    if error.is_retryable() && attempt < self.retry_policy.max_retries {
                        let delay = self.retry_policy.delay_for(attempt, error.retry_after());
                        if !delay.is_zero() {
                            std::thread::sleep(delay);
                        }
                        attempt += 1;
                        continue;
                    }
                    return Err(error);
                }
                Err(ureq::Error::Transport(_)) if request_kind == RequestKind::Mutation => {
                    return Err(PhalaApiError::AmbiguousMutation { operation });
                }
                Err(ureq::Error::Transport(_)) => {
                    return Err(PhalaApiError::Transport { operation });
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PhalaPreflightSummary {
    pub api_version: &'static str,
    pub instance_type: &'static str,
    pub vcpu: u32,
    pub memory_mb: u64,
    pub disk_size_gb: u32,
    pub hourly_price_usd_micros: u64,
    pub provider_reported_max_instances: Option<u32>,
    pub available_node_count: u32,
    pub active_finite_cvm_count: u32,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum RequestKind {
    ReadOnly,
    Mutation,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum PhalaApiError {
    #[error("Phala client configuration is invalid: {0}")]
    Configuration(&'static str),
    #[error("Phala {operation} transport failed")]
    Transport { operation: &'static str },
    #[error(
        "Phala {operation} outcome is unknown; inspect provider state before retrying the mutation"
    )]
    AmbiguousMutation { operation: &'static str },
    #[error("Phala {operation} returned HTTP {status}{details}")]
    Status {
        operation: &'static str,
        status: u16,
        details: Box<StatusDetails>,
    },
    #[error("Phala {operation} returned an invalid or oversized response")]
    InvalidResponse { operation: &'static str },
    #[error("Phala API contract violation: {0}")]
    Contract(&'static str),
}

impl PhalaApiError {
    pub fn status_details(&self) -> Option<(u16, &StatusDetails)> {
        match self {
            Self::Status {
                status, details, ..
            } => Some((*status, details)),
            _ => None,
        }
    }

    fn is_retryable(&self) -> bool {
        match self {
            Self::Status {
                status: 429 | 503, ..
            } => true,
            Self::Status {
                status: 409,
                details,
                ..
            } => details.error_code.is_none(),
            _ => false,
        }
    }

    fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Status { details, .. } => details.retry_after,
            _ => None,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct StatusDetails {
    pub request_id: Option<String>,
    pub error_code: Option<String>,
    pub retry_after: Option<Duration>,
}

impl fmt::Display for StatusDetails {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(request_id) = &self.request_id {
            write!(formatter, " (request {request_id})")?;
        }
        if let Some(code) = &self.error_code {
            write!(formatter, " [{code}]")?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    error_code: Option<String>,
}

fn decode_status_error(
    operation: &'static str,
    response: ureq::Response,
) -> Result<PhalaApiError, PhalaApiError> {
    let status = response.status();
    let header_request_id = response
        .header("X-Request-ID")
        .and_then(safe_error_identifier);
    let retry_after = response
        .header("Retry-After")
        .and_then(|value| parse_retry_after(value, SystemTime::now()));
    let body = read_bounded(operation, response)?;
    let parsed: Option<ErrorBody> = serde_json::from_slice(&body).ok();
    let request_id = parsed
        .as_ref()
        .and_then(|body| body.request_id.as_deref())
        .and_then(safe_error_identifier)
        .or(header_request_id);
    let error_code = parsed
        .and_then(|body| body.error_code)
        .as_deref()
        .and_then(safe_error_identifier);
    Ok(PhalaApiError::Status {
        operation,
        status,
        details: Box::new(StatusDetails {
            request_id,
            error_code,
            retry_after,
        }),
    })
}

fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let retry_at = httpdate::parse_http_date(value).ok()?;
    retry_at.duration_since(now).ok()
}

fn safe_error_identifier(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return None;
    }
    Some(value.to_string())
}

fn encode_body<B: Serialize + ?Sized>(
    operation: &'static str,
    body: Option<&B>,
) -> Result<Option<Vec<u8>>, PhalaApiError> {
    body.map(serde_json::to_vec)
        .transpose()
        .map_err(|_| PhalaApiError::InvalidResponse { operation })
}

fn decode_json<T: DeserializeOwned>(
    operation: &'static str,
    response: ureq::Response,
) -> Result<T, PhalaApiError> {
    let body = read_bounded(operation, response)?;
    serde_json::from_slice(&body).map_err(|_| PhalaApiError::InvalidResponse { operation })
}

fn read_bounded(
    operation: &'static str,
    response: ureq::Response,
) -> Result<Vec<u8>, PhalaApiError> {
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| PhalaApiError::InvalidResponse { operation })?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(PhalaApiError::InvalidResponse { operation });
    }
    Ok(bytes)
}

fn cvm_path(cvm_id: &str, suffix: Option<&str>) -> Result<String, PhalaApiError> {
    validate_provider_id(cvm_id)?;
    Ok(match suffix {
        Some(suffix) => format!("/cvms/{cvm_id}/{suffix}"),
        None => format!("/cvms/{cvm_id}"),
    })
}

fn validate_provider_id(value: &str) -> Result<(), PhalaApiError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_PROVIDER_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(PhalaApiError::Contract("invalid Phala provider id"));
    }
    Ok(())
}

fn validate_cvm_name(value: &str) -> Result<(), PhalaApiError> {
    if !(5..=63).contains(&value.len())
        || !value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(PhalaApiError::Contract(
            "invalid Phala CVM correlation name",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct InstanceTypeCatalog {
    pub items: Vec<InstanceType>,
    pub total: u32,
    pub family: String,
}

impl InstanceTypeCatalog {
    pub fn verify_finite_instance_type(&self) -> Result<VerifiedFiniteInstanceType, PhalaApiError> {
        let instance = self
            .items
            .iter()
            .find(|instance| instance.id == FINITE_INSTANCE_TYPE)
            .ok_or(PhalaApiError::Contract(
                "Phala catalog did not contain tdx.medium",
            ))?;
        if instance.vcpu != FINITE_INSTANCE_VCPU
            || instance.memory_mb != FINITE_INSTANCE_MEMORY_MB
            || instance.hourly_rate.usd_micros() != FINITE_HOURLY_PRICE_USD_MICROS
            || instance.requires_gpu
        {
            return Err(PhalaApiError::Contract(
                "Phala tdx.medium shape or live price changed",
            ));
        }
        Ok(VerifiedFiniteInstanceType {
            hourly_rate: instance.hourly_rate.clone(),
            default_disk_size_gb: instance.default_disk_size_gb,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct InstanceType {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub vcpu: u32,
    pub memory_mb: u64,
    pub hourly_rate: HourlyUsd,
    pub requires_gpu: bool,
    pub default_disk_size_gb: u32,
    pub family: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct VerifiedFiniteInstanceType {
    pub hourly_rate: HourlyUsd,
    pub default_disk_size_gb: u32,
}

#[derive(Clone, Eq, PartialEq)]
pub struct HourlyUsd {
    source: String,
    usd_micros: u64,
}

impl HourlyUsd {
    pub fn usd_micros(&self) -> u64 {
        self.usd_micros
    }
}

impl fmt::Debug for HourlyUsd {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("HourlyUsd")
            .field(&self.source)
            .finish()
    }
}

impl Serialize for HourlyUsd {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.source)
    }
}

impl<'de> Deserialize<'de> for HourlyUsd {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let source = String::deserialize(deserializer)?;
        parse_usd_micros(&source)
            .map(|usd_micros| Self { source, usd_micros })
            .ok_or_else(|| serde::de::Error::custom("invalid hourly USD decimal"))
    }
}

fn parse_usd_micros(value: &str) -> Option<u64> {
    let value = value.strip_prefix('$').unwrap_or(value);
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 6
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let whole = whole.parse::<u64>().ok()?.checked_mul(1_000_000)?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u64>().ok()? * 10_u64.pow(6 - fraction.len() as u32)
    };
    whole.checked_add(fraction)
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AvailableCapacity {
    pub tier: String,
    pub capacity: ResourceThreshold,
    pub nodes: Vec<NodeCapacity>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct ResourceThreshold {
    pub max_instances: Option<u32>,
    pub max_vcpu: Option<u32>,
    pub max_memory: Option<u64>,
    pub max_disk: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct NodeCapacity {
    pub teepod_id: u64,
    pub name: String,
    pub listed: bool,
    pub remaining_vcpu: f64,
    pub remaining_memory: f64,
    pub remaining_cvm_slots: f64,
    pub region_identifier: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct Paginated<T> {
    pub items: Vec<T>,
    pub total: u32,
    pub page: u32,
    pub page_size: u32,
    pub pages: u32,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct CvmInfo {
    pub id: String,
    pub name: String,
    pub app_id: Option<String>,
    pub vm_uuid: Option<String>,
    pub status: String,
    pub resource: CvmResource,
    #[serde(default)]
    pub endpoints: Vec<CvmEndpoint>,
    pub public_logs: Option<bool>,
    pub deleted_at: Option<String>,
    pub compose_hash: Option<String>,
}

impl CvmInfo {
    pub fn is_finite_active(&self) -> bool {
        self.name.starts_with(FINITE_CVM_NAME_PREFIX)
            && self.deleted_at.is_none()
            && !matches!(
                self.status.trim().to_ascii_lowercase().as_str(),
                "deleted" | "terminated"
            )
    }

    pub fn verify_finite_runtime(&self) -> Result<(), PhalaApiError> {
        if self.resource.instance_type.as_deref() != Some(FINITE_INSTANCE_TYPE)
            || self.resource.vcpu != Some(FINITE_INSTANCE_VCPU)
            || self.resource.memory_in_gb != Some(4.0)
            || self.resource.disk_in_gb != Some(FINITE_DISK_SIZE_GB)
            || self.public_logs != Some(false)
        {
            return Err(PhalaApiError::Contract(
                "Phala CVM shape, disk, or private-log policy did not match",
            ));
        }
        Ok(())
    }

    pub fn public_application_endpoint(&self) -> Result<&str, PhalaApiError> {
        let mut endpoints = self
            .endpoints
            .iter()
            .map(|endpoint| endpoint.app.trim())
            .filter(|url| !url.is_empty());
        let endpoint = endpoints.next().ok_or(PhalaApiError::Contract(
            "Phala CVM did not expose an application endpoint",
        ))?;
        if endpoints.next().is_some() || !endpoint.starts_with("https://") {
            return Err(PhalaApiError::Contract(
                "Phala CVM did not expose exactly one HTTPS application endpoint",
            ));
        }
        Ok(endpoint)
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct CvmResource {
    pub instance_type: Option<String>,
    pub vcpu: Option<u32>,
    pub memory_in_gb: Option<f64>,
    pub disk_in_gb: Option<u32>,
    pub compute_billing_price: Option<String>,
    pub billing_period: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct CvmEndpoint {
    pub app: String,
    #[serde(default)]
    pub instance: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FiniteInventory {
    pub cvms: Vec<CvmInfo>,
}

impl FiniteInventory {
    pub fn active_cvm_count(&self) -> u32 {
        self.cvms.len().try_into().unwrap_or(u32::MAX)
    }
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct CvmAction {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Serialize)]
struct RestartCvmRequest {
    force: bool,
}

#[derive(Clone, Serialize)]
pub struct ProvisionCvmRequest {
    name: String,
    instance_type: &'static str,
    disk_size: u32,
    kms: &'static str,
    listed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    region: Option<String>,
    compose_file: PrivateComposeFile,
}

impl fmt::Debug for ProvisionCvmRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProvisionCvmRequest")
            .field("name", &self.name)
            .field("instance_type", &self.instance_type)
            .field("disk_size", &self.disk_size)
            .field("kms", &self.kms)
            .field("listed", &self.listed)
            .field("region", &self.region)
            .field("compose_file", &"<compose redacted>")
            .finish()
    }
}

impl ProvisionCvmRequest {
    pub fn finite_private(
        correlation_name: impl Into<String>,
        docker_compose_file: impl Into<String>,
    ) -> Result<Self, PhalaApiError> {
        let name = correlation_name.into();
        validate_cvm_name(&name)?;
        let docker_compose_file = docker_compose_file.into();
        if docker_compose_file.is_empty() || docker_compose_file.len() > MAX_COMPOSE_BYTES {
            return Err(PhalaApiError::Contract(
                "Phala compose must be nonempty and at most 200 KiB",
            ));
        }
        Ok(Self {
            compose_file: PrivateComposeFile {
                name: name.clone(),
                docker_compose_file,
                gateway_enabled: true,
                public_logs: false,
                public_sysinfo: false,
            },
            name,
            instance_type: FINITE_INSTANCE_TYPE,
            disk_size: FINITE_DISK_SIZE_GB,
            kms: CLOUD_KMS,
            listed: false,
            region: None,
        })
    }

    pub fn with_region(mut self, region: impl Into<String>) -> Result<Self, PhalaApiError> {
        let region = region.into();
        if region.is_empty()
            || region.len() > 64
            || !region
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(PhalaApiError::Contract("invalid Phala region"));
        }
        self.region = Some(region);
        Ok(self)
    }
}

#[derive(Debug, Clone, Serialize)]
struct PrivateComposeFile {
    name: String,
    docker_compose_file: String,
    gateway_enabled: bool,
    public_logs: bool,
    public_sysinfo: bool,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct UnverifiedProvision {
    pub app_id: String,
    pub compose_hash: String,
    pub app_env_encrypt_pubkey: String,
    pub kms_info: Option<KmsInfo>,
    pub instance_type: Option<String>,
    pub node_id: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct KmsInfo {
    pub encrypted_env_pubkey: Option<String>,
    pub k256_pubkey: Option<String>,
    pub dstack_kms_address: Option<String>,
    pub dstack_app_address: Option<String>,
}

/// Core-acknowledged provision identifiers. Construction remains crate-private
/// until the durable handle write is wired into the generic Runner contract.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PersistedProvision {
    app_id: String,
    compose_hash: String,
}

impl PersistedProvision {
    #[cfg(test)]
    fn fixture(app_id: &str, compose_hash: &str) -> Self {
        Self {
            app_id: app_id.to_string(),
            compose_hash: compose_hash.to_string(),
        }
    }
}

/// Ciphertext created only by the future reviewed official helper. It has no
/// public constructor and deliberately contains no plaintext environment API.
#[derive(Clone, Eq, PartialEq)]
pub struct VerifiedEncryptedEnvironment {
    ciphertext: String,
    keys: Vec<String>,
}

impl fmt::Debug for VerifiedEncryptedEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedEncryptedEnvironment")
            .field("ciphertext", &"<redacted>")
            .field("keys", &self.keys)
            .finish()
    }
}

impl VerifiedEncryptedEnvironment {
    fn ciphertext(&self) -> &str {
        &self.ciphertext
    }

    fn keys(&self) -> &[String] {
        &self.keys
    }

    #[cfg(test)]
    fn fixture(ciphertext: &str, keys: &[&str]) -> Self {
        Self {
            ciphertext: ciphertext.to_string(),
            keys: keys.iter().map(|key| (*key).to_string()).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
struct CommitProvisionRequest<'a> {
    app_id: &'a str,
    compose_hash: &'a str,
    encrypted_env: &'a str,
    env_keys: &'a [String],
}

#[derive(Clone, Serialize)]
pub struct ProvisionUpdateRequest {
    name: String,
    docker_compose_file: String,
    encrypted_env: String,
    allowed_envs: Vec<String>,
    gateway_enabled: bool,
    public_logs: bool,
    public_sysinfo: bool,
    update_env_vars: bool,
}

impl fmt::Debug for ProvisionUpdateRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProvisionUpdateRequest")
            .field("name", &self.name)
            .field("docker_compose_file", &"<compose redacted>")
            .field("encrypted_env", &"<redacted>")
            .field("allowed_envs", &self.allowed_envs)
            .field("public_logs", &self.public_logs)
            .field("update_env_vars", &self.update_env_vars)
            .finish_non_exhaustive()
    }
}

impl ProvisionUpdateRequest {
    pub fn finite_private(
        name: impl Into<String>,
        docker_compose_file: impl Into<String>,
        environment: &VerifiedEncryptedEnvironment,
    ) -> Result<Self, PhalaApiError> {
        let name = name.into();
        validate_cvm_name(&name)?;
        let docker_compose_file = docker_compose_file.into();
        if docker_compose_file.is_empty() || docker_compose_file.len() > MAX_COMPOSE_BYTES {
            return Err(PhalaApiError::Contract(
                "Phala compose must be nonempty and at most 200 KiB",
            ));
        }
        Ok(Self {
            name,
            docker_compose_file,
            encrypted_env: environment.ciphertext.clone(),
            allowed_envs: environment.keys.clone(),
            gateway_enabled: true,
            public_logs: false,
            public_sysinfo: false,
            update_env_vars: true,
        })
    }
}

/// Core-acknowledged update identifiers, gated exactly like initial provision.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PersistedUpdate {
    cvm_id: String,
    compose_hash: String,
}

impl PersistedUpdate {
    #[cfg(test)]
    fn fixture(cvm_id: &str, compose_hash: &str) -> Self {
        Self {
            cvm_id: cvm_id.to_string(),
            compose_hash: compose_hash.to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
struct CommitUpdateRequest<'a> {
    compose_hash: &'a str,
    encrypted_env: &'a str,
    env_keys: &'a [String],
    update_env_vars: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FinitePrivateLaunchKey;
    use finite_saas_core::{
        AgentCreationRequest, AgentCreationRequestStatus, AgentRuntime, HostOwnedRuntimeFacts,
        Project, RuntimeControlKind, RuntimeControlRequest, RuntimeControlRequestStatus,
        RuntimeSummaryStatus,
    };
    use std::collections::{BTreeMap, VecDeque};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;

    const FIXTURE_API_KEY: &str = "fixture-api-key-not-a-secret";
    const FIXTURE_CVM_ID: &str = "cvm_fixture_01";

    #[derive(Debug)]
    struct CapturedRequest {
        method: String,
        path: String,
        headers: BTreeMap<String, String>,
        body: Vec<u8>,
    }

    #[derive(Debug)]
    enum FixtureResponse {
        Http {
            status: u16,
            headers: Vec<(&'static str, &'static str)>,
            body: &'static str,
        },
        Close,
    }

    struct FakePhalaServer {
        base_url: String,
        requests: Arc<Mutex<Vec<CapturedRequest>>>,
        stop: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl FakePhalaServer {
        fn start(responses: Vec<FixtureResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let requests_thread = requests.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let stop_thread = stop.clone();
            let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
            let responses_thread = responses.clone();
            let thread = thread::spawn(move || {
                while !stop_thread.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            if let Some(request) = read_request(&mut stream) {
                                requests_thread.lock().unwrap().push(request);
                            }
                            match responses_thread.lock().unwrap().pop_front() {
                                Some(FixtureResponse::Http {
                                    status,
                                    headers,
                                    body,
                                }) => write_response(&mut stream, status, &headers, body),
                                Some(FixtureResponse::Close) | None => {}
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                base_url: format!("http://{address}/api/v1"),
                requests,
                stop,
                thread: Some(thread),
            }
        }

        fn client(&self) -> PhalaApiClient {
            PhalaApiClient::with_config(
                FIXTURE_API_KEY,
                PhalaApiConfig::for_fake_server(self.base_url.clone()),
            )
            .unwrap()
        }

        fn requests(&self) -> Vec<CapturedRequest> {
            std::mem::take(&mut *self.requests.lock().unwrap())
        }
    }

    impl Drop for FakePhalaServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(address) = self.base_url.strip_prefix("http://") {
                let address = address.split('/').next().unwrap();
                let _ = TcpStream::connect(address);
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn read_request(stream: &mut TcpStream) -> Option<CapturedRequest> {
        stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        let header_end = loop {
            let read = stream.read(&mut chunk).ok()?;
            if read == 0 {
                return None;
            }
            bytes.extend_from_slice(&chunk[..read]);
            if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers_text = String::from_utf8_lossy(&bytes[..header_end]);
        let mut lines = headers_text.split("\r\n");
        let request_line = lines.next()?.split_whitespace().collect::<Vec<_>>();
        if request_line.len() < 2 {
            return None;
        }
        let method = request_line[0].to_string();
        let path = request_line[1].to_string();
        let mut headers = BTreeMap::new();
        for line in lines.filter(|line| !line.is_empty()) {
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
            }
        }
        let content_length = headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        while bytes.len() < header_end + content_length {
            let read = stream.read(&mut chunk).ok()?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        Some(CapturedRequest {
            method,
            path,
            headers,
            body: bytes[header_end..].to_vec(),
        })
    }

    fn write_response(stream: &mut TcpStream, status: u16, headers: &[(&str, &str)], body: &str) {
        let reason = match status {
            200 => "OK",
            201 => "Created",
            202 => "Accepted",
            204 => "No Content",
            409 => "Conflict",
            429 => "Too Many Requests",
            503 => "Service Unavailable",
            _ => "Fixture",
        };
        let mut response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in headers {
            response.push_str(&format!("{name}: {value}\r\n"));
        }
        response.push_str("\r\n");
        response.push_str(body);
        stream.write_all(response.as_bytes()).unwrap();
    }

    fn json_response(body: &'static str) -> FixtureResponse {
        FixtureResponse::Http {
            status: 200,
            headers: vec![],
            body,
        }
    }

    fn launcher_config() -> PhalaConfig {
        PhalaConfig {
            api_key: FIXTURE_API_KEY.to_string(),
            source_host_id: "phala-worker-1".to_string(),
            image: format!(
                "ghcr.io/finitecomputer/finite-agent-runtime@sha256:{}",
                "a".repeat(64)
            ),
            runtime_artifact_id: Some("artifact-phala-1".to_string()),
            runtime_state_schema_version: Some("runtime-state-v1".to_string()),
            readiness_timeout: Duration::from_millis(100),
            readiness_interval: Duration::from_millis(1),
            ..PhalaConfig::default()
        }
    }

    fn sample_creation_lease() -> AgentCreationLease {
        AgentCreationLease {
            project: Project {
                id: "project_123".to_string(),
                customer_org_id: "org_123".to_string(),
                owner_user_id: "user_123".to_string(),
                display_name: "Fixture Agent".to_string(),
                import_candidate_id: None,
                hosting_tier: None,
                placement: None,
                created_at: "2026-07-01T00:00:00Z".to_string(),
                updated_at: "2026-07-01T00:00:00Z".to_string(),
            },
            request: AgentCreationRequest {
                id: "agent_request_Fixture.01".to_string(),
                customer_org_id: "org_123".to_string(),
                owner_user_id: "user_123".to_string(),
                project_id: "project_123".to_string(),
                idempotency_key: "fixture-idempotency".to_string(),
                display_name: "Fixture Agent".to_string(),
                runner_class: RunnerClass::Phala,
                hosting_tier: None,
                placement: None,
                desired_runtime_artifact_id: None,
                runtime_spec: None,
                profile_picture_url: None,
                status: AgentCreationRequestStatus::Launching,
                requested_launch_code: None,
                agent_runtime_id: None,
                runner_id: Some("runner-phala".to_string()),
                lease_token: Some("fixture-lease".to_string()),
                lease_expires_at: None,
                failure_message: None,
                created_at: "2026-07-01T00:00:00Z".to_string(),
                updated_at: "2026-07-01T00:00:00Z".to_string(),
            },
        }
    }

    fn sample_control_lease(
        kind: RuntimeControlKind,
        handle: Option<ProviderRuntimeHandleEnvelope>,
    ) -> RuntimeControlLease {
        RuntimeControlLease {
            request: RuntimeControlRequest {
                id: "runtime_control_fixture".to_string(),
                project_id: "project_123".to_string(),
                agent_runtime_id: "runtime_123".to_string(),
                source_host_id: "phala-worker-1".to_string(),
                source_machine_id: "legacy-request-machine-id".to_string(),
                requested_by_user_id: "user_123".to_string(),
                kind,
                target_runtime_artifact_id: None,
                status: RuntimeControlRequestStatus::Running,
                runner_id: Some("runner-phala".to_string()),
                lease_token: Some("fixture-lease".to_string()),
                lease_expires_at: None,
                failure_message: None,
                created_at: "2026-07-01T00:00:00Z".to_string(),
                updated_at: "2026-07-01T00:00:00Z".to_string(),
                completed_at: None,
            },
            runtime: AgentRuntime {
                id: "runtime_123".to_string(),
                project_id: "project_123".to_string(),
                source_host_id: "phala-worker-1".to_string(),
                source_machine_id: "legacy-runtime-machine-id".to_string(),
                source_import_key: "fixture-import-key".to_string(),
                runtime_artifact_id: Some("artifact-phala-1".to_string()),
                state_schema_version: Some("runtime-state-v1".to_string()),
                placement: None,
                provider_runtime_handle: handle,
                provider_runtime_handle_history: Vec::new(),
                contact_endpoint: None,
                host_facts: HostOwnedRuntimeFacts {
                    display_name: "Fixture Agent".to_string(),
                    hostname: None,
                    runtime_host: "fixture-runtime".to_string(),
                    runtime_status: RuntimeSummaryStatus::Online,
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                },
                created_at: "2026-07-01T00:00:00Z".to_string(),
                updated_at: "2026-07-01T00:00:00Z".to_string(),
            },
            target_runtime_artifact: None,
        }
    }

    #[test]
    fn launcher_compose_is_digest_pinned_private_and_never_contains_plaintext_secrets() {
        let config = launcher_config();
        let lease = sample_creation_lease();
        let options = RuntimeLaunchOptions {
            finite_private: Some(FinitePrivateLaunchKey {
                api_key_id: "fixture-key-id".to_string(),
                raw_api_key: "fixture-plaintext-inference-key".to_string(),
                base_url: "https://inference.example.invalid/v1".to_string(),
                model: "fixture-model".to_string(),
                revoke_on_launch_failure: true,
            }),
            profile_picture_url: None,
            environment: BTreeMap::new(),
            secret_environment: BTreeMap::from([(
                "FAL_KEY".to_string(),
                "fixture-plaintext-provider-key".to_string(),
            )]),
        };
        let compose = phala_compose(&config, &lease, &options).unwrap();

        assert!(compose.contains("platform: linux/amd64"));
        assert!(compose.contains("- agent_state:/data"));
        assert!(compose.contains("- /var/run/dstack.sock:/var/run/dstack.sock"));
        assert!(compose.contains("FINITECHAT_HOME: '/data/agent'"));
        assert!(compose.contains(
            "FINITE_PRIVATE_API_KEY: '${FINITE_PRIVATE_API_KEY:?FINITE_PRIVATE_API_KEY is required}'"
        ));
        assert!(compose.contains("FAL_KEY: '${FAL_KEY:?FAL_KEY is required}'"));
        assert!(!compose.contains("fixture-plaintext-inference-key"));
        assert!(!compose.contains("fixture-plaintext-provider-key"));
        assert!(!compose.contains(FIXTURE_API_KEY));
        assert_eq!(
            phala_cvm_name_for_request_id(&lease.request.id),
            "finite-agent-fixture-01"
        );

        let mut unpinned = config;
        unpinned.image = "ghcr.io/finitecomputer/runtime:latest".to_string();
        assert!(unpinned.validate().is_err());
    }

    #[test]
    fn launcher_preflight_pins_shape_price_capacity_and_inventory_count() {
        let server = FakePhalaServer::start(vec![
            json_response(include_str!(
                "../tests/fixtures/phala/instance-types-cpu.json"
            )),
            json_response(include_str!("../tests/fixtures/phala/available-nodes.json")),
            json_response(include_str!("../tests/fixtures/phala/cvm-list.json")),
        ]);
        let launcher = PhalaLauncher::with_client(launcher_config(), server.client());
        launcher.validate_ready().unwrap();
        let capacity = launcher.runner_capacity();
        assert!(!capacity.draining);
        assert_eq!(capacity.active_sandbox_count, Some(1));
        assert_eq!(capacity.max_sandbox_count, Some(1));
        assert!(!capacity.accepts_agent_creation());
        assert_eq!(
            server
                .requests()
                .iter()
                .map(|request| request.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "/api/v1/instance-types/cpu",
                "/api/v1/teepods/available",
                "/api/v1/cvms/paginated?page=1&page_size=100",
            ]
        );
    }

    #[test]
    fn provider_preflight_failure_blocks_creation_but_not_handle_based_restart() {
        let server = FakePhalaServer::start(vec![
            FixtureResponse::Http {
                status: 500,
                headers: vec![],
                body: r#"{"error_code":"FIXTURE_UNAVAILABLE"}"#,
            },
            json_response(include_str!("../tests/fixtures/phala/cvm-detail.json")),
            json_response(include_str!("../tests/fixtures/phala/action.json")),
            json_response(include_str!("../tests/fixtures/phala/cvm-detail.json")),
        ]);
        let mut launcher = PhalaLauncher::with_client(launcher_config(), server.client());

        launcher.validate_ready().unwrap();
        let capacity = launcher.runner_capacity();
        assert!(capacity.draining);
        assert_eq!(capacity.active_sandbox_count, Some(0));

        let lease = sample_control_lease(
            RuntimeControlKind::Restart,
            Some(PhalaRuntimeHandleV1::fixture().core_envelope()),
        );
        launcher
            .restart_runtime(&lease, &RuntimeRestartOptions::default())
            .unwrap();
        let requests = server.requests();
        assert_eq!(
            requests
                .iter()
                .map(|request| (request.method.as_str(), request.path.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("GET", "/api/v1/instance-types/cpu"),
                ("GET", "/api/v1/cvms/cvm_fixture_01"),
                ("POST", "/api/v1/cvms/cvm_fixture_01/restart"),
                ("GET", "/api/v1/cvms/cvm_fixture_01"),
            ]
        );
        assert!(requests.iter().all(|request| {
            !request.path.contains("legacy-runtime-machine-id")
                && !request.path.contains("legacy-request-machine-id")
        }));
    }

    #[test]
    fn launcher_starts_stopped_cvm_and_shutdown_waits_for_stopped_state() {
        let stopped = include_str!("../tests/fixtures/phala/cvm-stopped.json");
        let running = include_str!("../tests/fixtures/phala/cvm-detail.json");
        let action = include_str!("../tests/fixtures/phala/action.json");
        let handle = Some(PhalaRuntimeHandleV1::fixture().core_envelope());

        let start_server = FakePhalaServer::start(vec![
            json_response(stopped),
            json_response(action),
            json_response(running),
        ]);
        let mut launcher = PhalaLauncher::with_client(launcher_config(), start_server.client());
        launcher
            .restart_runtime(
                &sample_control_lease(RuntimeControlKind::Restart, handle.clone()),
                &RuntimeRestartOptions::default(),
            )
            .unwrap();
        assert_eq!(
            start_server.requests()[1].path,
            "/api/v1/cvms/cvm_fixture_01/start"
        );

        let stop_server = FakePhalaServer::start(vec![
            json_response(running),
            json_response(action),
            json_response(stopped),
        ]);
        let mut launcher = PhalaLauncher::with_client(launcher_config(), stop_server.client());
        launcher
            .stop_runtime(&sample_control_lease(RuntimeControlKind::Stop, handle))
            .unwrap();
        assert_eq!(
            stop_server.requests()[1].path,
            "/api/v1/cvms/cvm_fixture_01/shutdown"
        );
    }

    #[test]
    fn missing_or_wrong_provider_handle_fails_before_provider_request() {
        let server = FakePhalaServer::start(Vec::new());
        let mut launcher = PhalaLauncher::with_client(launcher_config(), server.client());
        let missing = sample_control_lease(RuntimeControlKind::Restart, None);
        assert!(
            launcher
                .restart_runtime(&missing, &RuntimeRestartOptions::default())
                .unwrap_err()
                .to_string()
                .contains("missing its persisted provider handle")
        );

        let wrong = ProviderRuntimeHandleEnvelope::V1(ProviderRuntimeHandleV1 {
            runner_class: RunnerClass::Kata,
            opaque: serde_json::json!({
                "schema": "phala_runtime_handle.v1",
                "handle": {"cvmId": "cvm_fixture_01", "appId": "app_fixture_01"}
            }),
        });
        assert!(
            launcher
                .stop_runtime(&sample_control_lease(RuntimeControlKind::Stop, Some(wrong)))
                .unwrap_err()
                .to_string()
                .contains("does not belong to the Phala runner")
        );
        assert!(server.requests().is_empty());
    }

    #[test]
    fn create_upgrade_and_destroy_are_fail_closed_without_provider_mutation() {
        let server = FakePhalaServer::start(Vec::new());
        let mut launcher = PhalaLauncher::with_client(launcher_config(), server.client());
        let creation = launcher
            .launch(&sample_creation_lease(), &RuntimeLaunchOptions::default())
            .unwrap_err();
        assert!(creation.to_string().contains("creation is disabled"));

        let control = sample_control_lease(
            RuntimeControlKind::Upgrade,
            Some(PhalaRuntimeHandleV1::fixture().core_envelope()),
        );
        assert!(
            launcher
                .upgrade_runtime(&control, &RuntimeRestartOptions::default())
                .unwrap_err()
                .to_string()
                .contains("upgrade is disabled")
        );
        assert!(launcher.destroy_runtime(&control).is_err());
        assert!(server.requests().is_empty());
    }

    #[test]
    fn client_pins_official_headers_and_redacts_api_key() {
        let server = FakePhalaServer::start(vec![json_response(include_str!(
            "../tests/fixtures/phala/instance-types-cpu.json"
        ))]);
        server.client().list_cpu_instance_types().unwrap();
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].path, "/api/v1/instance-types/cpu");
        assert_eq!(
            requests[0].headers.get("x-api-key").map(String::as_str),
            Some(FIXTURE_API_KEY)
        );
        assert_eq!(
            requests[0]
                .headers
                .get("x-phala-version")
                .map(String::as_str),
            Some(API_VERSION)
        );
        let debug = format!("{:?}", server.client());
        assert!(!debug.contains(FIXTURE_API_KEY));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn catalog_verifies_exact_medium_shape_and_decimal_price() {
        let catalog: InstanceTypeCatalog = serde_json::from_str(include_str!(
            "../tests/fixtures/phala/instance-types-cpu.json"
        ))
        .unwrap();
        let verified = catalog.verify_finite_instance_type().unwrap();
        assert_eq!(verified.hourly_rate.usd_micros(), 116_000);
        assert_eq!(verified.default_disk_size_gb, 40);

        let changed = include_str!("../tests/fixtures/phala/instance-types-cpu.json")
            .replace("0.116", "0.117");
        let changed: InstanceTypeCatalog = serde_json::from_str(&changed).unwrap();
        assert_eq!(
            changed.verify_finite_instance_type().unwrap_err(),
            PhalaApiError::Contract("Phala tdx.medium shape or live price changed")
        );
    }

    #[test]
    fn inventory_counts_only_active_finite_cvms_and_checks_capacity() {
        let server = FakePhalaServer::start(vec![
            json_response(include_str!("../tests/fixtures/phala/cvm-list.json")),
            json_response(include_str!("../tests/fixtures/phala/available-nodes.json")),
        ]);
        let client = server.client();
        let inventory = client.inventory().unwrap();
        assert_eq!(inventory.active_cvm_count(), 1);
        assert_eq!(inventory.cvms[0].id, FIXTURE_CVM_ID);
        let capacity = client.capacity().unwrap();
        assert_eq!(capacity.capacity.max_instances, Some(1));
        assert_eq!(capacity.nodes[0].remaining_cvm_slots, 1.0);
    }

    #[test]
    fn read_only_preflight_summary_contains_counts_but_no_provider_identity() {
        let server = FakePhalaServer::start(vec![
            json_response(include_str!(
                "../tests/fixtures/phala/instance-types-cpu.json"
            )),
            json_response(include_str!("../tests/fixtures/phala/available-nodes.json")),
            json_response(include_str!("../tests/fixtures/phala/cvm-list.json")),
        ]);
        let summary = server.client().preflight_summary().unwrap();
        assert_eq!(summary.api_version, API_VERSION);
        assert_eq!(summary.instance_type, FINITE_INSTANCE_TYPE);
        assert_eq!(summary.hourly_price_usd_micros, 116_000);
        assert_eq!(summary.available_node_count, 1);
        assert_eq!(summary.active_finite_cvm_count, 1);
        let output = serde_json::to_string(&summary).unwrap();
        for sensitive_or_identifying in [
            FIXTURE_API_KEY,
            "fixture-node",
            "fixture-region",
            "cvm_fixture_01",
            "app_fixture_01",
            "fixture-app.example.invalid",
        ] {
            assert!(!output.contains(sensitive_or_identifying));
        }
        assert!(
            server
                .requests()
                .iter()
                .all(|request| request.method == "GET")
        );
    }

    #[test]
    fn inspect_requires_exact_disk_private_logs_and_one_https_endpoint() {
        let server = FakePhalaServer::start(vec![json_response(include_str!(
            "../tests/fixtures/phala/cvm-detail.json"
        ))]);
        let cvm = server.client().inspect_cvm(FIXTURE_CVM_ID).unwrap();
        cvm.verify_finite_runtime().unwrap();
        assert_eq!(
            cvm.public_application_endpoint().unwrap(),
            "https://fixture-app.example.invalid"
        );
    }

    #[test]
    fn lifecycle_uses_start_shutdown_restart_and_never_force() {
        let action = include_str!("../tests/fixtures/phala/action.json");
        let server = FakePhalaServer::start(vec![
            json_response(action),
            json_response(action),
            json_response(action),
        ]);
        let client = server.client();
        client.start_cvm(FIXTURE_CVM_ID).unwrap();
        client.shutdown_cvm(FIXTURE_CVM_ID).unwrap();
        client.restart_cvm(FIXTURE_CVM_ID).unwrap();
        let requests = server.requests();
        assert_eq!(
            requests
                .iter()
                .map(|request| request.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "/api/v1/cvms/cvm_fixture_01/start",
                "/api/v1/cvms/cvm_fixture_01/shutdown",
                "/api/v1/cvms/cvm_fixture_01/restart",
            ]
        );
        let restart: serde_json::Value = serde_json::from_slice(&requests[2].body).unwrap();
        assert_eq!(restart, serde_json::json!({ "force": false }));
    }

    #[test]
    fn provision_is_private_medium_cloud_kms_and_contains_no_plaintext_env() {
        let server = FakePhalaServer::start(vec![json_response(include_str!(
            "../tests/fixtures/phala/provision.json"
        ))]);
        let request = ProvisionCvmRequest::finite_private(
            "finite-agent-fixture-01",
            "services:\n  agent:\n    image: example.invalid/finite@sha256:fixture",
        )
        .unwrap();
        let provision = server.client().provision_cvm(&request).unwrap();
        assert_eq!(provision.app_id, "app_fixture_01");
        let requests = server.requests();
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["instance_type"], "tdx.medium");
        assert_eq!(body["disk_size"], 40);
        assert_eq!(body["kms"], "PHALA");
        assert_eq!(body["listed"], false);
        assert_eq!(body["compose_file"]["public_logs"], false);
        assert_eq!(body["compose_file"]["public_sysinfo"], false);
        assert!(body.get("env").is_none());
        assert!(body["compose_file"].get("env").is_none());
        assert!(body["compose_file"].get("encrypted_env").is_none());
    }

    #[test]
    fn commit_and_update_accept_only_reviewed_envelope_boundary() {
        let server = FakePhalaServer::start(vec![
            json_response(include_str!("../tests/fixtures/phala/action.json")),
            json_response(include_str!("../tests/fixtures/phala/provision.json")),
            FixtureResponse::Http {
                status: 204,
                headers: vec![],
                body: "",
            },
        ]);
        let client = server.client();
        let environment = VerifiedEncryptedEnvironment::fixture(
            "fixture-encrypted-envelope",
            &["FAL_KEY", "FINITECHAT_SERVER_URL"],
        );
        client
            .commit_provision(
                &PersistedProvision::fixture("app_fixture_01", "compose_fixture_01"),
                &environment,
            )
            .unwrap();
        let update = ProvisionUpdateRequest::finite_private(
            "finite-agent-fixture-01",
            "services:\n  agent:\n    image: example.invalid/finite@sha256:fixture2",
            &environment,
        )
        .unwrap();
        client.provision_update(FIXTURE_CVM_ID, &update).unwrap();
        client
            .commit_update(
                &PersistedUpdate::fixture(FIXTURE_CVM_ID, "compose_fixture_02"),
                &environment,
            )
            .unwrap();

        let requests = server.requests();
        assert_eq!(requests[0].path, "/api/v1/cvms");
        assert_eq!(
            requests[1].path,
            "/api/v1/cvms/cvm_fixture_01/compose_file/provision"
        );
        assert_eq!(requests[2].path, "/api/v1/cvms/cvm_fixture_01/compose_file");
        for request in &requests {
            let body = String::from_utf8_lossy(&request.body);
            assert!(body.contains("fixture-encrypted-envelope"));
            assert!(!body.contains("plaintext-secret"));
            if request.path.contains("compose_file") {
                assert!(body.contains("\"public_logs\":false") || request.method == "PATCH");
            }
        }
        let debug = format!("{environment:?} {update:?}");
        assert!(!debug.contains("fixture-encrypted-envelope"));
    }

    #[test]
    fn retry_after_and_unstructured_transient_errors_retry() {
        for status in [409, 429, 503] {
            let server = FakePhalaServer::start(vec![
                FixtureResponse::Http {
                    status,
                    headers: vec![("Retry-After", "0")],
                    body: r#"{"message":"fixture transient"}"#,
                },
                json_response(include_str!(
                    "../tests/fixtures/phala/instance-types-cpu.json"
                )),
            ]);
            server.client().list_cpu_instance_types().unwrap();
            assert_eq!(server.requests().len(), 2, "status {status}");
        }
        assert_eq!(
            parse_retry_after("7", SystemTime::UNIX_EPOCH),
            Some(Duration::from_secs(7))
        );
        let date = httpdate::fmt_http_date(SystemTime::UNIX_EPOCH + Duration::from_secs(9));
        assert_eq!(
            parse_retry_after(&date, SystemTime::UNIX_EPOCH),
            Some(Duration::from_secs(9))
        );
    }

    #[test]
    fn structured_conflict_is_permanent_and_error_is_redacted() {
        let server = FakePhalaServer::start(vec![FixtureResponse::Http {
            status: 409,
            headers: vec![("X-Request-ID", "request_fixture_01")],
            body: include_str!("../tests/fixtures/phala/structured-error.json"),
        }]);
        let error = server.client().start_cvm(FIXTURE_CVM_ID).unwrap_err();
        assert_eq!(server.requests().len(), 1);
        let rendered = format!("{error:?} {error}");
        assert!(rendered.contains("request_fixture_01"));
        assert!(rendered.contains("ERR-FIXTURE-CONFLICT"));
        assert!(!rendered.contains(FIXTURE_API_KEY));
        assert!(!rendered.contains("fixture-sensitive-echo"));
    }

    #[test]
    fn ambiguous_mutation_stops_for_inspection_instead_of_repeating() {
        let server = FakePhalaServer::start(vec![FixtureResponse::Close]);
        let error = server.client().restart_cvm(FIXTURE_CVM_ID).unwrap_err();
        assert_eq!(
            error,
            PhalaApiError::AmbiguousMutation {
                operation: "restart_cvm"
            }
        );
        assert_eq!(server.requests().len(), 1);
    }

    #[test]
    fn provider_ids_cannot_escape_the_typed_path() {
        let client = PhalaApiClient::new("fixture").unwrap();
        assert_eq!(
            client.inspect_cvm("../cvms/other").unwrap_err(),
            PhalaApiError::Contract("invalid Phala provider id")
        );
    }

    #[test]
    fn retry_policy_bounds_server_retry_after() {
        let policy =
            RetryPolicy::new(2, Duration::from_millis(10), Duration::from_millis(50)).unwrap();
        assert_eq!(
            policy.delay_for(0, Some(Duration::from_secs(100))),
            Duration::from_millis(50)
        );
        assert_eq!(policy.delay_for(2, None), Duration::from_millis(40));
    }
}
