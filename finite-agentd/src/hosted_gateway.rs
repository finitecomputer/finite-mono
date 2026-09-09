use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::AgentdError;

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GatewayConfig {
    enabled: bool,
    token: Option<String>,
}

pub(crate) fn script_path() -> PathBuf {
    std::env::var_os("FINITE_AGENTD_HOSTED_GATEWAY_SCRIPT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/opt/hosted_gateway.py"))
}

fn config_path(agent_home: &Path) -> PathBuf {
    agent_home.join("agentd/hosted-gateway.json")
}

fn read_config(agent_home: &Path) -> Result<GatewayConfig, AgentdError> {
    match fs::read(config_path(agent_home)) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(GatewayConfig::default()),
        Err(error) => Err(error.into()),
    }
}

/// Persist intent first. The supervisor restarts only the dedicated process.
/// Disabling discards the credential, so re-enabling invalidates old clients.
pub(crate) fn configure(agent_home: &Path, enabled: bool) -> Result<(), AgentdError> {
    if !script_path().is_file() {
        return Err(AgentdError::Config(
            "This runtime image does not support hosted gateway access".to_owned(),
        ));
    }
    persist_enabled(agent_home, enabled)
}

fn persist_enabled(agent_home: &Path, enabled: bool) -> Result<(), AgentdError> {
    let mut config = read_config(agent_home)?;
    if enabled && config.token.is_none() {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes)
            .map_err(|_| AgentdError::Config("Could not create a gateway credential".to_owned()))?;
        config.token = Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect());
    }
    config.enabled = enabled;
    if !enabled {
        config.token = None;
    }
    let path = config_path(agent_home);
    let parent = path.parent().expect("gateway config has a parent");
    fs::create_dir_all(parent)?;
    let mut file = NamedTempFile::new_in(parent)?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(&serde_json::to_vec(&config)?)?;
    file.as_file().sync_all()?;
    file.persist(&path).map_err(|error| error.error)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub(crate) async fn status(agent_home: &Path) -> Result<serde_json::Value, AgentdError> {
    let config = read_config(agent_home)?;
    let ready = if config.enabled {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()?
            .get("http://127.0.0.1:9120/api/sessions?limit=1")
            .header(
                "X-Hermes-Session-Token",
                config.token.as_deref().unwrap_or_default(),
            )
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    } else {
        false
    };
    Ok(serde_json::json!({ "enabled": config.enabled, "ready": ready, "token": config.token }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_is_opt_in_private_durable_and_revoked_on_disable() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!read_config(dir.path()).unwrap().enabled);
        persist_enabled(dir.path(), true).unwrap();
        let first = read_config(dir.path()).unwrap().token.unwrap();
        assert_eq!(first.len(), 64);
        assert_eq!(
            fs::metadata(config_path(dir.path()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        persist_enabled(dir.path(), true).unwrap();
        assert_eq!(
            read_config(dir.path()).unwrap().token.as_deref(),
            Some(first.as_str())
        );
        persist_enabled(dir.path(), false).unwrap();
        let disabled = read_config(dir.path()).unwrap();
        assert!(!disabled.enabled);
        assert!(disabled.token.is_none());
        assert!(
            !fs::read_to_string(config_path(dir.path()))
                .unwrap()
                .contains(&first)
        );
        persist_enabled(dir.path(), true).unwrap();
        assert_ne!(read_config(dir.path()).unwrap().token.unwrap(), first);
    }
}
