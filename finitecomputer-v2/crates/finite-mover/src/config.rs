//! The mover's configuration: one JSON file of no secrets, and its key in a
//! file of its own that only the mover may read.
//!
//! ```json
//! {
//!   "finite_chat": "https://finite.chat",
//!   "key_file": "/var/lib/finite-mover/mover.key",
//!   "locks": "/var/lib/finite-mover/locks",
//!   "fleet": { "postgres": { "conninfo": "host=/run/postgresql user=finite_mover dbname=…",
//!                            "sizes_file": "/var/lib/finite-mover/sizes.json" } },
//!   "core": {
//!     "program": ["sudo", "-n", "/run/wrappers/bin/finite-mover-core"],
//!     "admin_email": "…", "admin_workos_user_id": "…", "wait_seconds": 900
//!   },
//!   "hosts": { "<source_host_id>": ["ssh", "-i", "…", "finite-mover@<host>"] }
//! }
//! ```

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::Url;
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::core_cli::{CORE_MARGIN, CoreCli};
use crate::hosts::{HOST_IDLE, HostCommands};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub finite_chat: Url,
    pub key_file: PathBuf,
    pub locks: PathBuf,
    pub fleet: FleetConfig,
    pub core: CoreConfig,
    #[serde(default)]
    pub hosts: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum FleetConfig {
    /// A fleet written to a file (tests, a hand-run roster).
    File(PathBuf),
    /// Core's database, through the mover's read-only role (M1), with
    /// the daily `sizes` run's cache.
    Postgres {
        conninfo: String,
        sizes_file: PathBuf,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreConfig {
    pub program: Vec<String>,
    pub admin_email: String,
    pub admin_workos_user_id: String,
    #[serde(default = "CoreConfig::default_wait")]
    pub wait_seconds: u64,
}

impl CoreConfig {
    fn default_wait() -> u64 {
        900
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{}: {why}", .path.display())]
    Unreadable { path: PathBuf, why: String },
    #[error("{}: the mover's key is readable by others; it must be 0400 or 0600", .0.display())]
    KeyExposed(PathBuf),
    #[error("{}: the mover's key is 64 hex characters (32 bytes)", .0.display())]
    KeyMalformed(PathBuf),
    #[error("finite_chat must be an https origin (http only on loopback)")]
    NotHttps,
}

impl Config {
    pub fn read(path: &Path) -> Result<Self, ConfigError> {
        let unreadable = |why: String| ConfigError::Unreadable {
            path: path.to_path_buf(),
            why,
        };
        let bytes = std::fs::read(path).map_err(|error| unreadable(error.to_string()))?;
        let config: Config =
            serde_json::from_slice(&bytes).map_err(|error| unreadable(error.to_string()))?;
        let loopback = matches!(
            config.finite_chat.host_str(),
            Some("127.0.0.1" | "localhost" | "[::1]")
        );
        if config.finite_chat.scheme() != "https"
            && !(config.finite_chat.scheme() == "http" && loopback)
        {
            return Err(ConfigError::NotHttps);
        }
        Ok(config)
    }

    /// The mover's key: read once, kept in memory, never printed.
    pub fn key(&self) -> Result<Zeroizing<[u8; 32]>, ConfigError> {
        let path = &self.key_file;
        let metadata = std::fs::metadata(path).map_err(|error| ConfigError::Unreadable {
            path: path.clone(),
            why: error.to_string(),
        })?;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ConfigError::KeyExposed(path.clone()));
        }
        let text = Zeroizing::new(std::fs::read_to_string(path).map_err(|error| {
            ConfigError::Unreadable {
                path: path.clone(),
                why: error.to_string(),
            }
        })?);
        let hex = text.trim();
        let mut key = Zeroizing::new([0u8; 32]);
        hex::decode_to_slice(hex, &mut key[..])
            .map_err(|_| ConfigError::KeyMalformed(path.clone()))?;
        Ok(key)
    }

    pub fn core(&self) -> CoreCli {
        CoreCli {
            program: self.core.program.clone(),
            admin_email: self.core.admin_email.clone(),
            admin_workos_user_id: self.core.admin_workos_user_id.clone(),
            wait_seconds: self.core.wait_seconds,
            deadline: Duration::from_secs(self.core.wait_seconds) + CORE_MARGIN,
        }
    }

    pub fn hosts(&self) -> HostCommands {
        HostCommands {
            hosts: self.hosts.clone(),
            idle: HOST_IDLE,
        }
    }
}

/// The npub of `key`: what the deployment's `migrations.mover` names.
pub fn npub(key: &[u8; 32]) -> Option<String> {
    use nostr::nips::nip19::ToBech32;
    let secret = nostr::SecretKey::from_slice(key).ok()?;
    nostr::Keys::new(secret).public_key().to_bech32().ok()
}
