//! `finite-shell`: the only fixed layer of an Agent Runtime (ADR 0006).
//!
//! The shell is PID 1 in the runtime container. It is deterministic code with
//! no LLM involvement: it verifies `/data`, unpacks the seed payload on first
//! boot, stages signed payload generations, flips the `current` symlink
//! atomically with a cheap health gate and automatic rollback, supervises
//! exactly one child (the active generation's `finite-agentd`), and serves
//! `/healthz` + `/contact` itself so a broken payload is still observable.
//!
//! Everything here operates on injected [`ShellSettings`]; nothing mutates
//! global process state, so the whole machine is testable against temp dirs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;

pub mod control;
pub mod fixup;
pub mod generations;
pub mod health;
pub mod poll;
pub mod shims;
pub mod state;
pub mod supervise;

/// The shell's own version: the crate version, compared against payload
/// manifests' `min_shell_version`.
pub const SHELL_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const DATA_ENV: &str = "FINITE_SHELL_DATA";
pub const SEED_DIR_ENV: &str = "FINITE_SHELL_SEED_DIR";
pub const RELEASE_PUBLIC_KEY_ENV: &str = "FINITE_RELEASE_PUBLIC_KEY";
pub const QUIESCE_TIMEOUT_ENV: &str = "FINITE_SHELL_QUIESCE_TIMEOUT_SECS";
pub const HEALTH_TIMEOUT_ENV: &str = "FINITE_SHELL_HEALTH_TIMEOUT_SECS";
pub const HTTP_ADDR_ENV: &str = "FINITE_SHELL_HTTP_ADDR";
pub const AGENTD_REQUIRED_ENV: &str = "FINITE_AGENTD_REQUIRED";
pub const BOOT_INTENT_ENV: &str = "FINITE_AGENT_BOOT_INTENT_JSON";
/// Autonomous channel convergence (M4): how often the shell polls the
/// service directory for its channel's payload head. 0 disables the poller.
pub const POLL_INTERVAL_ENV: &str = "FINITE_SHELL_POLL_INTERVAL_SECS";
/// The same container-level env agentd already receives; the shell reads it
/// for its own channel poll.
pub const SERVICE_DIRECTORY_URL_ENV: &str = "FINITE_SERVICE_DIRECTORY_URL";
/// Dev-harness overrides for the host-global paths (default
/// `/usr/local/bin` and `/usr/local/bin/python3` in the container).
pub const SHIM_DIR_ENV: &str = "FINITE_SHELL_SHIM_DIR";
pub const SHELL_PYTHON_ENV: &str = "FINITE_SHELL_PYTHON";
/// How the shell hands its per-boot control-socket token to the one agentd it
/// supervises. agentd forwards it on every socket request and scrubs it from
/// every child it spawns.
pub const CONTROL_TOKEN_ENV: &str = "FINITE_SHELL_CONTROL_TOKEN";

/// Free space the stage path requires on `/data` beyond twice the tarball
/// size (unpack + breathing room for state writes): 512 MiB.
pub const STAGE_HEADROOM_BYTES: u64 = 512 * 1024 * 1024;

/// The channel an agent is on when `/data/shell/channel` was never written.
pub const DEFAULT_CHANNEL: &str = "stable";

/// Fetch bounds for `stage` from URLs (finite-release policy + limits).
pub const STAGE_MANIFEST_CAP_BYTES: usize = 64 * 1024;
pub const STAGE_TARBALL_CAP_BYTES: usize = 512 * 1024 * 1024;
pub const STAGE_FETCH_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Error)]
pub enum ShellError {
    #[error("I/O failure: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON failure: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Release(#[from] finite_release::ReleaseError),
    #[error("shell configuration is invalid: {0}")]
    Config(String),
    #[error("/data is not usable: {0}")]
    DataDir(String),
    #[error("FINITE_RELEASE_PUBLIC_KEY is not configured; refusing unsigned payloads")]
    MissingReleaseKey,
    #[error("payload requires shell >= {required}, this shell is {shell}")]
    MinShellVersion { required: String, shell: String },
    #[error("version {0} is on the bad list (health gate failed); re-stage with force to retry")]
    VersionBad(String),
    #[error("version {0} is the current generation; nothing to stage")]
    VersionIsCurrent(String),
    #[error("no staged generation; stage a payload first")]
    NothingStaged,
    #[error("no previous generation to roll back to")]
    NoPreviousGeneration,
    #[error("a flip or rollback is already in progress")]
    FlipInProgress,
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("payload contract violation: {0}")]
    Contract(String),
    #[error("fetch failed: {0}")]
    Fetch(String),
    #[error("agentd supervision failed: {0}")]
    Supervisor(String),
    #[error("request token is missing or wrong")]
    Unauthorized,
    #[error(
        "insufficient disk on /data: staging needs {needed_bytes} bytes of headroom, \
         {available_bytes} available"
    )]
    InsufficientDisk {
        needed_bytes: u64,
        available_bytes: u64,
    },
    #[error("version label {0:?} cannot safely name a generation directory")]
    UnsafeVersionLabel(String),
}

impl ShellError {
    /// Stable machine-readable error code for the socket protocol.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "io_failure",
            Self::Json(_) => "json_failure",
            Self::Release(finite_release::ReleaseError::UrlPolicy(_)) => "url_policy",
            Self::Release(_) => "verification_failed",
            Self::Config(_) => "config_invalid",
            Self::DataDir(_) => "data_dir_unusable",
            Self::MissingReleaseKey => "release_key_missing",
            Self::MinShellVersion { .. } => "min_shell_version_unsatisfied",
            Self::VersionBad(_) => "version_bad",
            Self::VersionIsCurrent(_) => "version_is_current",
            Self::NothingStaged => "nothing_staged",
            Self::NoPreviousGeneration => "no_previous_generation",
            Self::FlipInProgress => "flip_in_progress",
            Self::InvalidRequest(_) => "invalid_request",
            Self::Contract(_) => "payload_contract_violation",
            Self::Fetch(_) => "fetch_failed",
            Self::Supervisor(_) => "supervisor_failure",
            Self::Unauthorized => "unauthorized",
            Self::InsufficientDisk { .. } => "insufficient_disk",
            Self::UnsafeVersionLabel(_) => "unsafe_version_label",
        }
    }
}

/// All shell configuration, injected (tests never mutate global env).
#[derive(Debug, Clone)]
pub struct ShellSettings {
    /// `FINITE_SHELL_DATA`, default `/data`.
    pub data_dir: PathBuf,
    /// `FINITE_SHELL_SEED_DIR`, default `/seed`: contains `payload.tar.gz` +
    /// `payload.tar.gz.manifest.json` baked into the image.
    pub seed_dir: PathBuf,
    /// `FINITE_RELEASE_PUBLIC_KEY` (hex). Required for stage-from-URL and
    /// seed verification; the shell refuses to stage unsigned payloads.
    pub release_public_key: Option<String>,
    /// `FINITE_SHELL_QUIESCE_TIMEOUT_SECS`, default 120.
    pub quiesce_timeout: Duration,
    /// `FINITE_SHELL_HEALTH_TIMEOUT_SECS`, default 180.
    pub health_timeout: Duration,
    /// `FINITE_SHELL_HTTP_ADDR`, default `0.0.0.0:8080`.
    pub http_addr: String,
    /// `FINITE_ALLOW_INSECURE_BUNDLE_URL=1` (finite-release policy).
    pub allow_insecure_bundle_url: bool,
    /// Where shims for `<current>/bin/*` are written; `/usr/local/bin`.
    pub shim_dir: PathBuf,
    /// The shell-owned Python interpreter venv fixup points at.
    pub shell_python: PathBuf,
    /// `FINITE_AGENTD_REQUIRED` (health contract, mirrors health_server.py).
    pub agentd_required: bool,
    /// Whether `FINITE_AGENT_BOOT_INTENT_JSON` is set (health contract).
    pub recovery_boot_intent_active: bool,
    /// The PATH the supervised agentd starts from; `<current>/bin` is
    /// prefixed onto it.
    pub base_path: String,
    /// Extra environment for the supervised agentd (tests inject
    /// `FINITECHAT_HOME` here; production inherits the shell's environment).
    pub agentd_env: BTreeMap<String, String>,
    /// agentd restart backoff bounds (1s..60s in production).
    pub restart_backoff_initial: Duration,
    pub restart_backoff_max: Duration,
    /// Crash-loop detection: `crash_loop_restarts` restarts within
    /// `crash_loop_window` (5 in 10 minutes in production).
    pub crash_loop_window: Duration,
    pub crash_loop_restarts: usize,
    /// Poll interval for the flip health gate.
    pub health_gate_poll: Duration,
    /// `FINITE_SHELL_POLL_INTERVAL_SECS`, default 900; zero disables the
    /// autonomous channel poller.
    pub poll_interval: Duration,
    /// `FINITE_SERVICE_DIRECTORY_URL`: where the channel poller fetches the
    /// signed service directory. Absent means the poller stays off.
    pub service_directory_url: Option<String>,
}

impl ShellSettings {
    /// Production defaults over `data_dir`/`seed_dir` (tests override the
    /// host-global paths and timings).
    pub fn new(data_dir: impl Into<PathBuf>, seed_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            seed_dir: seed_dir.into(),
            release_public_key: None,
            quiesce_timeout: Duration::from_secs(120),
            health_timeout: Duration::from_secs(180),
            http_addr: "0.0.0.0:8080".to_owned(),
            allow_insecure_bundle_url: false,
            shim_dir: PathBuf::from("/usr/local/bin"),
            shell_python: PathBuf::from("/usr/local/bin/python3"),
            agentd_required: false,
            recovery_boot_intent_active: false,
            base_path: "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_owned(),
            agentd_env: BTreeMap::new(),
            restart_backoff_initial: Duration::from_secs(1),
            restart_backoff_max: Duration::from_secs(60),
            crash_loop_window: Duration::from_secs(600),
            crash_loop_restarts: 5,
            health_gate_poll: Duration::from_millis(200),
            poll_interval: Duration::from_secs(900),
            service_directory_url: None,
        }
    }

    /// Read settings from the environment (the PID-1 entrypoint).
    pub fn from_env() -> Result<Self, ShellError> {
        fn truthy(value: Option<String>) -> bool {
            value.is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
        }
        fn seconds(name: &str, default: u64) -> Result<Duration, ShellError> {
            match std::env::var(name) {
                Ok(value) => value
                    .trim()
                    .parse::<u64>()
                    .map(Duration::from_secs)
                    .map_err(|_| {
                        ShellError::Config(format!("{name} must be an integer seconds value"))
                    }),
                Err(_) => Ok(Duration::from_secs(default)),
            }
        }
        let data_dir = std::env::var(DATA_ENV).unwrap_or_else(|_| "/data".to_owned());
        let seed_dir = std::env::var(SEED_DIR_ENV).unwrap_or_else(|_| "/seed".to_owned());
        let mut settings = Self::new(data_dir, seed_dir);
        settings.release_public_key = std::env::var(RELEASE_PUBLIC_KEY_ENV)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        settings.quiesce_timeout = seconds(QUIESCE_TIMEOUT_ENV, 120)?;
        settings.health_timeout = seconds(HEALTH_TIMEOUT_ENV, 180)?;
        settings.http_addr =
            std::env::var(HTTP_ADDR_ENV).unwrap_or_else(|_| "0.0.0.0:8080".to_owned());
        settings.allow_insecure_bundle_url =
            std::env::var(finite_release::ALLOW_INSECURE_BUNDLE_URL_ENV)
                .is_ok_and(|value| value == "1");
        settings.poll_interval = seconds(POLL_INTERVAL_ENV, 900)?;
        settings.service_directory_url = std::env::var(SERVICE_DIRECTORY_URL_ENV)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        settings.agentd_required = truthy(std::env::var(AGENTD_REQUIRED_ENV).ok());
        settings.recovery_boot_intent_active =
            std::env::var(BOOT_INTENT_ENV).is_ok_and(|value| !value.is_empty());
        if let Ok(path) = std::env::var("PATH") {
            settings.base_path = path;
        }
        // Dev-harness overrides for the two host-global paths (defaults are
        // the production container layout).
        if let Ok(shim_dir) = std::env::var(SHIM_DIR_ENV) {
            settings.shim_dir = PathBuf::from(shim_dir);
        }
        if let Ok(shell_python) = std::env::var(SHELL_PYTHON_ENV) {
            settings.shell_python = PathBuf::from(shell_python);
        }
        Ok(settings)
    }

    pub fn layout(&self) -> DataLayout {
        DataLayout::new(&self.data_dir)
    }
}

/// The pinned on-disk contract under `/data`.
#[derive(Debug, Clone)]
pub struct DataLayout {
    pub data_dir: PathBuf,
}

impl DataLayout {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
        }
    }

    pub fn generations_dir(&self) -> PathBuf {
        self.data_dir.join("generations")
    }

    pub fn generation_dir(&self, version: &str) -> PathBuf {
        self.generations_dir().join(version)
    }

    pub fn current_link(&self) -> PathBuf {
        self.generations_dir().join("current")
    }

    pub fn previous_link(&self) -> PathBuf {
        self.generations_dir().join("previous")
    }

    /// `<current>` through the symlink — the stable path shims exec through.
    pub fn current_via_link(&self) -> PathBuf {
        self.current_link()
    }

    pub fn shell_dir(&self) -> PathBuf {
        self.data_dir.join("shell")
    }

    pub fn state_path(&self) -> PathBuf {
        self.shell_dir().join("state.json")
    }

    pub fn socket_path(&self) -> PathBuf {
        self.shell_dir().join("shell.sock")
    }

    /// The per-boot control-socket token file (0600, root-owned).
    pub fn control_token_path(&self) -> PathBuf {
        self.shell_dir().join("control-token")
    }

    pub fn channel_path(&self) -> PathBuf {
        self.shell_dir().join("channel")
    }

    /// The shell's release channel: the contents of `/data/shell/channel`,
    /// defaulting to `"stable"` when the file is absent or empty.
    pub fn channel_name(&self) -> String {
        std::fs::read_to_string(self.channel_path())
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_CHANNEL.to_owned())
    }

    pub fn staging_dir(&self) -> PathBuf {
        self.shell_dir().join("staging")
    }

    pub fn agent_home(&self) -> PathBuf {
        self.data_dir.join("agent")
    }

    // The exact files health_server.py reads (and the flip health gate
    // re-reads).

    pub fn agent_config_file(&self) -> PathBuf {
        self.agent_home().join("config.json")
    }

    pub fn identity_file(&self) -> PathBuf {
        self.agent_home().join("identity").join("identity.json")
    }

    pub fn bridge_status_file(&self) -> PathBuf {
        self.agent_home().join("hermes-bridge-status.json")
    }

    pub fn agentd_status_file(&self) -> PathBuf {
        self.agent_home().join("agentd").join("status.json")
    }

    pub fn finitechat_ready_file(&self) -> PathBuf {
        self.agent_home()
            .join("agentd")
            .join("finitechat-ready.json")
    }

    pub fn startup_report_file(&self) -> PathBuf {
        self.agent_home().join("startup-report.json")
    }
}
