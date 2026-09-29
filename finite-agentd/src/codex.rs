//! Codex (ChatGPT subscription) sign-in and catalog command contracts.
//! These commands remain unsupported and unadvertised until implemented.

use std::path::Path;

use serde::Serialize;

use crate::AgentdError;
use crate::facts::{CodexStateFact, InferenceFacts};

/// Login and catalog capabilities stay absent until their handlers are implemented.
pub(crate) const CAPABILITIES: &[&str] = &[];

/// The in-memory login attempt. `start`, `cancel`, and
/// `cancel_for_disconnect` serialize on it.
#[derive(Debug, Default)]
pub(crate) struct CodexState {}

/// `inference.routes.openai_codex` in status.
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
#[expect(dead_code, reason = "reserved for Codex login and catalog support")]
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
#[expect(dead_code, reason = "reserved for Codex login and catalog support")]
pub(crate) enum CodexLoginError {
    RateLimited,
    StartFailed,
    PollFailed,
    ExchangeFailed,
    NetworkError,
    SaveFailed,
}

/// The raw authenticated catalog, as the helper's `codex-models` reports it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[expect(dead_code, reason = "reserved for Codex login and catalog support")]
pub(crate) enum CodexModels {
    Live { models: Vec<String> },
    Unavailable { reason: CodexModelsUnavailable },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[expect(dead_code, reason = "reserved for Codex login and catalog support")]
pub(crate) enum CodexModelsUnavailable {
    NotSignedIn,
    FetchFailed,
    Empty,
}

/// `None` unless `codex.login.v1` is advertised.
pub(crate) fn route_status(
    _state: &CodexState,
    _facts: &InferenceFacts,
) -> Option<CodexRouteStatus> {
    None
}

pub(crate) async fn start(
    _state: &CodexState,
    _hermes_home: &Path,
) -> Result<CodexLoginView, AgentdError> {
    Err(AgentdError::UnsupportedCommand(
        "agent.codex.login.start".to_owned(),
    ))
}

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
/// completes here (cell 6b).
pub(crate) async fn cancel_for_disconnect(_state: &CodexState) -> Result<(), AgentdError> {
    Ok(())
}

pub(crate) async fn models(_hermes_home: &Path) -> Result<CodexModels, AgentdError> {
    Err(AgentdError::UnsupportedCommand(
        "agent.codex.models".to_owned(),
    ))
}
