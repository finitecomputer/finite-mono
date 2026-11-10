//! `owner-telegram-notice`: staff's one message to an old agent's owner,
//! sent by the agent's own Telegram bot, so that people who use only
//! Telegram hear about the move (FIN-463). The text arrives on stdin.
//!
//! - **The token stays on this host.** It is read from the agent's own
//!   config and handed to curl on stdin, inside curl's config, never on a
//!   command line, in a log or in an answer. The reader gives up every
//!   capability before curl runs, so curl can read nothing the account
//!   couldn't.
//! - **Only the owner is messaged.** The chat is the single user the
//!   agent's Telegram pairing approved, a private chat. No pairing, several
//!   approved users, or a group id sends nothing.
//! - **Plain text**, at most Telegram's 4,096 characters, no Markdown.
//! - **One message per notice id.** Each agent's log (`<log>/<id>.jsonl`,
//!   which only this account writes) records every notice's id, time and
//!   result, never its text. An id tried once is never sent again, even
//!   if that attempt's result is unknown, and an agent gets at most
//!   [`NOTICES_PER_DAY`] attempts in any 24 hours.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

use rustix::fs::{FlockOperation, flock};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::HostError;
use crate::pairing::{Approved, approved};
use crate::secrets;
use crate::tree::{DurableId, Tree};

/// Telegram's own limit on a message, counted here in UTF-16 units, the
/// strictest way to read it: an emoji counts twice.
pub const TEXT_MAX_CHARS: usize = 4096;
/// Attempts per agent in any 24 hours.
pub const NOTICES_PER_DAY: usize = 3;
pub const TELEGRAM_API: &str = "https://api.telegram.org";
const DAY_SECONDS: u64 = 24 * 60 * 60;
const NOTICE_ID_MAX: usize = 64;
const TEXT_MAX_BYTES: u64 = 4 * TEXT_MAX_CHARS as u64;
const LOG_MAX_BYTES: u64 = 1024 * 1024;
const ANSWER_MAX_BYTES: u64 = 64 * 1024;
/// curl, pinned by the package build; `curl` from PATH elsewhere (tests).
const CURL: &str = match option_env!("FINITE_MOVER_CURL") {
    Some(path) => path,
    None => "curl",
};

/// A notice's id, chosen by staff: 1 to 64 of `[A-Za-z0-9._-]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NoticeId(String);

impl NoticeId {
    pub fn parse(value: &str) -> Result<Self, HostError> {
        let valid = !value.is_empty()
            && value.len() <= NOTICE_ID_MAX
            && !value.starts_with('.')
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte));
        if !valid {
            return Err(HostError::Usage(
                "a notice id is 1 to 64 of [A-Za-z0-9._-]".into(),
            ));
        }
        Ok(Self(value.to_owned()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Telegram took the message.
    Sent,
    /// No connected Telegram, or no approved user: nothing sent.
    NoTelegram,
    /// The pairing approved several users, or one that isn't a private
    /// chat: nothing sent.
    NoSingleOwner,
    /// This agent's attempts for the day are used up: nothing sent.
    RateLimited,
    /// Telegram refused it (`telegram_error` holds its code).
    Refused,
    /// No answer from Telegram: it may or may not have arrived.
    Failed,
    /// An attempt began and its result was never recorded.
    Sending,
}

impl Outcome {
    /// Whether this record is a send attempt, which uses up its id.
    fn attempted(self) -> bool {
        matches!(self, Self::Sending)
    }
}

/// One line of an agent's notice log: never the text, the token or the chat.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    notice: NoticeId,
    at: u64,
    result: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    telegram_error: Option<u16>,
}

/// The answer printed to the caller.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Answer {
    pub notice: NoticeId,
    pub result: Outcome,
    /// False when this id was tried before and nothing was sent now.
    pub first: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telegram_error: Option<u16>,
}

/// The text, from stdin: UTF-8 of 1 to 4,096 UTF-16 units, sent as it is.
pub fn read_text(input: impl Read) -> Result<Zeroizing<String>, HostError> {
    let bad = || HostError::Usage("the text is 1 to 4096 characters of UTF-8 on stdin".into());
    let mut bytes = Zeroizing::new(Vec::new());
    input
        .take(TEXT_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| bad())?;
    let text = std::str::from_utf8(&bytes).map_err(|_| bad())?;
    if text.trim().is_empty() || text.encode_utf16().count() > TEXT_MAX_CHARS {
        return Err(bad());
    }
    Ok(Zeroizing::new(text.to_owned()))
}

/// Send `text` to the owner of `tree`'s agent through `api` (Telegram's
/// Bot API, or a fake in tests), recording it in `log_dir`.
pub fn send(
    tree: &Tree,
    log_dir: &Path,
    api: &str,
    notice: &NoticeId,
    text: &str,
    now: u64,
) -> Result<Answer, HostError> {
    let mut log = NoticeLog::open(log_dir, tree.id())?;
    let records = log.records()?;
    let answer = |result, first, telegram_error| Answer {
        notice: notice.clone(),
        result,
        first,
        telegram_error,
    };
    if records
        .iter()
        .any(|record| record.notice == *notice && record.result.attempted())
    {
        // Tried before: report what happened then, send nothing now.
        let last = records
            .iter()
            .rev()
            .find(|record| record.notice == *notice)
            .expect("an attempted notice has a record");
        return Ok(answer(last.result, false, last.telegram_error));
    }
    let mut finish = |result, telegram_error| -> Result<Answer, HostError> {
        log.append(&Record {
            notice: notice.clone(),
            at: now,
            result,
            telegram_error,
        })?;
        Ok(answer(result, true, telegram_error))
    };
    let Some(token) = secrets::connected_telegram(tree)? else {
        return finish(Outcome::NoTelegram, None);
    };
    let chat = match owner(tree)? {
        Owner::None => return finish(Outcome::NoTelegram, None),
        Owner::NotOne => return finish(Outcome::NoSingleOwner, None),
        Owner::One(chat) => chat,
    };
    let today = records
        .iter()
        .filter(|record| record.result.attempted() && now.saturating_sub(record.at) < DAY_SECONDS)
        .count();
    if today >= NOTICES_PER_DAY {
        return finish(Outcome::RateLimited, None);
    }
    drop_capabilities()?;
    // The claim goes to disk first: a crash mid-send never sends again.
    finish(Outcome::Sending, None)?;
    let (result, telegram_error) = deliver(api, &token, chat, text);
    finish(result, telegram_error)
}

enum Owner {
    None,
    NotOne,
    One(i64),
}

/// The single user the agent's Telegram pairing approved, as a private
/// chat id (a positive user id; groups are negative). A pairing file that
/// is there but can't be read whole is never taken for no file: it may
/// hold the users that make the other folder's lone user not the owner.
fn owner(tree: &Tree) -> Result<Owner, HostError> {
    let ids = match approved(tree, "telegram")? {
        Approved::Unreadable => return Ok(Owner::NotOne),
        Approved::Ids(ids) => ids,
    };
    let mut ids = ids.into_iter();
    Ok(match (ids.next(), ids.next()) {
        (None, _) => Owner::None,
        (Some(chat), None) => Owner::One(chat),
        _ => Owner::NotOne,
    })
}

/// Give up every capability, for good. NixOS's wrapper raises the reader's
/// CAP_DAC_READ_SEARCH into the ambient set, which an exec would pass on to
/// curl. The tree has been read by now, and the log is this account's own.
fn drop_capabilities() -> Result<(), HostError> {
    use rustix::thread::{
        CapabilitySet, CapabilitySets, clear_ambient_capability_set, set_capabilities,
    };
    let failed =
        |error: rustix::io::Errno| HostError::Write(format!("dropping capabilities: {error}"));
    clear_ambient_capability_set().map_err(failed)?;
    set_capabilities(
        None,
        CapabilitySets {
            effective: CapabilitySet::empty(),
            permitted: CapabilitySet::empty(),
            inheritable: CapabilitySet::empty(),
        },
    )
    .map_err(failed)
}

/// One `sendMessage` through curl, its whole config (token included) on
/// stdin. The token is checked to be Telegram's shape first, so it can't
/// add config of its own.
fn deliver(api: &str, token: &str, chat: i64, text: &str) -> (Outcome, Option<u16>) {
    if !telegram_token(token) {
        return (Outcome::Failed, None);
    }
    let body = serde_json::json!({ "chat_id": chat, "text": text }).to_string();
    let mut config = Zeroizing::new(String::new());
    config.push_str(&format!("url = \"{api}/bot{token}/sendMessage\"\n"));
    if api.starts_with("https://") {
        config.push_str("proto = \"=https\"\n");
    }
    config.push_str("header = \"Content-Type: application/json\"\n");
    config.push_str(&format!("data = \"{}\"\n", curl_quoted(&body)));
    config.push_str("max-time = 30\nsilent\n");
    let mut command = Command::new(CURL);
    command
        .args(["--config", "-"])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for name in ["SSL_CERT_FILE", "NIX_SSL_CERT_FILE"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let Ok(mut child) = command.spawn() else {
        return (Outcome::Failed, None);
    };
    let written = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(config.as_bytes()).is_ok());
    let mut answer = Zeroizing::new(Vec::new());
    let read = child.stdout.take().is_some_and(|stdout| {
        stdout
            .take(ANSWER_MAX_BYTES)
            .read_to_end(&mut answer)
            .is_ok()
    });
    let _ = child.wait();
    if !written || !read {
        return (Outcome::Failed, None);
    }
    #[derive(Deserialize)]
    struct Reply {
        ok: bool,
        error_code: Option<u16>,
    }
    match serde_json::from_slice::<Reply>(&answer) {
        Ok(Reply { ok: true, .. }) => (Outcome::Sent, None),
        Ok(Reply {
            ok: false,
            error_code,
        }) => (Outcome::Refused, error_code),
        Err(_) => (Outcome::Failed, None),
    }
}

/// A bot token as Telegram issues them: digits, `:`, then 30 or more of
/// `[A-Za-z0-9_-]`.
fn telegram_token(token: &str) -> bool {
    let Some((bot, secret)) = token.split_once(':') else {
        return false;
    };
    !bot.is_empty()
        && bot.len() <= 20
        && bot.bytes().all(|byte| byte.is_ascii_digit())
        && (30..=128).contains(&secret.len())
        && secret
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}

/// A string inside curl's config quotes: `\` and `"` escaped. Compact JSON
/// has no raw control characters to escape.
fn curl_quoted(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
thread_local! {
    /// Directory syncs this thread made, for the test that a log's
    /// directory entry is durable before a send.
    static DIRECTORY_SYNCS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Make `dir`'s entries durable: syncing a file doesn't persist the name a
/// new file was just given (fsync(2)).
fn sync_directory(dir: &Path) -> std::io::Result<()> {
    File::open(dir)?.sync_all()?;
    #[cfg(test)]
    DIRECTORY_SYNCS.with(|count| count.set(count.get() + 1));
    Ok(())
}

/// An agent's notice log, locked while open.
struct NoticeLog {
    file: File,
}

impl NoticeLog {
    fn open(dir: &Path, id: &DurableId) -> Result<Self, HostError> {
        let path = dir.join(format!("{}.jsonl", id.as_str()));
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(&path)
            .map_err(|error| HostError::io(path.clone(), error))?;
        flock(&file, FlockOperation::LockExclusive)
            .map_err(|error| HostError::io(path.clone(), error.into()))?;
        // A new log's name is durable before anything is read or sent, so
        // a crash after a send can't forget its claim.
        sync_directory(dir).map_err(|error| HostError::io(path, error))?;
        Ok(Self { file })
    }

    fn records(&mut self) -> Result<Vec<Record>, HostError> {
        let broken = || HostError::Write("the notice log is not readable".into());
        let mut lines = Vec::new();
        let mut reader = BufReader::new((&self.file).take(LOG_MAX_BYTES + 1));
        let mut read = 0u64;
        let mut line = String::new();
        loop {
            line.clear();
            let length = reader.read_line(&mut line).map_err(|_| broken())?;
            if length == 0 {
                break;
            }
            read += length as u64;
            if read > LOG_MAX_BYTES {
                return Err(broken());
            }
            lines.push(serde_json::from_str::<Record>(line.trim_end()).map_err(|_| broken())?);
        }
        Ok(lines)
    }

    fn append(&mut self, record: &Record) -> Result<(), HostError> {
        let broken = |error: std::io::Error| HostError::Write(format!("the notice log: {error}"));
        let mut line = serde_json::to_vec(record).expect("a record serializes");
        line.push(b'\n');
        self.file.write_all(&line).map_err(broken)?;
        self.file.sync_data().map_err(broken)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A log's directory entry is made durable when the log is opened,
    /// before anything is read or sent: a file sync alone doesn't persist
    /// a new file's name.
    #[test]
    fn opening_a_log_syncs_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let before = DIRECTORY_SYNCS.with(|count| count.get());
        let _log = NoticeLog::open(dir.path(), &DurableId::parse("runtime_1").unwrap()).unwrap();
        assert_eq!(DIRECTORY_SYNCS.with(|count| count.get()), before + 1);
        assert!(dir.path().join("runtime_1.jsonl").exists());
    }

    #[test]
    fn notice_ids_and_tokens_are_checked() {
        assert!(NoticeId::parse("move-p1.2026-11-10").is_ok());
        for bad in ["", ".hidden", "a b", "a/b", &"x".repeat(65)] {
            assert!(NoticeId::parse(bad).is_err(), "{bad}");
        }
        assert!(telegram_token(
            "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw1"
        ));
        for bad in [
            "123456789:short",
            "bot:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw1",
            "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw1\"\nurl = \"http://evil",
        ] {
            assert!(!telegram_token(bad), "{bad}");
        }
    }

    #[test]
    fn the_text_is_plain_and_capped() {
        assert_eq!(
            &*read_text(&b"Hello \xf0\x9f\x91\x8b\n"[..]).unwrap(),
            "Hello \u{1f44b}\n"
        );
        assert!(read_text("a".repeat(TEXT_MAX_CHARS).as_bytes()).is_ok());
        // An emoji is two UTF-16 units.
        assert!(read_text("\u{1f44b}".repeat(TEXT_MAX_CHARS / 2).as_bytes()).is_ok());
        let too_long = "a".repeat(TEXT_MAX_CHARS + 1);
        let too_many_emoji = "\u{1f44b}".repeat(TEXT_MAX_CHARS / 2 + 1);
        for bad in [
            &b""[..],
            b"  \n",
            b"\xff\xfe",
            too_long.as_bytes(),
            too_many_emoji.as_bytes(),
        ] {
            assert!(read_text(bad).is_err());
        }
    }

    #[test]
    fn json_survives_curl_quoting() {
        let body = serde_json::json!({ "text": "a \"quote\", a \\ and a\nnewline" }).to_string();
        let quoted = curl_quoted(&body);
        assert!(!quoted.contains(['\n', '\r']));
        // curl reads a backslash in quotes as "take the next character".
        let mut unquoted = String::new();
        let mut chars = quoted.chars();
        while let Some(c) = chars.next() {
            unquoted.push(if c == '\\' { chars.next().unwrap() } else { c });
        }
        assert_eq!(unquoted, body);
    }
}
