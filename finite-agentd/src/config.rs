use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tempfile::NamedTempFile;

use crate::AgentdError;
use crate::ledger::{Ledger, hex_digest};

pub const VISION_CONFIG_PATH: &str = "auxiliary.vision";
pub const MODEL_CONFIG_PATH: &str = "model";
pub const TELEGRAM_CONFIG_PATH: &str = "gateway.platforms.telegram";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigOfferPolicyV1 {
    ApplyIfUnset,
    ReplaceWithConfirmation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HermesConfigOfferV1 {
    pub proposal_id: String,
    pub path: String,
    pub policy: ConfigOfferPolicyV1,
    #[serde(default)]
    pub approved: bool,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HermesConfigRollbackV1 {
    pub proposal_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigPreviewV1 {
    pub proposal_id: String,
    pub path: String,
    pub policy: ConfigOfferPolicyV1,
    pub current: Value,
    pub proposed: Value,
    pub ownership: String,
    pub would_apply: bool,
    pub requires_confirmation: bool,
    pub conflict: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigApplyResultV1 {
    pub proposal_id: String,
    pub path: String,
    pub applied: bool,
    pub already_applied: bool,
    pub restart_required: bool,
}

#[derive(Debug, Clone)]
pub struct ConfigManager {
    path: PathBuf,
    ledger: Ledger,
}

impl ConfigManager {
    pub fn new(path: impl Into<PathBuf>, ledger: Ledger) -> Self {
        Self {
            path: path.into(),
            ledger,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn current_value(&self, path: &str) -> Result<Value, AgentdError> {
        let (_bytes, document) = self.load_document()?;
        Ok(value_at_path(&document, path)
            .cloned()
            .unwrap_or(Value::Null))
    }

    pub fn preview(&self, offer: &HermesConfigOfferV1) -> Result<ConfigPreviewV1, AgentdError> {
        validate_offer(offer)?;
        let (_bytes, document) = self.load_document()?;
        let current = value_at_path(&document, &offer.path)
            .cloned()
            .unwrap_or(Value::Null);
        let current_hash = value_hash(&current)?;
        let ownership = self.ledger.config_ownership(&offer.path)?;
        let owned_match = ownership
            .as_ref()
            .is_some_and(|record| record.applied_hash == current_hash);
        let unset = value_is_unset(&offer.path, &current);
        let requires_confirmation =
            offer.policy == ConfigOfferPolicyV1::ReplaceWithConfirmation && !offer.approved;
        let conflict = match offer.policy {
            ConfigOfferPolicyV1::ApplyIfUnset if !unset && !owned_match => Some(
                "Hermes or the user already owns this field; Finite will not overwrite it"
                    .to_owned(),
            ),
            ConfigOfferPolicyV1::ReplaceWithConfirmation if !offer.approved => {
                Some("explicit confirmation is required before replacement".to_owned())
            }
            _ => None,
        };
        Ok(ConfigPreviewV1 {
            proposal_id: offer.proposal_id.clone(),
            path: offer.path.clone(),
            policy: offer.policy,
            current: redact_value(&current),
            proposed: redact_value(&offer.value),
            ownership: if owned_match {
                "finite".to_owned()
            } else if unset {
                "unset".to_owned()
            } else {
                "custom".to_owned()
            },
            would_apply: conflict.is_none(),
            requires_confirmation,
            conflict,
        })
    }

    pub fn apply(
        &self,
        offer: &HermesConfigOfferV1,
        validate: impl FnOnce() -> Result<(), AgentdError>,
    ) -> Result<ConfigApplyResultV1, AgentdError> {
        let preview = self.preview(offer)?;
        if let Some(history) = self.ledger.config_history(&offer.proposal_id)? {
            if history.path != offer.path {
                return Err(AgentdError::ConfigConflict(
                    "proposal id was already used for another configuration path".to_owned(),
                ));
            }
            if history.rolled_back {
                return Err(AgentdError::ConfigConflict(
                    "rolled-back proposal ids cannot be reused".to_owned(),
                ));
            }
            let (_bytes, document) = self.load_document()?;
            let current = value_at_path(&document, &offer.path)
                .cloned()
                .unwrap_or(Value::Null);
            if value_hash(&current)? == history.applied_hash {
                return Ok(ConfigApplyResultV1 {
                    proposal_id: offer.proposal_id.clone(),
                    path: offer.path.clone(),
                    applied: false,
                    already_applied: true,
                    restart_required: false,
                });
            }
            return Err(AgentdError::ConfigConflict(
                "the configuration changed after this proposal was applied".to_owned(),
            ));
        }
        if let Some(conflict) = preview.conflict {
            return Err(AgentdError::ConfigConflict(conflict));
        }

        let (before_bytes, mut document) = self.load_document()?;
        set_value_at_path(&mut document, &offer.path, offer.value.clone())?;
        let rendered = serde_yaml::to_string(&document)?;
        self.atomic_write(rendered.as_bytes())?;
        if let Err(error) = validate() {
            self.atomic_write(&before_bytes)?;
            return Err(error);
        }
        let applied_hash = value_hash(&offer.value)?;
        if let Err(error) = self.ledger.record_config_apply(
            &offer.proposal_id,
            &offer.path,
            &before_bytes,
            &applied_hash,
        ) {
            self.atomic_write(&before_bytes)?;
            return Err(error);
        }
        Ok(ConfigApplyResultV1 {
            proposal_id: offer.proposal_id.clone(),
            path: offer.path.clone(),
            applied: true,
            already_applied: false,
            restart_required: true,
        })
    }

    pub fn rollback(
        &self,
        request: &HermesConfigRollbackV1,
        validate: impl FnOnce() -> Result<(), AgentdError>,
    ) -> Result<ConfigApplyResultV1, AgentdError> {
        let history = self
            .ledger
            .config_history(&request.proposal_id)?
            .ok_or_else(|| AgentdError::ConfigConflict("proposal was not applied".to_owned()))?;
        if history.rolled_back {
            return Ok(ConfigApplyResultV1 {
                proposal_id: history.proposal_id,
                path: history.path,
                applied: false,
                already_applied: true,
                restart_required: false,
            });
        }
        let (current_bytes, document) = self.load_document()?;
        let current = value_at_path(&document, &history.path)
            .cloned()
            .unwrap_or(Value::Null);
        if value_hash(&current)? != history.applied_hash {
            return Err(AgentdError::ConfigConflict(
                "Hermes or the user changed this field after Finite applied it; rollback will not overwrite that change"
                    .to_owned(),
            ));
        }
        self.atomic_write(&history.before_bytes)?;
        if let Err(error) = validate() {
            self.atomic_write(&current_bytes)?;
            return Err(error);
        }
        if let Err(error) = self
            .ledger
            .record_config_rollback(&history.proposal_id, &history.path)
        {
            self.atomic_write(&current_bytes)?;
            return Err(error);
        }
        Ok(ConfigApplyResultV1 {
            proposal_id: history.proposal_id,
            path: history.path,
            applied: true,
            already_applied: false,
            restart_required: true,
        })
    }

    fn load_document(&self) -> Result<(Vec<u8>, Value), AgentdError> {
        let bytes = fs::read(&self.path)?;
        let document = serde_yaml::from_slice::<Value>(&bytes)?;
        if !document.is_object() {
            return Err(AgentdError::Config(
                "Hermes config root must be an object".to_owned(),
            ));
        }
        Ok((bytes, document))
    }

    fn atomic_write(&self, bytes: &[u8]) -> Result<(), AgentdError> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| AgentdError::Config("Hermes config has no parent".to_owned()))?;
        fs::create_dir_all(parent)?;
        let mode = fs::metadata(&self.path)
            .map(|metadata| metadata.permissions().mode() & 0o777)
            .unwrap_or(0o600);
        let mut temporary = NamedTempFile::new_in(parent)?;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(mode))?;
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&self.path)
            .map_err(|error| AgentdError::Io(error.error))?;
        File::open(parent)?.sync_all()?;
        Ok(())
    }
}

fn validate_offer(offer: &HermesConfigOfferV1) -> Result<(), AgentdError> {
    if offer.proposal_id.trim().is_empty() || offer.proposal_id.len() > 128 {
        return Err(AgentdError::Config(
            "proposal_id must contain 1..128 bytes".to_owned(),
        ));
    }
    match offer.path.as_str() {
        VISION_CONFIG_PATH => validate_vision_value(&offer.value),
        MODEL_CONFIG_PATH => validate_model_value(&offer.value),
        TELEGRAM_CONFIG_PATH => validate_telegram_value(&offer.value),
        _ => Err(AgentdError::UnsupportedConfigPath(offer.path.clone())),
    }
}

fn validate_model_value(value: &Value) -> Result<(), AgentdError> {
    let object = value
        .as_object()
        .ok_or_else(|| AgentdError::Config("model must be an object".to_owned()))?;
    let allowed = ["default", "provider", "base_url", "api_key", "api_mode"];
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(AgentdError::Config(format!(
                "model field {key:?} is not allowlisted"
            )));
        }
    }
    for key in ["default", "provider"] {
        let valid = object
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty() && value.len() <= 256);
        if !valid {
            return Err(AgentdError::Config(format!("model.{key} is required")));
        }
    }
    for key in ["base_url", "api_key", "api_mode"] {
        if let Some(value) = object.get(key)
            && !value.is_string()
        {
            return Err(AgentdError::Config(format!("model.{key} must be a string")));
        }
    }
    if let Some(mode) = object.get("api_mode").and_then(Value::as_str)
        && !["chat_completions", "codex_responses", "anthropic_messages"].contains(&mode)
    {
        return Err(AgentdError::Config(
            "model.api_mode is unsupported".to_owned(),
        ));
    }
    Ok(())
}

fn validate_telegram_value(value: &Value) -> Result<(), AgentdError> {
    let object = value
        .as_object()
        .ok_or_else(|| AgentdError::Config("Telegram settings must be an object".to_owned()))?;
    let allowed = [
        "enabled",
        "token",
        "home_channel",
        "reply_to_mode",
        "gateway_restart_notification",
        "typing_indicator",
        "extra",
    ];
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(AgentdError::Config(format!(
                "Telegram field {key:?} is not allowlisted"
            )));
        }
    }
    if !object.get("enabled").is_some_and(Value::is_boolean) {
        return Err(AgentdError::Config(
            "Telegram enabled state is required".to_owned(),
        ));
    }
    if object.get("enabled") == Some(&Value::Bool(true)) {
        let token = object
            .get("token")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let valid = token.len() >= 32
            && token.len() <= 256
            && token.split_once(':').is_some_and(|(id, secret)| {
                id.bytes().all(|byte| byte.is_ascii_digit()) && !secret.is_empty()
            });
        if !valid {
            return Err(AgentdError::Config(
                "Telegram bot token is invalid".to_owned(),
            ));
        }
    }
    if let Some(mode) = object.get("reply_to_mode").and_then(Value::as_str)
        && !["off", "first", "all"].contains(&mode)
    {
        return Err(AgentdError::Config(
            "Telegram reply mode is unsupported".to_owned(),
        ));
    }
    if let Some(home) = object.get("home_channel") {
        let home = home
            .as_object()
            .ok_or_else(|| AgentdError::Config("Telegram home channel is invalid".to_owned()))?;
        for key in ["platform", "chat_id", "name"] {
            if !home
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
            {
                return Err(AgentdError::Config(format!(
                    "Telegram home channel {key} is required"
                )));
            }
        }
        if home.get("platform").and_then(Value::as_str) != Some("telegram") {
            return Err(AgentdError::Config(
                "Telegram home channel platform is invalid".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_vision_value(value: &Value) -> Result<(), AgentdError> {
    let object = value
        .as_object()
        .ok_or_else(|| AgentdError::Config("auxiliary.vision must be an object".to_owned()))?;
    let allowed = [
        "provider",
        "model",
        "base_url",
        "api_key",
        "api_mode",
        "timeout",
        "download_timeout",
        "extra_body",
    ];
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(AgentdError::Config(format!(
                "auxiliary.vision field {key:?} is not allowlisted"
            )));
        }
    }
    let provider = object
        .get("provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if provider.is_empty() || provider.len() > 128 {
        return Err(AgentdError::Config(
            "auxiliary.vision.provider is required".to_owned(),
        ));
    }
    for key in ["model", "base_url", "api_key"] {
        if let Some(value) = object.get(key)
            && !value.is_string()
        {
            return Err(AgentdError::Config(format!(
                "auxiliary.vision.{key} must be a string"
            )));
        }
    }
    if let Some(mode) = object.get("api_mode").and_then(Value::as_str)
        && !["chat_completions", "codex_responses", "anthropic_messages"].contains(&mode)
    {
        return Err(AgentdError::Config(
            "auxiliary.vision.api_mode is unsupported".to_owned(),
        ));
    }
    for key in ["timeout", "download_timeout"] {
        if let Some(value) = object.get(key) {
            let Some(value) = value.as_u64() else {
                return Err(AgentdError::Config(format!(
                    "auxiliary.vision.{key} must be an integer"
                )));
            };
            if !(1..=900).contains(&value) {
                return Err(AgentdError::Config(format!(
                    "auxiliary.vision.{key} must be between 1 and 900 seconds"
                )));
            }
        }
    }
    if let Some(extra_body) = object.get("extra_body")
        && !extra_body.is_object()
    {
        return Err(AgentdError::Config(
            "auxiliary.vision.extra_body must be an object".to_owned(),
        ));
    }
    Ok(())
}

fn value_is_unset(path: &str, value: &Value) -> bool {
    if value.is_null() {
        return true;
    }
    let Some(object) = value.as_object() else {
        return false;
    };
    if path != VISION_CONFIG_PATH {
        return object.is_empty();
    }
    object.is_empty()
        || object
            .get("provider")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_none_or(|provider| provider.is_empty() || provider == "auto")
}

fn value_at_path<'a>(document: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = document;
    for part in path.split('.') {
        current = current.as_object()?.get(part)?;
    }
    Some(current)
}

fn set_value_at_path(document: &mut Value, path: &str, value: Value) -> Result<(), AgentdError> {
    let mut parts = path.split('.').peekable();
    let mut current = document;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            let object = current.as_object_mut().ok_or_else(|| {
                AgentdError::Config(format!("configuration parent for {path} is not an object"))
            })?;
            object.insert(part.to_owned(), value);
            return Ok(());
        }
        let object = current.as_object_mut().ok_or_else(|| {
            AgentdError::Config(format!("configuration parent for {path} is not an object"))
        })?;
        current = object
            .entry(part.to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    Err(AgentdError::Config(
        "configuration path is empty".to_owned(),
    ))
}

fn value_hash(value: &Value) -> Result<String, AgentdError> {
    Ok(hex_digest(&serde_json::to_vec(value)?))
}

pub fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    let lowered = key.to_ascii_lowercase();
                    let redacted = ["key", "token", "secret", "password", "credential"]
                        .iter()
                        .any(|needle| lowered.contains(needle));
                    (
                        key.clone(),
                        if redacted {
                            json!("<redacted>")
                        } else {
                            redact_value(value)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact_value).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::*;

    const ORIGINAL: &str = "model: anthropic/claude\nauxiliary:\n  vision:\n    provider: auto\n";

    fn manager() -> (tempfile::TempDir, ConfigManager) {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("config.yaml");
        fs::write(&config_path, ORIGINAL).unwrap();
        let ledger = Ledger::open(directory.path().join("agentd.sqlite3")).unwrap();
        (directory, ConfigManager::new(config_path, ledger))
    }

    fn offer(proposal_id: &str) -> HermesConfigOfferV1 {
        HermesConfigOfferV1 {
            proposal_id: proposal_id.to_owned(),
            path: VISION_CONFIG_PATH.to_owned(),
            policy: ConfigOfferPolicyV1::ApplyIfUnset,
            approved: false,
            value: json!({
                "provider": "openai",
                "model": "gpt-5-mini",
                "api_key": "never-display-this"
            }),
        }
    }

    #[test]
    fn preview_redacts_secrets_and_apply_is_idempotent() {
        let (_directory, manager) = manager();
        let offer = offer("vision-1");
        let preview = manager.preview(&offer).unwrap();
        assert_eq!(preview.ownership, "unset");
        assert!(preview.would_apply);
        assert_eq!(preview.proposed["api_key"], "<redacted>");

        let applied = manager.apply(&offer, || Ok(())).unwrap();
        assert!(applied.applied);
        assert!(applied.restart_required);
        let repeated = manager.apply(&offer, || Ok(())).unwrap();
        assert!(repeated.already_applied);
        assert!(!repeated.restart_required);
    }

    #[test]
    fn failed_validation_restores_exact_previous_bytes() {
        let (_directory, manager) = manager();
        let error = manager
            .apply(&offer("vision-invalid"), || {
                Err(AgentdError::Config("Hermes rejected it".to_owned()))
            })
            .unwrap_err();
        assert!(matches!(error, AgentdError::Config(_)));
        assert_eq!(fs::read(manager.path()).unwrap(), ORIGINAL.as_bytes());
        assert!(
            manager
                .ledger
                .config_history("vision-invalid")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn apply_if_unset_does_not_overwrite_custom_hermes_config() {
        let (_directory, manager) = manager();
        fs::write(
            manager.path(),
            "auxiliary:\n  vision:\n    provider: custom\n",
        )
        .unwrap();
        let preview = manager.preview(&offer("vision-custom")).unwrap();
        assert_eq!(preview.ownership, "custom");
        assert!(!preview.would_apply);
        assert!(matches!(
            manager.apply(&offer("vision-custom"), || Ok(())),
            Err(AgentdError::ConfigConflict(_))
        ));
    }

    #[test]
    fn rollback_is_exact_but_refuses_to_clobber_later_user_edits() {
        let (_directory, manager) = manager();
        manager.apply(&offer("vision-rollback"), || Ok(())).unwrap();
        let rollback = HermesConfigRollbackV1 {
            proposal_id: "vision-rollback".to_owned(),
        };
        let result = manager.rollback(&rollback, || Ok(())).unwrap();
        assert!(result.applied);
        assert_eq!(fs::read(manager.path()).unwrap(), ORIGINAL.as_bytes());

        manager.apply(&offer("vision-drift"), || Ok(())).unwrap();
        fs::write(
            manager.path(),
            "auxiliary:\n  vision:\n    provider: user-edited\n",
        )
        .unwrap();
        assert!(matches!(
            manager.rollback(
                &HermesConfigRollbackV1 {
                    proposal_id: "vision-drift".to_owned()
                },
                || Ok(())
            ),
            Err(AgentdError::ConfigConflict(_))
        ));
    }
}
