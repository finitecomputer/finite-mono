//! `secrets`: the three values a person may carry to finite.chat's vault,
//! read for the mover's in-memory hand-off and printed once to stdout.
//! Nothing here is logged, written or put in an error: a file that can't
//! be read or parsed just reports the kind as absent.

use std::str::FromStr;

use serde::Serialize;
use serde_yaml::Value;
use zeroize::Zeroizing;

use crate::config_copy::CONFIG_MAX_BYTES;
use crate::error::HostError;
use crate::tree::Tree;

const ENV_MAX_BYTES: u64 = 1024 * 1024;
const IDENTITY_MAX_BYTES: u64 = 64 * 1024;
/// The longest value handed off (`validate_secret`,
/// finite-agentd/src/connections.rs:890-895).
const VALUE_MAX_BYTES: usize = 16 * 1024;
const VALUE_MIN_BYTES: usize = 8;

const HERMES_ENV: [&str; 3] = ["agent", "hermes-home", ".env"];
const HERMES_CONFIG: [&str; 3] = ["agent", "hermes-home", "config.yaml"];
const IDENTITY: [&str; 3] = ["agent", "identity", "identity.json"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    TelegramBotToken,
    OpenrouterKey,
    NostrNsec,
}

impl FromStr for Kind {
    type Err = HostError;

    fn from_str(value: &str) -> Result<Self, HostError> {
        match value {
            "telegram_bot_token" => Ok(Kind::TelegramBotToken),
            "openrouter_key" => Ok(Kind::OpenrouterKey),
            "nostr_nsec" => Ok(Kind::NostrNsec),
            _ => Err(HostError::Usage(
                "a secret kind is telegram_bot_token, openrouter_key or nostr_nsec".into(),
            )),
        }
    }
}

#[derive(Serialize)]
pub struct Item {
    pub kind: Kind,
    #[serde(serialize_with = "reveal")]
    pub value: Zeroizing<String>,
}

fn reveal<S: serde::Serializer>(
    value: &Zeroizing<String>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(value)
}

#[derive(Serialize)]
pub struct Secrets {
    pub items: Vec<Item>,
    /// Kinds asked for that this agent does not have, or that could not be
    /// read.
    pub absent: Vec<Kind>,
}

/// Read `kinds` from a stopped agent's tree.
pub fn read(tree: &Tree, kinds: &[Kind]) -> Result<Secrets, HostError> {
    tree.check_writer_lease()?;
    let mut secrets = Secrets {
        items: Vec::new(),
        absent: Vec::new(),
    };
    for kind in kinds {
        let value = match kind {
            Kind::TelegramBotToken => telegram(tree)?,
            Kind::OpenrouterKey => openrouter(tree)?,
            Kind::NostrNsec => nostr(tree)?,
        };
        match value.filter(|value| plausible(value)) {
            Some(value) => secrets.items.push(Item { kind: *kind, value }),
            None => secrets.absent.push(*kind),
        }
    }
    Ok(secrets)
}

/// `gateway.platforms.telegram.token` in `config.yaml`
/// (finite-agentd/src/connections.rs:399-402), or `TELEGRAM_BOT_TOKEN` in
/// the Hermes `.env`.
fn telegram(tree: &Tree) -> Result<Option<Zeroizing<String>>, HostError> {
    let from_config = config(tree)?.and_then(|config| {
        let token = config
            .get("gateway")?
            .get("platforms")?
            .get("telegram")?
            .get("token")?
            .as_str()?;
        Some(Zeroizing::new(token.trim().to_owned()))
    });
    match from_config {
        Some(token) => Ok(Some(token)),
        None => dotenv(tree, "TELEGRAM_BOT_TOKEN"),
    }
}

/// The bot token of a connected Telegram, for the owner notice: none when
/// the agent switched Telegram off (`enabled: false`) or has no plausible
/// token.
pub(crate) fn connected_telegram(tree: &Tree) -> Result<Option<Zeroizing<String>>, HostError> {
    let switched_off = config(tree)?.is_some_and(|config| {
        config
            .get("gateway")
            .and_then(|gateway| gateway.get("platforms"))
            .and_then(|platforms| platforms.get("telegram"))
            .and_then(|telegram| telegram.get("enabled"))
            .and_then(Value::as_bool)
            == Some(false)
    });
    if switched_off {
        return Ok(None);
    }
    Ok(telegram(tree)?.filter(|token| plausible(token)))
}

/// `OPENROUTER_API_KEY` in the Hermes `.env`
/// (finite-agentd/src/connections.rs:165-181), or the legacy `model.api_key`
/// when the model provider is OpenRouter (connections.rs:183-188).
fn openrouter(tree: &Tree) -> Result<Option<Zeroizing<String>>, HostError> {
    if let Some(key) = dotenv(tree, "OPENROUTER_API_KEY")? {
        return Ok(Some(key));
    }
    Ok(config(tree)?.and_then(|config| {
        let model = config.get("model")?;
        if model.get("provider")?.as_str()? != "openrouter" {
            return None;
        }
        Some(Zeroizing::new(
            model.get("api_key")?.as_str()?.trim().to_owned(),
        ))
    }))
}

/// The agent's Nostr key, `secret_hex` in `identity.json`
/// (finite-identity/src/identity.rs:24-35), as an `nsec`.
fn nostr(tree: &Tree) -> Result<Option<Zeroizing<String>>, HostError> {
    let Some(bytes) = tree
        .read_small(&IDENTITY, IDENTITY_MAX_BYTES)?
        .map(Zeroizing::new)
    else {
        return Ok(None);
    };
    let Ok(identity) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Ok(None);
    };
    if identity.get("kind").and_then(serde_json::Value::as_str) != Some("nostr-secp256k1") {
        return Ok(None);
    }
    let Some(hex) = identity
        .get("secret_hex")
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(None);
    };
    let mut secret = Zeroizing::new([0u8; 32]);
    if hex::decode_to_slice(hex, &mut secret[..]).is_err() {
        return Ok(None);
    }
    let hrp = bech32::Hrp::parse("nsec").map_err(|_| HostError::Write("nsec prefix".into()))?;
    Ok(bech32::encode::<bech32::Bech32>(hrp, &secret[..])
        .ok()
        .map(Zeroizing::new))
}

/// The parsed `config.yaml`. The process prints its answer and exits, so
/// the parsed copy is not cleared; the bytes read are.
fn config(tree: &Tree) -> Result<Option<Value>, HostError> {
    let Some(bytes) = tree
        .read_small(&HERMES_CONFIG, CONFIG_MAX_BYTES)?
        .map(Zeroizing::new)
    else {
        return Ok(None);
    };
    Ok(serde_yaml::from_slice::<Value>(&bytes).ok())
}

/// `name=value` in the Hermes `.env`, as finite-agentd reads it
/// (`dotenv_assignment`, finite-agentd/src/connections.rs:843-865).
fn dotenv(tree: &Tree, name: &str) -> Result<Option<Zeroizing<String>>, HostError> {
    let Some(bytes) = tree
        .read_small(&HERMES_ENV, ENV_MAX_BYTES)?
        .map(Zeroizing::new)
    else {
        return Ok(None);
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(None);
    };
    Ok(text
        .lines()
        .find_map(|line| assignment(line, name))
        .map(Zeroizing::new))
}

fn assignment(line: &str, name: &str) -> Option<String> {
    let line = line.trim_start();
    let line = line.strip_prefix("export ").map_or(line, str::trim_start);
    let (key, value) = line.split_once('=')?;
    if key.trim() != name {
        return None;
    }
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return Some(
            value[1..value.len() - 1]
                .replace("\\'", "'")
                .replace("\\\\", "\\"),
        );
    }
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return Some(value[1..value.len() - 1].to_owned());
    }
    Some(value.to_owned())
}

/// A value worth handing off: the bounds finite-agentd puts on a saved
/// secret, and no whitespace inside.
fn plausible(value: &str) -> bool {
    (VALUE_MIN_BYTES..=VALUE_MAX_BYTES).contains(&value.len())
        && !value.chars().any(|c| c.is_control() || c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_lines_read_as_agentd_writes_them() {
        assert_eq!(
            assignment("OPENROUTER_API_KEY='a\\'b'", "OPENROUTER_API_KEY").as_deref(),
            Some("a'b")
        );
        assert_eq!(assignment("export X=\"y\"", "X").as_deref(), Some("y"));
        assert_eq!(assignment("  X = plain ", "X").as_deref(), Some("plain"));
        assert_eq!(assignment("XY=1", "X"), None);
    }

    #[test]
    fn kinds_parse_by_their_vault_names() {
        assert_eq!("nostr_nsec".parse::<Kind>().unwrap(), Kind::NostrNsec);
        assert!("google_token".parse::<Kind>().is_err());
    }
}
