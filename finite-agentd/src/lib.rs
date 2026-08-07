mod config;
mod connections;
mod daemon;
mod directory;
mod ledger;
mod payload;
mod skills;
mod supervisor;
mod transport;

use thiserror::Error;

pub use config::{
    AeonSpecializationDesiredStateV1, ConfigApplyResultV1, ConfigManager, ConfigOfferPolicyV1,
    ConfigPreviewV1, DEFAULT_AEON_SPECIALIZATION_BUNDLE, DEFAULT_AEON_SPECIALIZATION_MODEL,
    DEFAULT_AEON_SPECIALIZATION_WORKER_URL, HermesConfigOfferV1, HermesConfigRollbackV1,
    SpecializationCapabilitiesV1, SpecializationNormalizationLimitsV1,
    SpecializationPromptVersionsV1, SpecializationReconcileResultV1, VISION_CONFIG_PATH,
    redact_value,
};
pub use daemon::{
    AgentdStatus, DaemonConfig, HealthServerSpec, SpecializationBundleStatusV1,
    StartupSpecializationBundleConfig, read_status, run_daemon,
};
pub use ledger::{CommandDecision, Ledger};
pub use payload::{
    PayloadSetChannelRequest, PayloadStageRequest, run_payload_stage_cli, run_payload_status_cli,
};
pub use skills::{SkillsSyncRequest, run_skills_sync_cli};
pub use supervisor::{ProcessStatus, SupervisorStatus};

#[derive(Debug, Error)]
pub enum AgentdError {
    #[error("I/O failure: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON failure: {0}")]
    Json(#[from] serde_json::Error),
    #[error("YAML failure: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("database failure: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("HTTP failure: {0}")]
    Http(#[from] reqwest::Error),
    #[error("ledger failure: {0}")]
    Ledger(String),
    #[error("request id conflicts with a previously recorded command: {0}")]
    ConflictingRequestId(String),
    #[error("configuration failure: {0}")]
    Config(String),
    #[error("configuration conflict: {0}")]
    ConfigConflict(String),
    #[error("unsupported configuration path: {0}")]
    UnsupportedConfigPath(String),
    #[error("transport failure: {0}")]
    Transport(String),
    #[error("supervisor failure: {0}")]
    Supervisor(String),
    #[error("authorization failure")]
    Unauthorized,
    #[error("release public key is not configured")]
    MissingReleaseKey,
    #[error("skills bundle rejected: {0}")]
    SkillsBundle(String),
    #[error("service directory unavailable: {0}")]
    ServiceDirectory(String),
    #[error("release channel has no skills bundle head: {0}")]
    SkillsChannelHeadMissing(String),
    #[error("release channel has no payload bundle head: {0}")]
    PayloadChannelHeadMissing(String),
    #[error("shell control socket unavailable: {0}")]
    ShellUnavailable(String),
    #[error("the shell rejected the request ({code}): {message}")]
    ShellRejected { code: String, message: String },
    #[error("unsupported command: {0}")]
    UnsupportedCommand(String),
    #[error("invalid command payload: {0}")]
    InvalidPayload(String),
}

impl AgentdError {
    pub fn public_code(&self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::MissingReleaseKey => "release_key_missing",
            Self::SkillsBundle(_) => "skills_bundle_rejected",
            Self::ServiceDirectory(_) => "service_directory_unavailable",
            Self::SkillsChannelHeadMissing(_) => "skills_channel_head_missing",
            Self::PayloadChannelHeadMissing(_) => "payload_channel_head_missing",
            Self::ShellUnavailable(_) => "shell_unavailable",
            Self::ShellRejected { .. } => "payload_shell_rejected",
            Self::UnsupportedCommand(_) => "unsupported_command",
            Self::InvalidPayload(_) => "invalid_payload",
            Self::ConflictingRequestId(_) => "conflicting_request_id",
            Self::ConfigConflict(_) => "config_conflict",
            Self::UnsupportedConfigPath(_) => "unsupported_config_path",
            Self::Config(_) | Self::Yaml(_) => "config_invalid",
            Self::Supervisor(_) => "supervisor_unavailable",
            Self::Transport(_) | Self::Http(_) => "transport_unavailable",
            Self::Io(_) | Self::Json(_) | Self::Database(_) | Self::Ledger(_) => "internal_error",
        }
    }

    pub fn public_message(&self) -> String {
        match self {
            Self::Unauthorized => {
                "This Principal is not authorized to manage the agent.".to_owned()
            }
            Self::MissingReleaseKey => {
                "FINITE_RELEASE_PUBLIC_KEY is not configured; refusing to fetch or verify a skills bundle."
                    .to_owned()
            }
            Self::UnsupportedCommand(command) => format!("Command {command:?} is not supported."),
            Self::SkillsChannelHeadMissing(channel) => format!(
                "The service directory advertises no skills_bundle head for channel {channel:?}."
            ),
            Self::PayloadChannelHeadMissing(channel) => format!(
                "The service directory advertises no payload_bundle head for channel {channel:?}."
            ),
            Self::ShellUnavailable(message) => {
                format!("The finite-shell control socket is unavailable: {}", truncate(message, 512))
            }
            Self::ShellRejected { code, message } => {
                format!("The shell rejected the request ({code}): {}", truncate(message, 512))
            }
            Self::ServiceDirectory(message)
            | Self::InvalidPayload(message)
            | Self::ConfigConflict(message)
            | Self::Config(message)
            | Self::Supervisor(message)
            | Self::Transport(message)
            | Self::SkillsBundle(message)
            | Self::Ledger(message) => truncate(message, 512),
            Self::UnsupportedConfigPath(path) => {
                format!("Configuration path {path:?} is not supported.")
            }
            Self::ConflictingRequestId(_) => {
                "The request id was already used for different command bytes.".to_owned()
            }
            Self::Yaml(_) => "Hermes configuration is not valid YAML.".to_owned(),
            Self::Http(_) => "The local Finite Chat bridge is unavailable.".to_owned(),
            Self::Io(_) | Self::Json(_) | Self::Database(_) => {
                "The agent could not complete the request safely.".to_owned()
            }
        }
    }
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}
