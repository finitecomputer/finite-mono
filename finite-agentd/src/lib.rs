mod codex;
mod config;
mod connections;
mod daemon;
mod executor;
mod facts;
mod helper;
mod hosted_hermes;
mod hosted_hermes_pull;
mod inference;
mod intent;
mod ledger;
mod openrouter;
mod simplex;
mod supervisor;
mod transport;

use thiserror::Error;

pub use config::{
    ConfigApplyResultV1, ConfigManager, ConfigOfferPolicyV1, ConfigPreviewV1, HermesConfigOfferV1,
    HermesConfigRollbackV1, VISION_CONFIG_PATH, redact_value,
};
pub use daemon::{
    AgentdStatus, DaemonConfig, SpecializationBundleStatusV1, read_status, run_daemon,
};
pub use hosted_hermes::run_hosted_hermes;
pub use ledger::{CommandDecision, Ledger};
pub use supervisor::{ProcessState, ProcessStatus, SupervisorStatus};

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
    #[error("not connected: {0}")]
    NotConnected(String),
    #[error("sign-in required")]
    SignInRequired,
    #[error("credential rejected: {0}")]
    CredentialRejected(String),
    #[error("provider unavailable: {0}")]
    ProviderUnavailable(String),
    #[error("model catalog unavailable")]
    CatalogUnavailable,
    #[error("model unavailable")]
    ModelUnavailable,
    #[error("attempt not found")]
    AttemptNotFound,
    #[error("another inference operation is in progress")]
    OperationInProgress,
    #[error("a ChatGPT disconnect is in progress")]
    DisconnectInProgress,
    #[error("Finite Private is not fully set up")]
    FinitePrivateUnavailable,
    #[error("the agent's inference facts could not be read")]
    FactsUnavailable,
    #[error("OpenRouter key has no remaining allowance")]
    KeyAllowanceExhausted,
    #[error("validated key saved but activation not recorded")]
    ActivationNotRecorded,
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
            Self::NotConnected(_) => "not_connected",
            Self::SignInRequired => "sign_in_required",
            Self::CredentialRejected(_) => "credential_rejected",
            Self::ProviderUnavailable(_) => "provider_unavailable",
            Self::CatalogUnavailable => "catalog_unavailable",
            Self::ModelUnavailable => "model_unavailable",
            Self::AttemptNotFound => "attempt_not_found",
            Self::OperationInProgress => "operation_in_progress",
            Self::DisconnectInProgress => "disconnect_in_progress",
            Self::FinitePrivateUnavailable => "finite_private_unavailable",
            Self::FactsUnavailable => "facts_unavailable",
            Self::KeyAllowanceExhausted => "key_allowance_exhausted",
            Self::ActivationNotRecorded => "activation_not_recorded",
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
            | Self::Ledger(message)
            | Self::NotConnected(message)
            | Self::CredentialRejected(message)
            | Self::ProviderUnavailable(message) => truncate(message, 512),
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
            Self::SignInRequired => "Sign in to ChatGPT again first.".to_owned(),
            Self::CatalogUnavailable => {
                "ChatGPT's model list is unavailable right now. Your saved model is kept.".to_owned()
            }
            Self::ModelUnavailable => {
                "That model isn't available for this ChatGPT account.".to_owned()
            }
            Self::AttemptNotFound => "That sign-in is no longer active. Start again.".to_owned(),
            Self::OperationInProgress => {
                "Another connection change is still finishing. Try again in a moment.".to_owned()
            }
            Self::DisconnectInProgress => "ChatGPT is being removed from this agent. Wait for that to finish, or try the removal again.".to_owned(),
            Self::FinitePrivateUnavailable => "Disconnecting would leave this agent without a model: Finite Private isn't fully set up here. Choose another model first.".to_owned(),
            Self::FactsUnavailable => {
                "The agent couldn't check its setup right now. Try again in a moment.".to_owned()
            }
            Self::KeyAllowanceExhausted => "This key has no remaining allowance. Raise its limit at openrouter.ai/keys, or use another key.".to_owned(),
            Self::ActivationNotRecorded => "The key was saved, but the agent couldn't record the switch to OpenRouter. Try Use OpenRouter again.".to_owned(),
        }
    }
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}
