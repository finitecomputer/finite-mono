use std::env;
use std::path::PathBuf;

/// Process-level environment for the CLI.
#[derive(Debug, Clone)]
pub struct CliEnvironment {
    pub cwd: PathBuf,
    pub config_dir: PathBuf,
    /// Optional root for default Vault Working Tree placement. Hosted Agent
    /// Runtimes set this to their durable workspace; native clients may leave
    /// it unset and keep the current-directory default.
    pub working_tree_root: Option<PathBuf>,
    pub now: Option<String>,
    /// Optional finite-identity Authority URL for email proof and native
    /// finite.vip binding flows.
    pub identity_authority_url: Option<String>,
    /// Loopback Finite Chat resident service used by hosted agents to obtain
    /// a user's bounded Personal Vault bootstrap authorization.
    pub finitechat_service_url: Option<String>,
    /// Trusted embedder/test seam for user-owned Personal Vault creation.
    /// The agent-native process entrypoint always leaves this false.
    #[doc(hidden)]
    pub allow_personal_vault_creation: bool,
    /// Explicit Finite home for the shared identity, used by tests and
    /// embedders. `None` resolves per the Finite Identity Contract v1:
    /// `$FINITE_HOME/identity/` when `FINITE_HOME` is set, otherwise
    /// `$HOME/.finite/identity/`. Deliberately not a CLI flag: the identity
    /// location is convention, not per-tool configuration.
    pub finite_home: Option<PathBuf>,
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
        let finitechat_service_url = env::var("FINITECHAT_HERMES_SERVICE_URL")
            .ok()
            .map(|value| value.trim().trim_end_matches('/').to_owned())
            .filter(|value| !value.is_empty());
        Self {
            cwd,
            config_dir,
            working_tree_root,
            now,
            identity_authority_url,
            finitechat_service_url,
            allow_personal_vault_creation: false,
            finite_home: None,
        }
    }
}
