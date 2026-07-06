use crate::models::RuntimeProfile;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
const DEFAULT_RUNTIME_PROFILE_ID: &str = "main";
const DEFAULT_RUNTIME_LABEL: &str = "Hermes Runtime";
const DEFAULT_RUNTIME_IMAGE_NAME: &str = "fc-agent-runtime";
const DEFAULT_RUNTIME_FEATURE_SET: &str = "hermes-local";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClusterConfig {
    pub base_domain: Option<String>,
    pub default_runtime_profile: Option<String>,
    pub org_domain: Option<String>,
    pub runtime_profiles: Option<BTreeMap<String, RuntimeProfileConfig>>,
    pub letsencrypt: Option<LetsEncryptConfig>,
    pub oauth2_proxy: Option<OAuth2ProxyConfig>,
    pub dashboard: Option<DashboardConfig>,
    pub gitea: Option<GiteaConfig>,
    pub agent_user: Option<String>,
    pub agent_uid: Option<u32>,
    pub agent_gid: Option<u32>,
    #[serde(default)]
    pub admin_authorized_keys: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardConfig {
    #[serde(default)]
    pub admins: Vec<String>,
    pub namespace: Option<String>,
    pub service_name: Option<String>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LetsEncryptConfig {
    pub email: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OAuth2ProxyConfig {
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GiteaConfig {
    pub enabled: Option<bool>,
    pub hostname: Option<String>,
    pub namespace: Option<String>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeProfileConfig {
    pub label: Option<String>,
    pub image_name: Option<String>,
    pub image_tag: Option<String>,
    pub feature_set: Option<String>,
}

impl ClusterConfig {
    pub fn read(workspace_root: &Path) -> Result<Self> {
        let path = Self::config_path(workspace_root);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn config_path(workspace_root: &Path) -> PathBuf {
        workspace_root.join("agent-cluster/cluster.json")
    }

    pub fn base_domain(&self) -> &str {
        self.base_domain.as_deref().unwrap_or("finite.vip")
    }

    pub fn agent_user(&self) -> &str {
        self.agent_user.as_deref().unwrap_or("node")
    }

    pub fn agent_uid(&self) -> u32 {
        self.agent_uid.unwrap_or(1000)
    }

    pub fn agent_gid(&self) -> u32 {
        self.agent_gid.unwrap_or(1000)
    }

    pub fn runtime_home(&self) -> String {
        format!("/home/{}", self.agent_user())
    }

    pub fn runtime_workspace(&self) -> String {
        format!("{}/workspace", self.runtime_home())
    }

    pub fn default_project_dir(&self) -> String {
        format!("{}/.hermes", self.runtime_home())
    }

    pub fn oauth_enabled(&self) -> bool {
        self.oauth2_proxy
            .as_ref()
            .and_then(|config| config.enabled)
            .unwrap_or(false)
    }

    pub fn letsencrypt_enabled(&self) -> bool {
        self.letsencrypt
            .as_ref()
            .and_then(|config| config.email.as_ref())
            .is_some()
    }

    pub fn published_url_scheme(&self) -> &'static str {
        if self.letsencrypt_enabled() {
            "https"
        } else {
            "http"
        }
    }

    pub fn dashboard_namespace(&self) -> &str {
        self.dashboard
            .as_ref()
            .and_then(|dashboard| dashboard.namespace.as_deref())
            .unwrap_or("fc-dashboard")
    }

    pub fn dashboard_service_name(&self) -> &str {
        self.dashboard
            .as_ref()
            .and_then(|dashboard| dashboard.service_name.as_deref())
            .unwrap_or("dashboard")
    }

    pub fn dashboard_port(&self) -> u16 {
        self.dashboard
            .as_ref()
            .and_then(|dashboard| dashboard.port)
            .unwrap_or(3000)
    }

    pub fn dashboard_internal_url(&self) -> String {
        format!(
            "http://{}.{}.svc.cluster.local:{}",
            self.dashboard_service_name(),
            self.dashboard_namespace(),
            self.dashboard_port()
        )
    }

    pub fn gitea_enabled(&self) -> bool {
        self.gitea
            .as_ref()
            .and_then(|config| config.enabled)
            .unwrap_or(false)
    }

    pub fn gitea_hostname(&self) -> String {
        self.gitea
            .as_ref()
            .and_then(|config| config.hostname.clone())
            .unwrap_or_else(|| format!("git.{}", self.base_domain()))
    }

    pub fn gitea_namespace(&self) -> &str {
        self.gitea
            .as_ref()
            .and_then(|config| config.namespace.as_deref())
            .unwrap_or("fc-gitea")
    }

    pub fn gitea_port(&self) -> u16 {
        self.gitea
            .as_ref()
            .and_then(|config| config.port)
            .unwrap_or(3000)
    }

    pub fn gitea_public_url(&self) -> String {
        format!("https://{}", self.gitea_hostname())
    }

    pub fn admin_authorized_keys_text(&self) -> String {
        self.admin_authorized_keys.join("\n")
    }

    pub fn runtime_profiles(&self) -> BTreeMap<String, RuntimeProfile> {
        match self.runtime_profiles.as_ref() {
            Some(profiles) if !profiles.is_empty() => profiles
                .iter()
                .map(|(id, profile)| (id.clone(), profile.resolve(id, self)))
                .collect(),
            _ => {
                let mut profiles = BTreeMap::new();
                profiles.insert(
                    DEFAULT_RUNTIME_PROFILE_ID.to_string(),
                    RuntimeProfile {
                        id: DEFAULT_RUNTIME_PROFILE_ID.to_string(),
                        label: DEFAULT_RUNTIME_LABEL.to_string(),
                        image_name: DEFAULT_RUNTIME_IMAGE_NAME.to_string(),
                        image_tag: DEFAULT_RUNTIME_PROFILE_ID.to_string(),
                        feature_set: DEFAULT_RUNTIME_FEATURE_SET.to_string(),
                    },
                );
                profiles
            }
        }
    }

    pub fn default_runtime_profile_id(&self) -> Result<String> {
        let profiles = self.runtime_profiles();
        let requested_id = self
            .default_runtime_profile
            .clone()
            .unwrap_or_else(|| DEFAULT_RUNTIME_PROFILE_ID.to_string());
        resolve_runtime_profile_id(&profiles, &requested_id).ok_or_else(|| {
            anyhow::anyhow!(
                "cluster default runtime profile '{}' is not defined",
                requested_id
            )
        })
    }

    pub fn resolve_runtime_profile(
        &self,
        requested: Option<&str>,
    ) -> Result<(String, RuntimeProfile)> {
        let profiles = self.runtime_profiles();
        let profile_id = requested
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or(self.default_runtime_profile_id()?);
        let resolved_id = resolve_runtime_profile_id(&profiles, &profile_id).ok_or_else(|| {
            anyhow::anyhow!(
                "runtime profile '{}' is not defined in cluster.json",
                profile_id
            )
        })?;
        let profile = profiles.get(&resolved_id).cloned().ok_or_else(|| {
            anyhow::anyhow!(
                "runtime profile '{}' is not defined in cluster.json",
                resolved_id
            )
        })?;
        Ok((resolved_id, profile))
    }

    pub fn runtime_image_for_profile(&self, requested: Option<&str>) -> Result<String> {
        let (_, profile) = self.resolve_runtime_profile(requested)?;
        Ok(format!("{}:{}", profile.image_name, profile.image_tag))
    }

    pub fn dashboard_admins(&self) -> Vec<String> {
        self.dashboard
            .as_ref()
            .map(|dashboard| {
                dashboard
                    .admins
                    .iter()
                    .map(|email| email.trim().to_lowercase())
                    .filter(|email| !email.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }
}

impl RuntimeProfileConfig {
    fn resolve(&self, id: &str, _cluster: &ClusterConfig) -> RuntimeProfile {
        RuntimeProfile {
            id: id.to_string(),
            label: self
                .label
                .clone()
                .unwrap_or_else(|| DEFAULT_RUNTIME_LABEL.to_string()),
            image_name: self
                .image_name
                .clone()
                .unwrap_or_else(|| DEFAULT_RUNTIME_IMAGE_NAME.to_string()),
            image_tag: self.image_tag.clone().unwrap_or_else(|| id.to_string()),
            feature_set: self
                .feature_set
                .clone()
                .unwrap_or_else(|| DEFAULT_RUNTIME_FEATURE_SET.to_string()),
        }
    }
}

fn resolve_runtime_profile_id(
    profiles: &BTreeMap<String, RuntimeProfile>,
    requested: &str,
) -> Option<String> {
    if profiles.contains_key(requested) {
        return Some(requested.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{ClusterConfig, DashboardConfig, RuntimeProfileConfig};
    use std::collections::BTreeMap;

    #[test]
    fn cluster_falls_back_to_main_profile() {
        let cluster = ClusterConfig::default();
        let (_, profile) = cluster.resolve_runtime_profile(None).unwrap();
        assert_eq!(profile.id, "main");
        assert_eq!(profile.image_name, "fc-agent-runtime");
        assert_eq!(profile.image_tag, "main");
    }

    #[test]
    fn cluster_resolves_explicit_profiles() {
        let cluster = ClusterConfig {
            default_runtime_profile: Some("main".to_string()),
            runtime_profiles: Some(BTreeMap::from([(
                "main".to_string(),
                RuntimeProfileConfig {
                    label: Some("Hermes Runtime".to_string()),
                    image_name: Some("fc-agent-runtime".to_string()),
                    image_tag: Some("2026-04-06".to_string()),
                    feature_set: Some("hermes-local".to_string()),
                },
            )])),
            ..ClusterConfig::default()
        };

        let (id, profile) = cluster.resolve_runtime_profile(None).unwrap();
        assert_eq!(id, "main");
        assert_eq!(profile.feature_set, "hermes-local");
    }

    #[test]
    fn cluster_normalizes_admins() {
        let cluster = ClusterConfig {
            dashboard: Some(DashboardConfig {
                admins: vec!["Paul@Finite.Vip".to_string(), " ".to_string()],
                namespace: None,
                service_name: None,
                port: None,
            }),
            ..ClusterConfig::default()
        };
        assert_eq!(
            cluster.dashboard_admins(),
            vec!["paul@finite.vip".to_string()]
        );
    }
}
