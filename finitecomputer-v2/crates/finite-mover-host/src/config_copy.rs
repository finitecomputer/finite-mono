//! The allow-listed copy of `agent/hermes-home/config.yaml`. The original
//! holds the Telegram token and model keys and never leaves the host; the
//! copy keeps only what an import uses (export.py `config_settings`,
//! 676-699, plus whether Telegram was on):
//!
//! ```yaml
//! timezone: <string>
//! agent: { personality: <string> }
//! model: { default: <string> }
//! gateway: { platforms: { telegram: { enabled: <bool> } } }
//! ```
//!
//! A value that looks like a credential is dropped, and a copy that still
//! looks like one is not made at all.

use serde_yaml::{Mapping, Value};

use crate::shapes::holds_secret;

/// The largest `config.yaml` read; a bigger one gets no copy.
pub const CONFIG_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// The longest string value kept.
const VALUE_MAX_BYTES: usize = 64 * 1024;

/// The copy's bytes, or `None` when the original can't be parsed or no
/// clean copy can be made. Parse errors are never printed: they can quote
/// the file.
pub fn public_config(original: &[u8]) -> Option<Vec<u8>> {
    let document: Value = serde_yaml::from_slice(original).ok()?;
    let mut copy = Mapping::new();
    if let Some(timezone) = text(document.get("timezone")) {
        copy.insert("timezone".into(), timezone);
    }
    let personality = document
        .get("agent")
        .and_then(|agent| agent.get("personality"))
        .or_else(|| document.get("personality"));
    if let Some(personality) = text(personality) {
        copy.insert("agent".into(), nested("personality", personality));
    }
    let model = document.get("model");
    let default_model = model
        .and_then(|model| model.get("default"))
        .or(model.filter(|model| model.is_string()));
    if let Some(default_model) = text(default_model) {
        copy.insert("model".into(), nested("default", default_model));
    }
    let telegram = document
        .get("gateway")
        .and_then(|gateway| gateway.get("platforms"))
        .and_then(|platforms| platforms.get("telegram"))
        .and_then(|telegram| telegram.get("enabled"))
        .and_then(Value::as_bool);
    if let Some(enabled) = telegram {
        let telegram = nested("enabled", Value::Bool(enabled));
        let platforms = nested("telegram", telegram);
        copy.insert("gateway".into(), nested("platforms", platforms));
    }
    let bytes = serde_yaml::to_string(&Value::Mapping(copy))
        .ok()?
        .into_bytes();
    (!holds_secret(&bytes)).then_some(bytes)
}

fn nested(key: &str, value: Value) -> Value {
    let mut mapping = Mapping::new();
    mapping.insert(key.into(), value);
    Value::Mapping(mapping)
}

/// A string value that is short and no credential.
fn text(value: Option<&Value>) -> Option<Value> {
    let value = value?.as_str()?;
    (value.len() <= VALUE_MAX_BYTES && !holds_secret(value.as_bytes()))
        .then(|| Value::String(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TELEGRAM: &str = "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw1";
    const OPENROUTER: &str = "sk-or-v1-0123456789abcdef0123456789abcdef";

    fn original(personality: &str) -> String {
        format!(
            "timezone: America/Chicago\n\
             agent:\n  personality: {personality}\n  max_turns: 90\n\
             model:\n  default: glm-5-3-flash\n  provider: openrouter\n  api_key: {OPENROUTER}\n\
             gateway:\n  platforms:\n    telegram:\n      enabled: true\n      token: '{TELEGRAM}'\n\
             mcp_servers:\n  github:\n    headers:\n      Authorization: Bearer ghp_abcdefghij0123456789\n"
        )
    }

    fn parsed(copy: &[u8]) -> Value {
        serde_yaml::from_slice(copy).unwrap()
    }

    #[test]
    fn the_copy_holds_only_the_allow_list() {
        let copy = public_config(original("kind and brief").as_bytes()).unwrap();
        let text = String::from_utf8(copy.clone()).unwrap();
        assert!(!text.contains(TELEGRAM) && !text.contains(OPENROUTER) && !text.contains("ghp_"));
        let expected: Value = serde_yaml::from_str(
            "timezone: America/Chicago\n\
             agent: {personality: kind and brief}\n\
             model: {default: glm-5-3-flash}\n\
             gateway: {platforms: {telegram: {enabled: true}}}\n",
        )
        .unwrap();
        assert_eq!(parsed(&copy), expected);
    }

    #[test]
    fn a_value_shaped_like_a_credential_is_dropped() {
        let copy =
            public_config(original("use sk-proj4abcdefghijklmnop always").as_bytes()).unwrap();
        assert!(parsed(&copy).get("agent").is_none());
        assert!(!String::from_utf8(copy).unwrap().contains("sk-proj"));
    }

    #[test]
    fn a_scalar_model_and_a_top_level_personality_are_read() {
        let copy = public_config(b"model: gpt-x\npersonality: terse\n").unwrap();
        let expected: Value =
            serde_yaml::from_str("agent: {personality: terse}\nmodel: {default: gpt-x}\n").unwrap();
        assert_eq!(parsed(&copy), expected);
    }

    #[test]
    fn an_unparseable_original_gets_no_copy() {
        assert_eq!(public_config(b"model: [unclosed\n  token: x"), None);
    }
}
