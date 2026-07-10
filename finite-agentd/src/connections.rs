use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tempfile::NamedTempFile;

use crate::AgentdError;
use crate::config::{
    ConfigManager, ConfigOfferPolicyV1, HermesConfigOfferV1, MODEL_CONFIG_PATH,
    TELEGRAM_CONFIG_PATH,
};

const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const OPENROUTER_DEFAULT_MODEL: &str = "anthropic/claude-sonnet-4.6";
const GOOGLE_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";

#[derive(Debug, Clone)]
pub(crate) struct ConnectionManager {
    agent_home: PathBuf,
    hermes_home: PathBuf,
    config: ConfigManager,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ConnectionsStatus {
    pub inference: InferenceStatus,
    pub telegram: TelegramStatus,
    pub google: GoogleStatus,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct InferenceStatus {
    pub profile: String,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TelegramStatus {
    pub connected: bool,
    pub home_channel: Option<String>,
    pub pending: Vec<TelegramPerson>,
    pub approved: Vec<TelegramPerson>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TelegramPerson {
    pub user_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct GoogleStatus {
    pub connected: bool,
    pub email: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InferenceApplyRequest {
    pub profile: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TelegramConnectRequest {
    pub token: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TelegramApproveRequest {
    pub code: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TelegramHomeRequest {
    pub user_id: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GoogleApplyRequest {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    pub access_token: String,
    pub redirect_uri: String,
    pub connected_email: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleMetadata {
    email: String,
}

impl ConnectionManager {
    pub(crate) fn new(
        agent_home: impl Into<PathBuf>,
        hermes_home: impl Into<PathBuf>,
        config: ConfigManager,
    ) -> Self {
        Self {
            agent_home: agent_home.into(),
            hermes_home: hermes_home.into(),
            config,
        }
    }

    pub(crate) fn status(&self) -> Result<ConnectionsStatus, AgentdError> {
        Ok(ConnectionsStatus {
            inference: self.inference_status()?,
            telegram: self.telegram_status()?,
            google: self.google_status(),
        })
    }

    pub(crate) fn inference_offer(
        &self,
        request_id: &str,
        request: InferenceApplyRequest,
    ) -> Result<HermesConfigOfferV1, AgentdError> {
        let value = match request.profile.as_str() {
            "finite_private" => json!({
                "default": required_env("FINITE_PRIVATE_MODEL")?,
                "provider": "custom",
                "base_url": required_env("FINITE_PRIVATE_BASE_URL")?,
                "api_key": "${FINITE_PRIVATE_API_KEY}",
                "api_mode": "chat_completions",
            }),
            "openrouter" => {
                let current = self.config.current_value(MODEL_CONFIG_PATH)?;
                let current_key = current
                    .as_object()
                    .filter(|value| {
                        value.get("provider").and_then(Value::as_str) == Some("openrouter")
                    })
                    .and_then(|value| value.get("api_key"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let api_key = request.api_key.or(current_key).ok_or_else(|| {
                    AgentdError::InvalidPayload("OpenRouter key is required".to_owned())
                })?;
                validate_secret("OpenRouter key", &api_key)?;
                let model = request
                    .model
                    .unwrap_or_else(|| OPENROUTER_DEFAULT_MODEL.to_owned());
                validate_model_name(&model)?;
                json!({
                    "default": model,
                    "provider": "openrouter",
                    "base_url": OPENROUTER_BASE_URL,
                    "api_key": api_key,
                    "api_mode": "chat_completions",
                })
            }
            _ => {
                return Err(AgentdError::InvalidPayload(
                    "Inference must be Finite Private or OpenRouter".to_owned(),
                ));
            }
        };
        Ok(approved_offer(request_id, MODEL_CONFIG_PATH, value))
    }

    pub(crate) fn telegram_connect_offer(
        &self,
        request_id: &str,
        request: TelegramConnectRequest,
    ) -> Result<HermesConfigOfferV1, AgentdError> {
        validate_telegram_token(&request.token)?;
        let mut value = self.telegram_config_object()?;
        value.insert("enabled".to_owned(), Value::Bool(true));
        value.insert("token".to_owned(), Value::String(request.token));
        value.insert(
            "reply_to_mode".to_owned(),
            Value::String("first".to_owned()),
        );
        value.insert(
            "gateway_restart_notification".to_owned(),
            Value::Bool(false),
        );
        value.insert("typing_indicator".to_owned(), Value::Bool(true));
        value
            .entry("extra".to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        Ok(approved_offer(
            request_id,
            TELEGRAM_CONFIG_PATH,
            Value::Object(value),
        ))
    }

    pub(crate) fn telegram_home_offer(
        &self,
        request_id: &str,
        request: TelegramHomeRequest,
    ) -> Result<HermesConfigOfferV1, AgentdError> {
        if request.user_id.is_empty()
            || request.user_id.len() > 64
            || !request.user_id.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(AgentdError::InvalidPayload(
                "Telegram chat is invalid".to_owned(),
            ));
        }
        let name = request
            .name
            .map(|value| value.trim().chars().take(128).collect::<String>())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "Telegram".to_owned());
        let mut value = self.telegram_config_object()?;
        if value
            .get("token")
            .and_then(Value::as_str)
            .is_none_or(|token| token.is_empty())
        {
            return Err(AgentdError::ConfigConflict(
                "Connect Telegram before choosing its default chat".to_owned(),
            ));
        }
        value.insert("enabled".to_owned(), Value::Bool(true));
        value.insert(
            "home_channel".to_owned(),
            json!({ "platform": "telegram", "chat_id": request.user_id, "name": name }),
        );
        Ok(approved_offer(
            request_id,
            TELEGRAM_CONFIG_PATH,
            Value::Object(value),
        ))
    }

    pub(crate) fn telegram_disconnect_offer(
        &self,
        request_id: &str,
    ) -> Result<HermesConfigOfferV1, AgentdError> {
        let mut value = self.telegram_config_object()?;
        value.insert("enabled".to_owned(), Value::Bool(false));
        value.remove("token");
        value.remove("home_channel");
        Ok(approved_offer(
            request_id,
            TELEGRAM_CONFIG_PATH,
            Value::Object(value),
        ))
    }

    pub(crate) fn approve_telegram(
        &self,
        request: TelegramApproveRequest,
    ) -> Result<(), AgentdError> {
        let code = request.code.trim().to_ascii_uppercase();
        if code.len() != 8
            || !code
                .bytes()
                .all(|byte| b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789".contains(&byte))
        {
            return Err(AgentdError::InvalidPayload(
                "Enter the eight-character code shown by your Telegram bot".to_owned(),
            ));
        }
        let status = StdCommand::new("hermes")
            .args(["pairing", "approve", "telegram", &code])
            .env("HERMES_HOME", &self.hermes_home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !status.success() {
            return Err(AgentdError::Config(
                "Telegram could not approve that code. Ask the bot for a new code and try again."
                    .to_owned(),
            ));
        }
        Ok(())
    }

    pub(crate) fn apply_google(&self, request: GoogleApplyRequest) -> Result<(), AgentdError> {
        validate_google_request(&request)?;
        let expected_scopes = self.google_scopes()?;
        let supplied_scopes = request.scopes.iter().cloned().collect::<BTreeSet<_>>();
        if supplied_scopes != expected_scopes {
            return Err(AgentdError::InvalidPayload(
                "Google did not grant the requested Finite Workspace access".to_owned(),
            ));
        }

        let client_path = self.hermes_home.join("google_client_secret.json");
        let token_path = self.hermes_home.join("google_token.json");
        let metadata_path = self.google_metadata_path();
        let previous = [
            snapshot(&client_path)?,
            snapshot(&token_path)?,
            snapshot(&metadata_path)?,
        ];
        let client = json!({
            "web": {
                "client_id": request.client_id,
                "client_secret": request.client_secret,
                "auth_uri": "https://accounts.google.com/o/oauth2/auth",
                "token_uri": GOOGLE_TOKEN_URI,
                "redirect_uris": [request.redirect_uri],
            }
        });
        let token = json!({
            "token": request.access_token,
            "refresh_token": request.refresh_token,
            "token_uri": GOOGLE_TOKEN_URI,
            "client_id": request.client_id,
            "client_secret": request.client_secret,
            "scopes": expected_scopes,
            "universe_domain": "googleapis.com",
            "account": "",
        });
        let metadata = json!({
            "email": request.connected_email.to_ascii_lowercase(),
            "connected_at_ms": now_ms(),
        });
        let result = (|| {
            atomic_private_json(&client_path, &client)?;
            atomic_private_json(&token_path, &token)?;
            atomic_private_json(&metadata_path, &metadata)?;
            self.check_google_auth()
        })();
        if let Err(error) = result {
            restore(&client_path, &previous[0])?;
            restore(&token_path, &previous[1])?;
            restore(&metadata_path, &previous[2])?;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn disconnect_google(&self) -> Result<(), AgentdError> {
        let setup = self.google_setup_script();
        if setup.is_file() {
            let _ = StdCommand::new("python")
                .arg(setup)
                .arg("--revoke")
                .env("HERMES_HOME", &self.hermes_home)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        for path in [
            self.hermes_home.join("google_token.json"),
            self.hermes_home.join("google_oauth_pending.json"),
            self.hermes_home.join("google_client_secret.json"),
            self.google_metadata_path(),
        ] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn inference_status(&self) -> Result<InferenceStatus, AgentdError> {
        let value = self.config.current_value(MODEL_CONFIG_PATH)?;
        let object = value.as_object();
        let provider = object
            .and_then(|value| value.get("provider"))
            .and_then(Value::as_str)
            .unwrap_or("custom")
            .to_owned();
        let model = object
            .and_then(|value| value.get("default"))
            .and_then(Value::as_str)
            .unwrap_or("Unknown model")
            .to_owned();
        let profile = if provider == "openrouter" {
            "openrouter"
        } else {
            "finite_private"
        };
        Ok(InferenceStatus {
            profile: profile.to_owned(),
            provider,
            model,
        })
    }

    fn telegram_status(&self) -> Result<TelegramStatus, AgentdError> {
        let value = self.config.current_value(TELEGRAM_CONFIG_PATH)?;
        let object = value.as_object();
        let connected = object
            .and_then(|value| value.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && object
                .and_then(|value| value.get("token"))
                .and_then(Value::as_str)
                .is_some_and(|token| !token.is_empty());
        let home_channel = object
            .and_then(|value| value.get("home_channel"))
            .and_then(Value::as_object)
            .and_then(|value| value.get("name").or_else(|| value.get("chat_id")))
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(TelegramStatus {
            connected,
            home_channel,
            pending: self.telegram_people("pending"),
            approved: self.telegram_people("approved"),
        })
    }

    fn google_status(&self) -> GoogleStatus {
        let email = fs::read(self.google_metadata_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<GoogleMetadata>(&bytes).ok())
            .map(|metadata| metadata.email);
        GoogleStatus {
            connected: self.hermes_home.join("google_token.json").is_file() && email.is_some(),
            email,
        }
    }

    fn telegram_config_object(&self) -> Result<Map<String, Value>, AgentdError> {
        match self.config.current_value(TELEGRAM_CONFIG_PATH)? {
            Value::Null => Ok(Map::new()),
            Value::Object(value) => Ok(value),
            _ => Err(AgentdError::ConfigConflict(
                "Telegram settings are not in a supported format".to_owned(),
            )),
        }
    }

    fn telegram_people(&self, suffix: &str) -> Vec<TelegramPerson> {
        let mut people = Vec::new();
        let mut seen = BTreeSet::new();
        for directory in [
            self.hermes_home.join("platforms/pairing"),
            self.hermes_home.join("pairing"),
        ] {
            let path = directory.join(format!("telegram-{suffix}.json"));
            let Ok(Value::Object(entries)) = fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .ok_or(())
            else {
                continue;
            };
            for (key, value) in entries {
                let Some(record) = value.as_object() else {
                    continue;
                };
                let user_id = if suffix == "approved" {
                    key
                } else {
                    record
                        .get("user_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                if user_id.is_empty() || !seen.insert(user_id.clone()) {
                    continue;
                }
                let name = record
                    .get("user_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .chars()
                    .take(128)
                    .collect();
                people.push(TelegramPerson { user_id, name });
            }
        }
        people
    }

    fn google_skill_root(&self) -> PathBuf {
        self.agent_home
            .join("managed-skills/finite/current/productivity/google-workspace-finite")
    }

    fn google_setup_script(&self) -> PathBuf {
        self.google_skill_root().join("scripts/setup.py")
    }

    fn google_metadata_path(&self) -> PathBuf {
        self.agent_home.join("agentd/google-workspace.json")
    }

    fn google_scopes(&self) -> Result<BTreeSet<String>, AgentdError> {
        let path = self
            .google_skill_root()
            .join("references/google-workspace-scopes.json");
        let scopes = serde_json::from_slice::<Vec<String>>(&fs::read(path)?)?;
        if scopes.is_empty()
            || scopes
                .iter()
                .any(|scope| !scope.starts_with("http") && scope != "openid")
        {
            return Err(AgentdError::Config(
                "The installed Google Workspace access contract is invalid".to_owned(),
            ));
        }
        Ok(scopes.into_iter().collect())
    }

    fn check_google_auth(&self) -> Result<(), AgentdError> {
        let setup = self.google_setup_script();
        if !setup.is_file() {
            return Err(AgentdError::Config(
                "Google Workspace is not installed on this agent yet".to_owned(),
            ));
        }
        let status = StdCommand::new("python")
            .arg(setup)
            .arg("--check")
            .env("HERMES_HOME", &self.hermes_home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(AgentdError::Config(
                "Google could not verify this connection. Nothing was changed.".to_owned(),
            ))
        }
    }
}

fn approved_offer(request_id: &str, path: &str, value: Value) -> HermesConfigOfferV1 {
    HermesConfigOfferV1 {
        proposal_id: request_id.to_owned(),
        path: path.to_owned(),
        policy: ConfigOfferPolicyV1::ReplaceWithConfirmation,
        approved: true,
        value,
    }
}

fn required_env(name: &str) -> Result<String, AgentdError> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AgentdError::Config(format!("{name} is not available on this agent")))
}

fn validate_secret(label: &str, value: &str) -> Result<(), AgentdError> {
    if value.trim().len() < 8 || value.len() > 16 * 1024 || value.chars().any(char::is_control) {
        return Err(AgentdError::InvalidPayload(format!("{label} is invalid")));
    }
    Ok(())
}

fn validate_model_name(value: &str) -> Result<(), AgentdError> {
    if value.trim().is_empty()
        || value.len() > 256
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(AgentdError::InvalidPayload(
            "OpenRouter model is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn validate_telegram_token(value: &str) -> Result<(), AgentdError> {
    let valid = value.len() <= 256
        && value.split_once(':').is_some_and(|(id, secret)| {
            !id.is_empty()
                && id.bytes().all(|byte| byte.is_ascii_digit())
                && secret.len() >= 30
                && secret
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        });
    if !valid {
        return Err(AgentdError::InvalidPayload(
            "Telegram bot token is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn validate_google_request(request: &GoogleApplyRequest) -> Result<(), AgentdError> {
    for (label, value) in [
        ("Google client id", request.client_id.as_str()),
        ("Google client secret", request.client_secret.as_str()),
        ("Google refresh token", request.refresh_token.as_str()),
        ("Google access token", request.access_token.as_str()),
    ] {
        validate_secret(label, value)?;
    }
    if request.connected_email.len() > 320
        || !request.connected_email.contains('@')
        || request.connected_email.chars().any(char::is_control)
    {
        return Err(AgentdError::InvalidPayload(
            "Google account email is invalid".to_owned(),
        ));
    }
    let redirect = request
        .redirect_uri
        .parse::<reqwest::Url>()
        .map_err(|_| AgentdError::InvalidPayload("Google callback is invalid".to_owned()))?;
    if !matches!(redirect.scheme(), "http" | "https") || redirect.host_str().is_none() {
        return Err(AgentdError::InvalidPayload(
            "Google callback is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn atomic_private_json(path: &Path, value: &Value) -> Result<(), AgentdError> {
    let parent = path
        .parent()
        .ok_or_else(|| AgentdError::Config("credential path has no parent".to_owned()))?;
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(&serde_json::to_vec_pretty(value)?)?;
    temporary.write_all(b"\n")?;
    temporary.as_file_mut().sync_all()?;
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o600))?;
    temporary.persist(path).map_err(|error| error.error)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn snapshot(path: &Path) -> Result<Option<Vec<u8>>, AgentdError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn restore(path: &Path, bytes: &Option<Vec<u8>>) -> Result<(), AgentdError> {
    if let Some(bytes) = bytes {
        let value = serde_json::from_slice::<Value>(bytes)?;
        atomic_private_json(path, &value)
    } else {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Ledger;

    #[test]
    fn status_never_returns_connection_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let agent_home = temp.path().join("agent");
        let hermes_home = agent_home.join("hermes-home");
        fs::create_dir_all(&hermes_home).unwrap();
        fs::write(
            hermes_home.join("config.yaml"),
            "model:\n  default: model-a\n  provider: openrouter\n  api_key: secret-key\ngateway:\n  platforms:\n    telegram:\n      enabled: true\n      token: 123456789:abcdefghijklmnopqrstuvwxyzABCDEFGH\n",
        )
        .unwrap();
        let ledger = Ledger::open(agent_home.join("agentd/ledger.sqlite3")).unwrap();
        let manager = ConnectionManager::new(
            &agent_home,
            &hermes_home,
            ConfigManager::new(hermes_home.join("config.yaml"), ledger),
        );
        let serialized = serde_json::to_string(&manager.status().unwrap()).unwrap();
        assert!(!serialized.contains("secret-key"));
        assert!(!serialized.contains("abcdefghijklmnopqrstuvwxyz"));
        assert!(serialized.contains("openrouter"));
    }

    #[test]
    fn telegram_token_validation_matches_hermes_shape() {
        assert!(validate_telegram_token("123456789:abcdefghijklmnopqrstuvwxyzABCDEFGH").is_ok());
        assert!(validate_telegram_token("not-a-token").is_err());
    }
}
