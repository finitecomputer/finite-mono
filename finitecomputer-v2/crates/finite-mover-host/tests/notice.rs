//! `owner-telegram-notice` against a fake Telegram Bot API on 127.0.0.1,
//! sent through the real curl: the owner alone, once per notice id, a
//! daily cap, plain text, and a log that never holds the text, the token or
//! the chat.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use finite_mover_host::notice::{self, NOTICES_PER_DAY, NoticeId, Outcome};
use finite_mover_host::tree::{DurableId, Tree};
use serde_json::{Value, json};

const ID: &str = "runtime_notice";
const TOKEN: &str = "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw1";
const OWNER: i64 = 4242;
const NOW: u64 = 1_800_000_000;
const TEXT: &str = "Hi! Your agent is moving to https://finite.chat \u{1f44b}\nSign in with \"you@example.org\" \\ thanks.";

/// What the fake answers next, in order; then `ok: true`.
enum Reply {
    Refuse(u16),
    Hang,
}

#[derive(Default)]
struct Seen {
    requests: Vec<(String, Value)>,
    replies: VecDeque<Reply>,
}

struct FakeTelegram {
    api: String,
    seen: Arc<Mutex<Seen>>,
}

impl FakeTelegram {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Seen::default()));
        let shared = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header.trim_end().is_empty() {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let mut seen = shared.lock().unwrap();
                seen.requests
                    .push((path, serde_json::from_slice(&body).unwrap_or(Value::Null)));
                let answer = match seen.replies.pop_front() {
                    Some(Reply::Hang) => continue,
                    Some(Reply::Refuse(code)) => json!({
                        "ok": false, "error_code": code, "description": "Forbidden: bot was blocked by the user"
                    }),
                    None => {
                        json!({ "ok": true, "result": { "message_id": 7, "text": "echoed back" } })
                    }
                };
                let answer = answer.to_string();
                let mut stream = stream;
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                    answer.len()
                );
            }
        });
        Self { api, seen }
    }

    fn requests(&self) -> Vec<(String, Value)> {
        self.seen.lock().unwrap().requests.clone()
    }

    fn reply(&self, reply: Reply) {
        self.seen.lock().unwrap().replies.push_back(reply);
    }
}

/// A work root with one agent's tree, and an empty notice log beside it.
struct Host {
    work_root: PathBuf,
    log: PathBuf,
}

impl Host {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("notice-")
            .tempdir()
            .unwrap()
            .keep();
        let work_root = dir.join("work");
        std::fs::create_dir_all(work_root.join("kata").join(ID).join("agent/hermes-home")).unwrap();
        let log = dir.join("notices");
        std::fs::create_dir(&log).unwrap();
        Self { work_root, log }
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.work_root.join("kata").join(ID).join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn telegram(&self, enabled: bool) {
        self.write(
            "agent/hermes-home/config.yaml",
            &format!(
                "model:\n  default: glm\ngateway:\n  platforms:\n    telegram:\n      enabled: {enabled}\n      token: '{TOKEN}'\n"
            ),
        );
    }

    fn approve(&self, users: &[&str]) {
        let approved: serde_json::Map<String, Value> = users
            .iter()
            .map(|user| (user.to_string(), json!({ "user_name": "someone" })))
            .collect();
        self.write(
            "agent/hermes-home/platforms/pairing/telegram-approved.json",
            &Value::Object(approved).to_string(),
        );
    }

    fn send(&self, fake: &FakeTelegram, notice: &str, now: u64) -> notice::Answer {
        let tree = Tree::open(&self.work_root, DurableId::parse(ID).unwrap()).unwrap();
        notice::send(
            &tree,
            &self.log,
            &fake.api,
            &NoticeId::parse(notice).unwrap(),
            TEXT,
            now,
        )
        .unwrap()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.log.join(format!("{ID}.jsonl"))).unwrap_or_default()
    }
}

#[test]
fn the_owner_gets_the_text_once_per_notice() {
    let (host, fake) = (Host::new(), FakeTelegram::new());
    host.telegram(true);
    host.approve(&[&OWNER.to_string()]);
    let first = host.send(&fake, "move-p1", NOW);
    assert_eq!((first.result, first.first), (Outcome::Sent, true));
    let requests = fake.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, format!("/bot{TOKEN}/sendMessage"));
    // Exactly the chat and the text: no parse mode, so no Markdown.
    assert_eq!(requests[0].1, json!({ "chat_id": OWNER, "text": TEXT }));

    let again = host.send(&fake, "move-p1", NOW + 60);
    assert_eq!((again.result, again.first), (Outcome::Sent, false));
    assert_eq!(fake.requests().len(), 1, "the same notice is one message");

    let log = host.log();
    let lines: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        lines,
        [
            json!({ "notice": "move-p1", "at": NOW, "result": "sending" }),
            json!({ "notice": "move-p1", "at": NOW, "result": "sent" }),
        ]
    );
    for private in ["moving", TOKEN, &OWNER.to_string(), "echoed"] {
        assert!(!log.contains(private), "the log holds {private}");
    }
}

#[test]
fn no_telegram_sends_nothing_and_keeps_the_id() {
    let (host, fake) = (Host::new(), FakeTelegram::new());
    assert_eq!(
        host.send(&fake, "move-p1", NOW).result,
        Outcome::NoTelegram,
        "no config"
    );
    host.telegram(false);
    host.approve(&[&OWNER.to_string()]);
    assert_eq!(
        host.send(&fake, "move-p1", NOW).result,
        Outcome::NoTelegram,
        "switched off"
    );
    host.telegram(true);
    host.approve(&[]);
    assert_eq!(
        host.send(&fake, "move-p1", NOW).result,
        Outcome::NoTelegram,
        "nobody approved"
    );
    assert!(fake.requests().is_empty());

    // Once paired, the same notice goes: nothing was sent before.
    host.approve(&[&OWNER.to_string()]);
    let answer = host.send(&fake, "move-p1", NOW);
    assert_eq!((answer.result, answer.first), (Outcome::Sent, true));
    assert_eq!(fake.requests().len(), 1);
}

#[test]
fn a_token_in_the_env_file_is_used_too() {
    let (host, fake) = (Host::new(), FakeTelegram::new());
    host.write(
        "agent/hermes-home/.env",
        &format!("TELEGRAM_BOT_TOKEN={TOKEN}\n"),
    );
    host.approve(&[&OWNER.to_string()]);
    assert_eq!(host.send(&fake, "move-p1", NOW).result, Outcome::Sent);
    assert_eq!(fake.requests()[0].0, format!("/bot{TOKEN}/sendMessage"));
}

#[test]
fn only_a_single_private_owner_is_ever_messaged() {
    let (host, fake) = (Host::new(), FakeTelegram::new());
    host.telegram(true);
    for approved in [
        vec!["4242", "777"],
        vec!["-1001234567890"],
        vec!["owner"],
        vec!["04242"],
    ] {
        host.approve(&approved);
        assert_eq!(
            host.send(&fake, "move-p1", NOW).result,
            Outcome::NoSingleOwner,
            "{approved:?}"
        );
    }
    // A user approved in both pairing folders is still one user.
    host.approve(&["4242"]);
    host.write(
        "agent/hermes-home/pairing/telegram-approved.json",
        r#"{"4242": {"user_name": "owner"}}"#,
    );
    assert_eq!(host.send(&fake, "move-p1", NOW).result, Outcome::Sent);
    assert_eq!(fake.requests().len(), 1);
    assert_eq!(fake.requests()[0].1["chat_id"], 4242);
}

#[test]
fn each_agent_gets_a_few_notices_a_day() {
    let (host, fake) = (Host::new(), FakeTelegram::new());
    host.telegram(true);
    host.approve(&[&OWNER.to_string()]);
    for index in 0..NOTICES_PER_DAY {
        let answer = host.send(&fake, &format!("notice-{index}"), NOW + index as u64);
        assert_eq!(answer.result, Outcome::Sent);
    }
    let over = host.send(&fake, "one-too-many", NOW + 3_600);
    assert_eq!(over.result, Outcome::RateLimited);
    assert_eq!(fake.requests().len(), NOTICES_PER_DAY);
    // A day after the first, there is room again, and the limited id goes.
    let later = host.send(&fake, "one-too-many", NOW + 24 * 3_600);
    assert_eq!((later.result, later.first), (Outcome::Sent, true));
}

#[test]
fn a_refusal_or_no_answer_is_recorded_and_never_retried() {
    let (host, fake) = (Host::new(), FakeTelegram::new());
    host.telegram(true);
    host.approve(&[&OWNER.to_string()]);
    fake.reply(Reply::Refuse(403));
    let refused = host.send(&fake, "blocked", NOW);
    assert_eq!(
        (refused.result, refused.telegram_error),
        (Outcome::Refused, Some(403))
    );
    let again = host.send(&fake, "blocked", NOW);
    assert_eq!(
        (again.result, again.first, again.telegram_error),
        (Outcome::Refused, false, Some(403))
    );

    // The connection closes with no answer: it may have arrived, so the
    // same id is never sent again.
    fake.reply(Reply::Hang);
    assert_eq!(host.send(&fake, "lost", NOW).result, Outcome::Failed);
    assert_eq!(host.send(&fake, "lost", NOW).result, Outcome::Failed);
    assert_eq!(fake.requests().len(), 2);
}

/// The command as the forced command runs it: no Telegram here, so no
/// network either.
#[test]
fn the_command_reads_the_text_from_stdin_and_answers_in_json() {
    let host = Host::new();
    let binary = env!("CARGO_BIN_EXE_finite-mover-host");
    let run = |log: Option<&Path>, notice: &str, text: &[u8]| {
        let mut command = Command::new(binary);
        command.arg("--work-root").arg(&host.work_root);
        if let Some(log) = log {
            command.arg("--notice-log").arg(log);
        }
        let mut child = command
            .args(["owner-telegram-notice", ID, "--notice", notice])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(text).unwrap();
        child.wait_with_output().unwrap()
    };
    let output = run(Some(&host.log), "move-p1", TEXT.as_bytes());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        answer,
        json!({ "notice": "move-p1", "result": "no_telegram", "first": true })
    );

    // No log configured, a bad id, or text past Telegram's limit: refused.
    assert_eq!(run(None, "move-p1", b"hi").status.code(), Some(64));
    assert_eq!(run(Some(&host.log), "../x", b"hi").status.code(), Some(64));
    let long = "a".repeat(4097);
    assert_eq!(
        run(Some(&host.log), "move-p2", long.as_bytes())
            .status
            .code(),
        Some(64)
    );
}
