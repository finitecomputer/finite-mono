use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use crate::EmbeddingProviderConfig;

pub const FBRAIN_EMBEDDING_ENDPOINT_ENV: &str = "FBRAIN_EMBEDDING_ENDPOINT";
pub const FBRAIN_EMBEDDING_BEARER_TOKEN_ENV: &str = "FBRAIN_EMBEDDING_BEARER_TOKEN";
pub const FBRAIN_EMBEDDING_TIMEOUT_SECONDS_ENV: &str = "FBRAIN_EMBEDDING_TIMEOUT_SECONDS";
pub(crate) const DEFAULT_FINITE_BRAIN_SERVER_URL: &str = "https://brain.finite.computer";
/// Canonical public finite-identity Authority origin, matching the default
/// used by the other Finite clients (fsite, finitesitesd).
pub const DEFAULT_FINITE_IDENTITY_AUTHORITY_URL: &str = "https://identity.finite.vip";

/// Process-level environment for the CLI.
#[derive(Debug, Clone)]
pub struct CliEnvironment {
    pub cwd: PathBuf,
    pub config_dir: PathBuf,
    /// Transport URL supplied by the Runtime contract. Process environments
    /// fall back to FiniteBrain's canonical production origin; injected test
    /// and embedded environments may leave it unset.
    pub server_url: Option<String>,
    /// Browser-visible origin used for signed HTTP authorization when it
    /// intentionally differs from the transport URL.
    pub public_base_url: Option<String>,
    /// Optional root for default Brain Working Tree placement. Hosted Agent
    /// Runtimes set this to their durable workspace; native clients may leave
    /// it unset and keep the current-directory default.
    pub working_tree_root: Option<PathBuf>,
    pub now: Option<String>,
    /// finite-identity Authority URL for email proof and native finite.vip
    /// binding flows. Process environments default it to the canonical public
    /// Authority; `FINITE_IDENTITY_AUTHORITY` is an override, never a
    /// requirement. `None` resolves to the same public default at use.
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
        let config_dir = config_dir_from_values(
            env::var_os("FBRAIN_CONFIG_DIR"),
            env::var_os("FINITE_HOME"),
            env::var_os("HOME"),
            &cwd,
        );
        let public_base_url = nonempty(env::var("FINITE_BRAIN_PUBLIC_BASE_URL").ok());
        let server_url = server_url_from_values(
            env::var("FINITE_BRAIN_SERVER_URL").ok(),
            public_base_url.clone(),
        );
        let working_tree_root = env::var_os("FBRAIN_WORKING_TREE_ROOT")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty());
        let now = env::var("FBRAIN_NOW").ok();
        let identity_authority_url =
            identity_authority_url_from_values(env::var("FINITE_IDENTITY_AUTHORITY").ok());
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
                        .clamp(1, 5),
                ),
            });
        Self {
            cwd,
            config_dir,
            server_url: Some(server_url),
            public_base_url,
            working_tree_root,
            now,
            identity_authority_url: Some(identity_authority_url),
            finite_home: None,
            embedding_provider,
        }
    }
}

fn config_dir_from_values(
    explicit: Option<OsString>,
    finite_home: Option<OsString>,
    home: Option<OsString>,
    cwd: &std::path::Path,
) -> PathBuf {
    explicit
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            finite_home
                .filter(|path| !path.is_empty())
                .map(|path| PathBuf::from(path).join("fbrain"))
        })
        .or_else(|| {
            home.filter(|path| !path.is_empty())
                .map(|path| PathBuf::from(path).join(".finitebrain/fbrain"))
        })
        .unwrap_or_else(|| cwd.join(".fbrain"))
}

fn server_url_from_values(server_url: Option<String>, public_base_url: Option<String>) -> String {
    nonempty(server_url)
        .or_else(|| nonempty(public_base_url))
        .unwrap_or_else(|| DEFAULT_FINITE_BRAIN_SERVER_URL.to_owned())
}

fn identity_authority_url_from_values(override_value: Option<String>) -> String {
    nonempty(override_value).unwrap_or_else(|| DEFAULT_FINITE_IDENTITY_AUTHORITY_URL.to_owned())
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().trim_end_matches('/').to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    #[test]
    fn hosted_config_defaults_below_finite_home() {
        assert_eq!(
            config_dir_from_values(
                None,
                Some(OsString::from("/data/agent")),
                Some(OsString::from("/home/hermes")),
                std::path::Path::new("/data/workspace"),
            ),
            PathBuf::from("/data/agent/fbrain")
        );
    }

    #[test]
    fn production_server_is_a_binary_default_with_advanced_overrides() {
        assert_eq!(
            server_url_from_values(None, None),
            DEFAULT_FINITE_BRAIN_SERVER_URL
        );
        assert_eq!(
            server_url_from_values(Some(" https://smoke.example/ ".to_owned()), None),
            "https://smoke.example"
        );
        assert_eq!(
            server_url_from_values(None, Some(" https://proxy.example/ ".to_owned())),
            "https://proxy.example"
        );
    }

    #[test]
    fn identity_authority_defaults_to_the_public_authority_with_env_override() {
        assert_eq!(
            identity_authority_url_from_values(None),
            DEFAULT_FINITE_IDENTITY_AUTHORITY_URL
        );
        assert_eq!(
            identity_authority_url_from_values(Some(String::new())),
            DEFAULT_FINITE_IDENTITY_AUTHORITY_URL
        );
        assert_eq!(
            identity_authority_url_from_values(Some(" https://identity.example/ ".to_owned())),
            "https://identity.example"
        );
    }
}
