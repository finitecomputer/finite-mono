//! The mover's half of fragment's wire for the move (FIN-463): the shapes
//! of `fragment_proto::migrations` the mover sends and reads, field for
//! field (fragment `crates/proto/src/migrations.rs`, `docs/migrations.md`).
//! finite-mono can't depend on fragment's crates, so they are copied here;
//! fragment's `cargo xtask wire` locks the other side.
//!
//! Two of them are open on fragment's wire (`x-fragment-open`): a command
//! kind or a move state the mover doesn't know is read, never refused. It
//! leaves such a command in the mailbox, done by no one.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub use crate::stage::Phase;

/// An old agent's id: Core's opaque project id (`valid_old`).
pub const OLD_ID_MAX_BYTES: usize = 64;
/// A short text the mover sends (a snapshot's id, a step, a receipt, an
/// email) and a failure's why, in bytes.
pub const TEXT_MAX_BYTES: usize = 512;
pub const ROSTER_PEOPLE_MAX: usize = 2_000;
pub const ROSTER_AGENTS_MAX: usize = 16;
pub const ROSTER_LIST_MAX: usize = 256;

/// Whether `id` is an old agent's id.
pub fn valid_old(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= OLD_ID_MAX_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// `text` cut to `TEXT_MAX_BYTES` on a character boundary.
pub fn short(text: &str) -> String {
    if text.len() <= TEXT_MAX_BYTES {
        return text.to_string();
    }
    let mut end = TEXT_MAX_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// `PUT /api/migrations/mover/roster`: every live old agent and its owner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Roster {
    pub snapshot: String,
    pub observed_ms: i64,
    pub people: Vec<RosterPerson>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RosterPerson {
    pub workos_id: String,
    pub email: String,
    pub agents: Vec<RosterAgent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RosterAgent {
    pub old: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active_ms: Option<i64>,
    pub sizes: Sizes,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub brains: Vec<OldBrain>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sites: Vec<OldSite>,
    pub has: Has,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sizes {
    pub cache: u64,
    pub history: u64,
    pub work: u64,
    #[serde(rename = "move")]
    pub staged: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OldBrain {
    pub id: String,
    pub title: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OldSite {
    pub name: String,
    pub public: bool,
}

/// Which secrets it holds (never their values).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Has {
    pub telegram: bool,
    pub openrouter: bool,
    pub nostr: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterTaken {
    pub bound: u32,
    pub withdrawn: u32,
    pub replay: bool,
}

/// `GET /api/migrations/mover/mailbox`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mailbox {
    pub commands: Vec<Command>,
}

/// One thing to do to one old agent, flat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Command {
    /// `<old>:<attempt>:<step>`.
    pub id: String,
    pub old: String,
    pub attempt: u32,
    pub kind: CommandKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, rename = "as", skip_serializing_if = "Option::is_none")]
    pub mark: Option<Mark>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<SecretKind>,
}

/// Open on fragment's wire: a kind the mover doesn't know is `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    StopAndStage,
    Retire,
    Restart,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    Migrated,
    Archived,
}

impl Mark {
    pub fn as_str(self) -> &'static str {
        match self {
            Mark::Migrated => "migrated",
            Mark::Archived => "archived",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    TelegramBotToken,
    OpenrouterKey,
    NostrNsec,
    #[serde(other)]
    Other,
}

impl SecretKind {
    /// Its name on fragment's wire and in the host reader's `secrets`.
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            SecretKind::TelegramBotToken => Some("telegram_bot_token"),
            SecretKind::OpenrouterKey => Some("openrouter_key"),
            SecretKind::NostrNsec => Some("nostr_nsec"),
            SecretKind::Other => None,
        }
    }
}

/// A secret's value, held in memory alone: cleared when dropped, and never
/// shown by `Debug`. It is read from the host reader's answer and written
/// once, into the hand-off's body.
pub struct SecretValue(Zeroizing<String>);

impl SecretValue {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn reveal(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue(..)")
    }
}

impl Serialize for SecretValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SecretValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(SecretValue::new)
    }
}

/// One of an old agent's secrets: what the host reader's `secrets`
/// answers, and what a hand-off carries.
#[derive(Debug, Serialize, Deserialize)]
pub struct HandoffItem {
    pub kind: SecretKind,
    pub value: SecretValue,
}

/// `POST /api/migrations/mover/agents/{old}/secrets`: the old agent's
/// secrets its `stop_and_stage` names, read once it is stopped, for its
/// person's Vault on finite.chat (F11).
#[derive(Debug, Serialize)]
pub struct Handoff<'a> {
    pub command: &'a str,
    pub items: &'a [HandoffItem],
}

/// Its answer: the kinds the person's Vault took.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HandedOff {
    pub handed: Vec<SecretKind>,
}

/// `POST /api/migrations/mover/agents/{old}/report`, flat: its `kind` and
/// that kind's fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub command: String,
    pub seq: u64,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<String>,
}

impl Report {
    pub fn of(command: &str, seq: u64, kind: &str) -> Report {
        Report {
            command: command.into(),
            seq,
            kind: kind.into(),
            bytes: None,
            files: None,
            manifest_sha256: None,
            step: None,
            why: None,
            receipt: None,
        }
    }
}

/// A report's answer. `state` is open on fragment's wire, so it is read as
/// text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reported {
    pub state: String,
    pub attempt: u32,
    pub taken: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_of_a_kind_the_mover_does_not_know_is_read() {
        let command: Command = serde_json::from_str(
            r#"{"id":"proj_1:1:shrink","old":"proj_1","attempt":1,"kind":"shrink","secrets":["telegram_bot_token","a_new_secret"]}"#,
        )
        .unwrap();
        assert_eq!(command.kind, CommandKind::Other);
        assert_eq!(
            command.secrets,
            [SecretKind::TelegramBotToken, SecretKind::Other]
        );
        let stage: Command = serde_json::from_str(
            r#"{"id":"proj_1:2:stop_and_stage","old":"proj_1","attempt":2,"kind":"stop_and_stage","phase":"p1","stage":"proj_1.p1.2"}"#,
        )
        .unwrap();
        assert_eq!(
            (stage.kind, stage.phase),
            (CommandKind::StopAndStage, Some(Phase::P1))
        );
        let retire: Command = serde_json::from_str(
            r#"{"id":"proj_1:2:retire_migrated","old":"proj_1","attempt":2,"kind":"retire","as":"migrated"}"#,
        )
        .unwrap();
        assert_eq!(retire.mark, Some(Mark::Migrated));
        let reported: Reported =
            serde_json::from_str(r#"{"state":"a_new_state","attempt":2,"taken":true}"#).unwrap();
        assert_eq!(reported.state, "a_new_state");
    }

    #[test]
    fn sizes_and_reports_are_fragments_shapes() {
        assert_eq!(
            serde_json::to_value(Sizes {
                cache: 1,
                history: 2,
                work: 3,
                staged: 5,
                total: 6
            })
            .unwrap(),
            serde_json::json!({"cache": 1, "history": 2, "work": 3, "move": 5, "total": 6})
        );
        let report = Report {
            bytes: Some(10),
            files: Some(2),
            ..Report::of("proj_1:1:stop_and_stage", 7, "progress")
        };
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::json!({"command": "proj_1:1:stop_and_stage", "seq": 7, "kind": "progress", "bytes": 10, "files": 2})
        );
        assert_eq!(short(&"é".repeat(400)).len(), 512);
    }

    #[test]
    fn a_handoff_is_fragments_shape_and_never_shows_a_value() {
        let items: Vec<HandoffItem> = serde_json::from_str(
            r#"[{"kind":"telegram_bot_token","value":"123:SENTINEL"},{"kind":"nostr_nsec","value":"nsec1x"}]"#,
        )
        .unwrap();
        let handoff = Handoff {
            command: "proj_1:1:stop_and_stage",
            items: &items,
        };
        assert_eq!(
            serde_json::to_value(&handoff).unwrap(),
            serde_json::json!({"command": "proj_1:1:stop_and_stage", "items": [{"kind": "telegram_bot_token", "value": "123:SENTINEL"}, {"kind": "nostr_nsec", "value": "nsec1x"}]})
        );
        assert!(!format!("{handoff:?}").contains("SENTINEL"));
        let handed: HandedOff =
            serde_json::from_str(r#"{"handed":["nostr_nsec","a_new_secret"]}"#).unwrap();
        assert_eq!(handed.handed, [SecretKind::NostrNsec, SecretKind::Other]);
        assert_eq!(SecretKind::Other.as_str(), None);
    }
}
