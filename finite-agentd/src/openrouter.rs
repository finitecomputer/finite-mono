//! OpenRouter key checks, usage, and connect. PR1 carries the signatures;
//! slice A1c fills `check_key`, and slice A2 (PR2) fills the rest and
//! `CAPABILITIES`.

use serde::Deserialize;
use serde_json::Value;

use crate::AgentdError;
use crate::facts::InferenceFacts;

/// `openrouter.connect.v1` and `openrouter.usage.v1` are advertised from PR2.
pub(crate) const CAPABILITIES: &[&str] = &[];

/// The `/key` limits a candidate key passed with (§3.9 step 3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct KeyInfo {
    pub limit_usd: Option<f64>,
    pub limit_remaining_usd: Option<f64>,
}

/// The Connections-managed key (`.env` `OPENROUTER_API_KEY`). No `Debug`: it
/// holds the secret.
pub(crate) struct SavedKey {
    #[expect(dead_code, reason = "wired in A2")]
    pub api_key: String,
}

/// The consumed-attempt cache for OAuth codes, in memory for this process.
#[derive(Debug, Default)]
pub(crate) struct OpenRouterState {}

/// `credential` in `finite.agent.openrouter.connect.v1` (§3.9). No `Debug`:
/// it holds a key or an OAuth code.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[expect(dead_code, reason = "wired in A2")]
pub(crate) enum ConnectCredential {
    ApiKey {
        api_key: String,
    },
    OauthCode {
        code: String,
        code_verifier: String,
        attempt_id: String,
    },
}

/// A key obtained from paste or an OAuth exchange, not yet validated or stored.
#[expect(dead_code, reason = "wired in A2")]
pub(crate) struct CandidateKey {
    pub api_key: String,
}

/// `GET /key` with the §3.9 step 3 rejections, in order. `Ok` only for a 200
/// with an object `data` that passes every rejection.
#[expect(dead_code, reason = "wired in A1c")]
pub(crate) async fn check_key(_api_key: &str) -> Result<KeyInfo, AgentdError> {
    Err(AgentdError::UnsupportedCommand(
        "agent.inference.select".to_owned(),
    ))
}

#[expect(dead_code, reason = "wired in A2")]
pub(crate) async fn usage(
    _saved_key: Option<SavedKey>,
    _facts: &InferenceFacts,
) -> Result<Value, AgentdError> {
    Err(AgentdError::UnsupportedCommand(
        "agent.openrouter.usage".to_owned(),
    ))
}

#[expect(dead_code, reason = "wired in A2")]
pub(crate) async fn obtain_candidate(
    _state: &OpenRouterState,
    _credential: ConnectCredential,
) -> Result<CandidateKey, AgentdError> {
    Err(AgentdError::UnsupportedCommand(
        "agent.openrouter.connect".to_owned(),
    ))
}
