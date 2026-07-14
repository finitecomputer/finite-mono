mod config;
mod connections;
mod daemon;
mod ledger;
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
    AgentdStatus, DaemonConfig, SpecializationBundleStatusV1, StartupSpecializationBundleConfig,
    read_status, run_daemon,
};
pub use ledger::{CommandDecision, Ledger};
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
    #[error("unsupported command: {0}")]
    UnsupportedCommand(String),
    #[error("invalid command payload: {0}")]
    InvalidPayload(String),
}

impl AgentdError {
    pub fn public_code(&self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
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
            Self::UnsupportedCommand(command) => format!("Command {command:?} is not supported."),
            Self::InvalidPayload(message)
            | Self::ConfigConflict(message)
            | Self::Config(message)
            | Self::Supervisor(message)
            | Self::Transport(message)
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
