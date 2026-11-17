//! The command line. On a runner host the reader is the forced command of
//! the mover's ssh key, so the operation arrives in `SSH_ORIGINAL_COMMAND`
//! and is parsed here, word by word, with no shell:
//!
//! ```text
//! sizes   <durable_state_id> [--carry|--leave <hex path>]...
//! stream  <durable_state_id> [--stage <stage id>] [--agent-key <hex>] [--person-key <hex>]...
//!         [--carry|--leave <hex path>]...
//! secrets <durable_state_id> <kind>[,<kind>...]
//! owner-telegram-notice <durable_state_id> --notice <notice id>   (text on stdin)
//! ```
//!
//! A key is 64 hex digits: the old agent's own, and its person's, which
//! place each brain's owner in its name file ([`crate::brains`]). A path
//! is its UTF-8 in hex: the operator's overrides ([`crate::overrides`]).
//! `--brain <word>`, which an older mover sends to `sizes` and `stream`,
//! is read and ignored: every brain is staged.
//!
//! The work root is never part of it: it comes from the forced command's
//! own arguments, which only the host's configuration sets.

use std::io::{Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::brains::{self, OwnerKeys, PERSON_KEYS_MAX};
use crate::error::HostError;
use crate::notice::{self, NoticeId};
use crate::overrides::{Override, Overrides};
use crate::secrets::{self, Kind};
use crate::stage;
use crate::tree::{DurableId, Tree};

/// The longest operation accepted: room for every key and override.
const COMMAND_MAX_BYTES: usize = 32 * 1024;
const STAGE_ID_MAX: usize = 128;

#[derive(Debug, PartialEq, Eq)]
pub enum Operation {
    Sizes {
        id: DurableId,
        overrides: Overrides,
    },
    Stream {
        id: DurableId,
        overrides: Overrides,
        /// The stage's id, which the SimpleX seal is named by.
        stage: Option<String>,
        /// The keys that place each brain's owner.
        keys: OwnerKeys,
    },
    Secrets {
        id: DurableId,
        kinds: Vec<Kind>,
    },
    OwnerTelegramNotice {
        id: DurableId,
        notice: NoticeId,
    },
}

/// Parse an operation from its words.
pub fn parse<'a>(words: impl IntoIterator<Item = &'a str>) -> Result<Operation, HostError> {
    let mut words = words.into_iter();
    let usage = || {
        HostError::Usage(
            "sizes <id> | stream <id> [--stage <id>] [--agent-key <hex>] [--person-key <hex>]... \
             [--carry|--leave <hex path>]... \
             | secrets <id> <kind>[,<kind>...] | owner-telegram-notice <id> --notice <notice id>"
                .into(),
        )
    };
    let op = words.next().ok_or_else(usage)?;
    if !["sizes", "stream", "secrets", "owner-telegram-notice"].contains(&op) {
        return Err(usage());
    }
    let id = DurableId::parse(words.next().ok_or_else(usage)?)?;
    match op {
        "sizes" | "stream" => {
            let mut stage = None;
            let mut keys = OwnerKeys::default();
            let mut overrides = Overrides::default();
            let stream = op == "stream";
            while let Some(flag) = words.next() {
                let value = words.next().ok_or_else(usage)?;
                match flag {
                    // An older mover's owned brains: every brain comes now.
                    "--brain" => {}
                    "--stage" if stream && stage.is_none() => {
                        if !stage_id(value) {
                            return Err(HostError::Usage(
                                "a stage id is 1 to 128 of [A-Za-z0-9_.-]".into(),
                            ));
                        }
                        stage = Some(value.to_owned());
                    }
                    "--agent-key" if stream && keys.agent.is_none() => {
                        keys.agent = Some(brains::parse_key(value)?);
                    }
                    "--person-key" if stream && keys.people.len() < PERSON_KEYS_MAX => {
                        keys.people.push(brains::parse_key(value)?);
                    }
                    "--carry" => overrides.push(Override::Carry, value)?,
                    "--leave" => overrides.push(Override::Leave, value)?,
                    _ => return Err(usage()),
                }
            }
            Ok(if stream {
                Operation::Stream {
                    id,
                    overrides,
                    stage,
                    keys,
                }
            } else {
                Operation::Sizes { id, overrides }
            })
        }
        "secrets" => {
            let kinds = words.next().ok_or_else(usage)?;
            if words.next().is_some() {
                return Err(usage());
            }
            let mut kinds = kinds
                .split(',')
                .map(str::parse)
                .collect::<Result<Vec<Kind>, _>>()?;
            kinds.sort();
            kinds.dedup();
            Ok(Operation::Secrets { id, kinds })
        }
        "owner-telegram-notice" => {
            if words.next() != Some("--notice") {
                return Err(usage());
            }
            let notice = NoticeId::parse(words.next().ok_or_else(usage)?)?;
            if words.next().is_some() {
                return Err(usage());
            }
            Ok(Operation::OwnerTelegramNotice { id, notice })
        }
        _ => Err(usage()),
    }
}

/// Parse the operation an ssh forced command received.
pub fn parse_ssh(original: &str) -> Result<Operation, HostError> {
    if original.len() > COMMAND_MAX_BYTES {
        return Err(HostError::Usage("the command is too long".into()));
    }
    parse(original.split_ascii_whitespace())
}

fn stage_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= STAGE_ID_MAX
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
}

/// Run one read operation against `<work_root>/kata/<id>`, writing its
/// answer to `out`.
pub fn run(work_root: &Path, operation: Operation, out: impl Write) -> Result<(), HostError> {
    run_with(work_root, None, operation, std::io::empty(), out)
}

/// Run any operation. `notice_log` is the host's notice log directory,
/// which only `owner-telegram-notice` uses; `input` is its text.
pub fn run_with(
    work_root: &Path,
    notice_log: Option<&Path>,
    operation: Operation,
    input: impl Read,
    mut out: impl Write,
) -> Result<(), HostError> {
    // The seal's write capabilities stay lowered but around the seal.
    crate::simplex::lower_writes()?;
    match operation {
        Operation::Sizes { id, overrides } => {
            let tree = Tree::open(work_root, id)?;
            print_json(&mut out, &stage::sizes(&tree, &overrides)?)
        }
        Operation::Stream {
            id,
            overrides,
            stage,
            keys,
        } => {
            let tree = Tree::open(work_root, id)?;
            stage::stream(&tree, &overrides, &keys, stage.as_deref(), out).map(drop)
        }
        Operation::Secrets { id, kinds } => {
            let tree = Tree::open(work_root, id)?;
            print_json(&mut out, &secrets::read(&tree, &kinds)?)
        }
        Operation::OwnerTelegramNotice { id, notice } => {
            let log = notice_log.ok_or_else(|| {
                HostError::Usage("this host keeps no notice log (--notice-log)".into())
            })?;
            let text = notice::read_text(input)?;
            let tree = Tree::open(work_root, id)?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_secs());
            let answer = notice::send(&tree, log, notice::TELEGRAM_API, &notice, &text, now)?;
            print_json(&mut out, &answer)
        }
    }
}

/// One line of JSON.
fn print_json(out: &mut impl Write, value: &impl Serialize) -> Result<(), HostError> {
    let write = |error: std::io::Error| HostError::Write(error.to_string());
    serde_json::to_writer(&mut *out, value).map_err(|error| HostError::Write(error.to_string()))?;
    out.write_all(b"\n").map_err(write)?;
    out.flush().map_err(write)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> DurableId {
        DurableId::parse(value).unwrap()
    }

    #[test]
    fn operations_parse_word_by_word() {
        assert_eq!(
            parse_ssh("stream runtime_1").unwrap(),
            Operation::Stream {
                id: id("runtime_1"),
                overrides: Overrides::default(),
                stage: None,
                keys: OwnerKeys::default(),
            }
        );
        assert_eq!(
            parse_ssh("secrets runtime_1 openrouter_key,telegram_bot_token").unwrap(),
            Operation::Secrets {
                id: id("runtime_1"),
                kinds: vec![Kind::TelegramBotToken, Kind::OpenrouterKey],
            }
        );
        assert_eq!(
            parse_ssh("sizes runtime_1").unwrap(),
            Operation::Sizes {
                id: id("runtime_1"),
                overrides: Overrides::default(),
            }
        );
    }

    #[test]
    fn a_stream_may_name_its_stage_and_the_keys_that_place_brains() {
        let (agent, person) = ("0a".repeat(32), "0b".repeat(32));
        assert_eq!(
            parse_ssh(&format!(
                "stream runtime_1 --stage proj_a.p1.2 --agent-key {agent} \
                 --person-key {person} --person-key {agent}"
            ))
            .unwrap(),
            Operation::Stream {
                id: id("runtime_1"),
                overrides: Overrides::default(),
                stage: Some("proj_a.p1.2".to_owned()),
                keys: OwnerKeys {
                    agent: Some([10; 32]),
                    people: vec![[11; 32], [10; 32]],
                },
            }
        );
        let too_many = format!("--person-key {person} ").repeat(PERSON_KEYS_MAX + 1);
        for bad in [
            format!("stream runtime_1 --agent-key {agent} --agent-key {agent}"),
            format!("stream runtime_1 --agent-key {}", &agent[2..]),
            "stream runtime_1 --person-key npub1xyz".to_owned(),
            format!("stream runtime_1 {too_many}"),
            format!("sizes runtime_1 --agent-key {agent}"),
        ] {
            assert!(parse_ssh(&bad).is_err(), "{bad}");
        }
    }

    /// An older mover names each owned brain, with its name in hex: read
    /// and ignored, so a host updated first still serves it.
    #[test]
    fn an_older_movers_brains_are_read_and_ignored() {
        let name = hex::encode("Notes & plans: été");
        assert_eq!(
            parse_ssh(&format!(
                "stream runtime_1 --stage s1 --brain b1:{name} --brain b-2"
            ))
            .unwrap(),
            Operation::Stream {
                id: id("runtime_1"),
                overrides: Overrides::default(),
                stage: Some("s1".to_owned()),
                keys: OwnerKeys::default(),
            }
        );
        assert_eq!(
            parse_ssh("sizes runtime_1 --brain b1").unwrap(),
            Operation::Sizes {
                id: id("runtime_1"),
                overrides: Overrides::default(),
            }
        );
    }

    #[test]
    fn sizes_and_stream_take_the_operators_overrides() {
        let (carry, leave) = (
            hex::encode("agent/hermes-home/sessions"),
            hex::encode("workspace/big"),
        );
        let mut overrides = Overrides::default();
        overrides.push(Override::Carry, &carry).unwrap();
        overrides.push(Override::Leave, &leave).unwrap();
        assert_eq!(
            parse_ssh(&format!("sizes runtime_1 --carry {carry} --leave {leave}")).unwrap(),
            Operation::Sizes {
                id: id("runtime_1"),
                overrides: overrides.clone(),
            }
        );
        assert!(matches!(
            parse_ssh(&format!("stream runtime_1 --stage s1 --carry {carry} --leave {leave}")).unwrap(),
            Operation::Stream { overrides: parsed, .. } if parsed == overrides
        ));
        for bad in [hex::encode("../etc"), hex::encode("/etc"), "zz".to_owned()] {
            assert!(
                parse_ssh(&format!("stream runtime_1 --carry {bad}")).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in [
            "",
            "sizes",
            "sizes ../etc",
            "sizes runtime_1 extra",
            "sizes runtime_1 --brain",
            "stream runtime_1;rm -rf /",
            "stream runtime_1 --stage",
            "stream runtime_1 --stage a/b",
            "stream runtime_1 --stage s1 --stage s2",
            "sizes runtime_1 --stage s1",
            "secrets runtime_1",
            "secrets runtime_1 google_token",
            "secrets runtime_1 nostr_nsec extra",
            "cat runtime_1",
        ] {
            assert!(parse_ssh(bad).is_err(), "{bad:?}");
        }
        assert!(parse_ssh(&format!("sizes {}", "a".repeat(COMMAND_MAX_BYTES))).is_err());
    }
}
