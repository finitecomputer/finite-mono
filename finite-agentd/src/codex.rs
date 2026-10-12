//! Codex status wire types. Sign-in and catalog commands are not implemented yet.

use serde::Serialize;

use crate::facts::CodexStateFact;

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
