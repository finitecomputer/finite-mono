use std::error::Error;
use std::fmt;
use std::path::PathBuf;

/// CLI error.
#[derive(Debug)]
pub enum CliError {
    Io(std::io::Error),
    Json(serde_json::Error),
    SearchIndex(String),
    SearchIndexCorrupt(String),
    EmbeddingProvider(String),
    InvalidCommand(String),
    InvalidSigner(String),
    InvalidInput(String),
    Http(String),
    /// A successful HTTP response could not be decoded as the expected JSON.
    /// The request may have committed a mutation, so collaboration callers
    /// render this as an indeterminate receipt rather than a clean failure.
    HttpResponseDecode(String),
    /// An authoritative HTTP response from the Brain server. Unlike a
    /// transport error, this response is proof that the server rejected the
    /// request and must not be reported as indeterminate mutation state.
    HttpStatus {
        status: u16,
        body: String,
    },
    Identity(String),
    InsecureWorkingTree {
        path: PathBuf,
        reason: String,
    },
    InsecureWorkingTreePermissions {
        path: PathBuf,
        actual_mode: u32,
        expected_mode: u32,
    },
    InsecureWorkingTreeOwnership {
        path: PathBuf,
    },
    AgentStateMigration {
        path: PathBuf,
        reason: String,
    },
    GrantOpening {
        brain_id: String,
        folder_id: String,
        key_version: u32,
        reason: String,
    },
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
            Self::SearchIndex(reason) => write!(f, "search index error: {reason}"),
            Self::SearchIndexCorrupt(reason) => write!(f, "search index is corrupt: {reason}"),
            Self::EmbeddingProvider(reason) => write!(f, "embedding provider error: {reason}"),
            Self::InvalidCommand(command) => write!(f, "unknown command: {command}"),
            Self::InvalidSigner(reason) => write!(f, "invalid local signer: {reason}"),
            Self::InvalidInput(reason) => write!(f, "invalid input: {reason}"),
            Self::Http(reason) => write!(f, "http request failed: {reason}"),
            Self::HttpResponseDecode(reason) => {
                write!(f, "invalid successful HTTP response: {reason}")
            }
            Self::HttpStatus { status, body } => {
                write!(f, "http request rejected with {status}: {}", body.trim())
            }
            Self::Identity(reason) => write!(f, "finite identity error: {reason}"),
            Self::InsecureWorkingTree { path, reason } => write!(
                f,
                "insecure Brain Working Tree boundary at {}: {reason}; run `fbrain repair` from the Working Tree",
                path.display()
            ),
            Self::InsecureWorkingTreePermissions {
                path,
                actual_mode,
                expected_mode,
            } => write!(
                f,
                "insecure Brain Working Tree permissions at {}: mode is {actual_mode:04o}, expected {expected_mode:04o}; run `fbrain repair` from the Working Tree",
                path.display()
            ),
            Self::InsecureWorkingTreeOwnership { path } => write!(
                f,
                "insecure Brain Working Tree ownership at {}: path is not owned by the current operating-system account; move it to current-account ownership before running `fbrain repair`",
                path.display()
            ),
            Self::AgentStateMigration { path, reason } => write!(
                f,
                "Agent State migration failed at {}: {reason}; restore a valid active state file or remove the Working Tree explicitly",
                path.display()
            ),
            Self::GrantOpening {
                brain_id,
                folder_id,
                key_version,
                reason,
            } => write!(
                f,
                "encrypted Folder Key Grant for {brain_id}/{folder_id} v{key_version} could not be opened: {reason}"
            ),
            Self::MissingServer => write!(f, "no FiniteBrain server URL configured"),
            Self::MissingWorkingTree => write!(f, "no Brain Working Tree found"),
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
