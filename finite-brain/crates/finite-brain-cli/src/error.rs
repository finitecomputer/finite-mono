use std::error::Error;
use std::fmt;

/// CLI error.
#[derive(Debug)]
pub enum CliError {
    Io(std::io::Error),
    Json(serde_json::Error),
    InvalidCommand(String),
    InvalidSigner(String),
    InvalidInput(String),
    Http(String),
    Identity(String),
    MissingServer,
    MissingWorkingTree,
    MissingArgument(&'static str),
    NotFound(String),
    Unsupported(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::InvalidCommand(command) => write!(f, "unknown command: {command}"),
            Self::InvalidSigner(reason) => write!(f, "invalid local signer: {reason}"),
            Self::InvalidInput(reason) => write!(f, "invalid input: {reason}"),
            Self::Http(reason) => write!(f, "http request failed: {reason}"),
            Self::Identity(reason) => write!(f, "finite identity error: {reason}"),
            Self::MissingServer => write!(f, "no FiniteBrain server URL configured"),
            Self::MissingWorkingTree => write!(f, "no Vault Working Tree found"),
            Self::MissingArgument(argument) => write!(f, "missing required argument: {argument}"),
            Self::NotFound(item) => write!(f, "not found: {item}"),
            Self::Unsupported(reason) => write!(f, "unsupported: {reason}"),
        }
    }
}

impl Error for CliError {}

impl From<std::io::Error> for CliError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for CliError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
