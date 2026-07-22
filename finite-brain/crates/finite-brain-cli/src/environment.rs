use std::env;
use std::path::PathBuf;
use std::time::Duration;

use crate::EmbeddingProviderConfig;

pub const FBRAIN_EMBEDDING_ENDPOINT_ENV: &str = "FBRAIN_EMBEDDING_ENDPOINT";
pub const FBRAIN_EMBEDDING_BEARER_TOKEN_ENV: &str = "FBRAIN_EMBEDDING_BEARER_TOKEN";
pub const FBRAIN_EMBEDDING_TIMEOUT_SECONDS_ENV: &str = "FBRAIN_EMBEDDING_TIMEOUT_SECONDS";

/// Process-level environment for the CLI.
#[derive(Debug, Clone)]
pub struct CliEnvironment {
    pub cwd: PathBuf,
    pub config_dir: PathBuf,
    /// Optional root for default Brain Working Tree placement. Hosted Agent
    /// Runtimes set this to their durable workspace; native clients may leave
    /// it unset and keep the current-directory default.
    pub working_tree_root: Option<PathBuf>,
    pub now: Option<String>,
    /// Optional finite-identity Authority URL for email proof and native
    /// finite.vip binding flows.
    pub identity_authority_url: Option<String>,
    /// Explicit Finite home for the shared identity, used by tests and
    /// embedders. `None` resolves per the Finite Identity Contract v1:
    /// `$FINITE_HOME/identity/` when `FINITE_HOME` is set, otherwise
    /// `$HOME/.finite/identity/`. Deliberately not a CLI flag: the identity
    /// location is convention, not per-tool configuration.
    pub finite_home: Option<PathBuf>,
    /// Runtime-only semantic provider configuration. The bearer token is never
    /// serialized into Brain or search-index state.
    pub embedding_provider: Option<EmbeddingProviderConfig>,
}

impl CliEnvironment {
    /// Build a CLI environment from process env vars.
    pub fn from_process() -> Self {
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let config_dir = env::var_os("FBRAIN_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                env::var_os("HOME").map(|home| PathBuf::from(home).join(".finitebrain/fbrain"))
            })
            .unwrap_or_else(|| cwd.join(".fbrain"));
        let working_tree_root = env::var_os("FBRAIN_WORKING_TREE_ROOT")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty());
        let now = env::var("FBRAIN_NOW").ok();
        let identity_authority_url = env::var("FINITE_IDENTITY_AUTHORITY")
            .ok()
            .map(|value| value.trim().trim_end_matches('/').to_owned())
            .filter(|value| !value.is_empty());
        let embedding_provider = env::var(FBRAIN_EMBEDDING_ENDPOINT_ENV)
            .ok()
            .zip(env::var(FBRAIN_EMBEDDING_BEARER_TOKEN_ENV).ok())
            .map(|(endpoint, bearer_token)| EmbeddingProviderConfig {
                endpoint,
                bearer_token,
                timeout: Duration::from_secs(
                    env::var(FBRAIN_EMBEDDING_TIMEOUT_SECONDS_ENV)
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(10)
                        .clamp(1, 120),
                ),
            });
        Self {
            cwd,
            config_dir,
            working_tree_root,
            now,
            identity_authority_url,
            finite_home: None,
            embedding_provider,
        }
    }
}
