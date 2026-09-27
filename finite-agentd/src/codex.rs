//! Codex (ChatGPT subscription) sign-in and catalog. PR1 carries only the
//! types and signatures; slice A3 (PR3) fills the bodies and `CAPABILITIES`.

use std::path::Path;

use serde::Serialize;

use crate::AgentdError;
use crate::facts::{CodexStateFact, InferenceFacts};

/// `codex.login.v1` and `codex.models.v1` are advertised from PR3.
pub(crate) const CAPABILITIES: &[&str] = &[];

/// The in-memory login attempt (§8.3). `start`, `cancel`, and
/// `cancel_for_disconnect` serialize on it.
#[derive(Debug, Default)]
pub(crate) struct CodexState {}

/// `inference.routes.openai_codex` in status (§3.4).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct CodexRouteStatus {
    pub state: CodexStateFact,
    pub quota_reset_at_ms: Option<u64>,
    pub reported_quota_reset_at_ms: Option<u64>,
    pub login: Option<CodexLoginView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct CodexLoginView {
    pub attempt_id: String,
    pub state: CodexLoginState,
    pub user_code: Option<String>,
    pub verification_uri: Option<String>,
    pub expires_at_ms: u64,
    pub poll_interval_s: u32,
    pub error_code: Option<CodexLoginError>,
    pub retry_after_s: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[expect(dead_code, reason = "wired in A3")]
pub(crate) enum CodexLoginState {
    Pending,
    Committing,
    Approved,
    Canceled,
    Expired,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[expect(dead_code, reason = "wired in A3")]
pub(crate) enum CodexLoginError {
    RateLimited,
    StartFailed,
    PollFailed,
    ExchangeFailed,
    NetworkError,
    SaveFailed,
}

/// The raw authenticated catalog, as the helper's `codex-models` reports it (§8.2).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[expect(dead_code, reason = "wired in A3")]
pub(crate) enum CodexModels {
    Live { models: Vec<String> },
    Unavailable { reason: CodexModelsUnavailable },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[expect(dead_code, reason = "wired in A3")]
pub(crate) enum CodexModelsUnavailable {
    NotSignedIn,
    FetchFailed,
    Empty,
}

/// `None` unless `codex.login.v1` is advertised.
#[expect(dead_code, reason = "wired in A1c")]
pub(crate) fn route_status(
    _state: &CodexState,
    _facts: &InferenceFacts,
) -> Option<CodexRouteStatus> {
    None
}

#[expect(dead_code, reason = "wired in A3")]
pub(crate) async fn start(
    _state: &CodexState,
    _hermes_home: &Path,
) -> Result<CodexLoginView, AgentdError> {
    Err(AgentdError::UnsupportedCommand(
        "agent.codex.login.start".to_owned(),
    ))
}

#[expect(dead_code, reason = "wired in A3")]
pub(crate) async fn cancel(
    _state: &CodexState,
    _attempt_id: &str,
) -> Result<CodexLoginView, AgentdError> {
    Err(AgentdError::UnsupportedCommand(
        "agent.codex.login.cancel".to_owned(),
    ))
}

/// Cancels any login attempt before a Codex disconnect proceeds; a forced
/// cancel leaves the attempt `interrupted`. Without a login manager there is
/// no attempt to cancel, so a disconnect recorded by a newer image still
/// completes here (§4.3 cell 6b).
#[expect(dead_code, reason = "wired in A1c")]
pub(crate) async fn cancel_for_disconnect(_state: &CodexState) -> Result<(), AgentdError> {
    Ok(())
}

#[expect(dead_code, reason = "wired in A3")]
pub(crate) async fn models(_hermes_home: &Path) -> Result<CodexModels, AgentdError> {
    Err(AgentdError::UnsupportedCommand(
        "agent.codex.models".to_owned(),
    ))
}
