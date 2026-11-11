//! The command line. On a runner host the reader is the forced command of
//! the mover's ssh key, so the operation arrives in `SSH_ORIGINAL_COMMAND`
//! and is parsed here, word by word, with no shell:
//!
//! ```text
//! sizes   <durable_state_id> [--brain <brain id>]...
//! stream  <durable_state_id> [--brain <brain id>]... [--stage <stage id>]
//! secrets <durable_state_id> <kind>[,<kind>...]
//! owner-telegram-notice <durable_state_id> --notice <notice id>   (text on stdin)
//! ```
//!
//! The work root is never part of it: it comes from the forced command's
//! own arguments, which only the host's configuration sets.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::error::HostError;
use crate::notice::{self, NoticeId};
use crate::secrets::{self, Kind};
use crate::stage;
use crate::tree::{DurableId, Tree};

/// The longest operation accepted.
const COMMAND_MAX_BYTES: usize = 8192;
/// The most brains one operation may name.
const BRAINS_MAX: usize = 256;
const BRAIN_ID_MAX: usize = 128;

#[derive(Debug, PartialEq, Eq)]
pub enum Operation {
    Sizes {
        id: DurableId,
        owned_brains: BTreeSet<String>,
    },
    Stream {
        id: DurableId,
        owned_brains: BTreeSet<String>,
        /// The stage's id. The mover names it only once Core has stopped
        /// the runtime for its move and holds it stopped; the SimpleX carry
        /// needs it ([`crate::simplex`]).
        stage: Option<String>,
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
            "sizes|stream <id> [--brain <id>]... [--stage <id>] | secrets <id> <kind>[,<kind>...] \
             | owner-telegram-notice <id> --notice <notice id>"
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
            let mut owned_brains = BTreeSet::new();
            let mut stage = None;
            while let Some(flag) = words.next() {
                if flag == "--stage" && op == "stream" && stage.is_none() {
                    let id = words.next().ok_or_else(usage)?;
                    if !brain_id(id) {
                        return Err(HostError::Usage(
                            "a stage id is 1 to 128 of [A-Za-z0-9_.-]".into(),
                        ));
                    }
                    stage = Some(id.to_owned());
                    continue;
                }
                if flag != "--brain" {
                    return Err(usage());
                }
                let brain = words.next().ok_or_else(usage)?;
                if !brain_id(brain) || owned_brains.len() >= BRAINS_MAX {
                    return Err(HostError::Usage(
                        "a brain id is 1 to 128 of [A-Za-z0-9_.-]".into(),
                    ));
                }
                owned_brains.insert(brain.to_owned());
            }
            Ok(if op == "sizes" {
                Operation::Sizes { id, owned_brains }
            } else {
                Operation::Stream {
                    id,
                    owned_brains,
                    stage,
                }
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

fn brain_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= BRAIN_ID_MAX
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
    match operation {
        Operation::Sizes { id, owned_brains } => {
            let tree = Tree::open(work_root, id)?;
            print_json(&mut out, &stage::sizes(&tree, &owned_brains)?)
        }
        Operation::Stream {
            id,
            owned_brains,
            stage,
        } => {
            let tree = Tree::open(work_root, id)?;
            stage::stream(&tree, &owned_brains, stage.as_deref(), out).map(drop)
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
            parse_ssh("stream runtime_1 --brain b1 --brain b-2").unwrap(),
            Operation::Stream {
                id: id("runtime_1"),
                owned_brains: ["b1".to_owned(), "b-2".to_owned()].into(),
                stage: None,
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
                owned_brains: BTreeSet::new()
            }
        );
    }

    #[test]
    fn a_stream_may_name_its_stage() {
        assert_eq!(
            parse_ssh("stream runtime_1 --stage proj_a.p1.2 --brain b1").unwrap(),
            Operation::Stream {
                id: id("runtime_1"),
                owned_brains: ["b1".to_owned()].into(),
                stage: Some("proj_a.p1.2".to_owned()),
            }
        );
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in [
            "",
            "sizes",
            "sizes ../etc",
            "sizes runtime_1 extra",
            "sizes runtime_1 --brain",
            "sizes runtime_1 --brain a/b",
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
