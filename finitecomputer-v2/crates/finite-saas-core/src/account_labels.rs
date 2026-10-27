//! Account display names from WorkOS and private per-viewer key aliases.
//!
//! Neither value is identity evidence. A WorkOS name only labels an account
//! that Core already describes; an alias only labels a key for the one account
//! that wrote it. Neither grants access, ownership, membership or roles.

use crate::brain_identity::{MAX_DISPLAY_NAME_BYTES, npub_for_hex};
use bech32::Hrp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Longest single WorkOS name part Core accepts, in characters.
pub const MAX_NAME_PART_CHARS: usize = 100;
/// Longest alias, in characters and in UTF-8 bytes.
pub const MAX_ALIAS_CHARS: usize = 64;
pub const MAX_ALIAS_BYTES: usize = 256;
/// Aliases one account may keep.
pub const MAX_ALIASES_PER_ACCOUNT: i64 = 1_000;
/// Keys one alias lookup may name.
pub const MAX_ALIAS_LOOKUP_KEYS: usize = 100;
pub const MAX_ALIAS_LOOKUP_BODY_BYTES: usize = 16 * 1024;
pub const MAX_ALIAS_WRITE_BODY_BYTES: usize = 2 * 1024;

/// Trim, collapse inner whitespace to single spaces and refuse text that
/// could hide or reorder what a reader sees: control characters and
/// bidirectional overrides. Returns `None` for blank or unusable text.
fn display_text(value: &str, max_chars: usize, max_bytes: usize) -> Option<String> {
    if value.chars().any(|c| c.is_control() || bidi_control(c)) {
        return None;
    }
    let text = value.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty() && text.chars().count() <= max_chars && text.len() <= max_bytes)
        .then_some(text)
}

fn bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// The account name to store for one fetched WorkOS user.
///
/// `first_name` and `last_name` are optional in WorkOS. A missing, null or
/// blank part is skipped. Any part that is not a string, is longer than
/// [`MAX_NAME_PART_CHARS`], or contains control or bidi characters makes the
/// whole name unusable. The parts are joined by one space. `None` means
/// "WorkOS gives no usable name": Core clears any stored name, so it never
/// shows a name WorkOS no longer has. Core never derives a name from email.
pub fn workos_display_name(first: Option<&Value>, last: Option<&Value>) -> Option<String> {
    let mut parts = Vec::new();
    for part in [first, last] {
        match part {
            None | Some(Value::Null) => {}
            Some(Value::String(text)) if text.trim().is_empty() => {}
            Some(Value::String(text)) => parts.push(display_text(
                text,
                MAX_NAME_PART_CHARS,
                MAX_DISPLAY_NAME_BYTES,
            )?),
            Some(_) => return None,
        }
    }
    let name = parts.join(" ");
    (!name.is_empty() && name.len() <= MAX_DISPLAY_NAME_BYTES).then_some(name)
}

/// A private alias as typed: trimmed, inner whitespace collapsed, 1 to
/// [`MAX_ALIAS_CHARS`] characters, no control or bidi characters.
pub fn normalize_alias(value: &str) -> Option<String> {
    display_text(value, MAX_ALIAS_CHARS, MAX_ALIAS_BYTES)
}

/// Canonical lowercase hex for a Nostr public key given as `npub1…` or as
/// 64 hex characters. The 32 bytes must be a valid x-only secp256k1 key, so a
/// well-shaped string that is not a key is refused.
pub fn canonical_public_key(value: &str) -> Option<String> {
    let value = value.trim();
    let bytes: Vec<u8> = if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        (0..32)
            .map(|index| u8::from_str_radix(&value[2 * index..2 * index + 2], 16))
            .collect::<Result<_, _>>()
            .ok()?
    } else {
        let (hrp, data) = bech32::decode(value).ok()?;
        if hrp != Hrp::parse("npub").ok()? || value.to_ascii_lowercase() != value {
            return None;
        }
        data
    };
    secp256k1::XOnlyPublicKey::from_slice(&bytes).ok()?;
    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyAlias {
    pub public_key_hex: String,
    pub npub: String,
    pub alias: String,
    pub updated_at: String,
}

impl KeyAlias {
    pub(crate) fn new(public_key_hex: String, alias: String, updated_at: String) -> Self {
        Self {
            npub: npub_for_hex(&public_key_hex).unwrap_or_default(),
            public_key_hex,
            alias,
            updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyAliasLookupRequest {
    pub public_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetKeyAliasRequest {
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyAliasesResponse {
    pub aliases: Vec<KeyAlias>,
}

#[cfg(test)]
mod tests;
