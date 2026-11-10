//! The uploader against a fake finite.chat that speaks the mover's three
//! stage routes as fragment's `docs/migrations.md` has them: NIP-98 by the
//! mover's key, the stage the current command names alone (403), only
//! while it stages (409), each part held to its SHA-256, and the zip made
//! whole from parts 1 to n at `complete`. The stages are the host reader's
//! own `stream` of a small tree, or a zip built by hand where a host would
//! never write one.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Cursor, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use finite_mover::migrations::MigrationsClient;
use finite_mover::sink::SinkError;
use finite_mover::stage::{PART_BYTES_MIN, Phase, StageError, StageTarget, upload_stage};
use finite_mover_host::command::{parse, run};
use finite_mover_host::stage::SUMMARY_ENTRY;
use finite_mover_host::zip64::ZipStream;
use finite_nostr::{
    HttpAuthValidation, NostrPublicKey, decode_http_auth_header, validate_http_auth_event,
};
use reqwest::Url;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const MIB: usize = 1024 * 1024;
const MOVER_KEY: [u8; 32] = [0x11; 32];
const OLD: &str = "proj_old1";
const SECRET_MARKER: &[u8] = b"SECRET-MARKER-NEVER-SENT";
const ENV: &[u8] = b"OPENROUTER_API_KEY=sk-or-v1-0123456789abcdef0123\n";

fn stage_id(attempt: u32) -> String {
    format!("{OLD}.p1.{attempt}")
}

fn target(attempt: u32) -> StageTarget {
    StageTarget::new(Phase::P1, OLD, attempt).unwrap()
}

fn sha_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn public_key(secret: &[u8; 32]) -> NostrPublicKey {
    let keys = nostr::Keys::new(nostr::SecretKey::from_slice(secret).unwrap());
    NostrPublicKey::from_protocol(keys.public_key())
}

/// What the fake finite.chat holds.
#[derive(Default)]
struct Chat {
    /// The stage the old agent's current `stop_and_stage` names.
    commanded: String,
    staging: bool,
    parts: BTreeMap<String, BTreeMap<u32, Vec<u8>>>,
    /// Each completed stage: its zip and its `manifest.json`.
    completed: BTreeMap<String, (Vec<u8>, Vec<u8>)>,
    /// The calls taken, as `PUT 2`.
    calls: Vec<String>,
    /// Every body byte that arrived, taken or not.
    received: Vec<u8>,
    /// Cut the connection halfway through this part's body, once.
    cut_part: Option<u32>,
    /// After taking part n, the command moves on to this stage.
    replace_after: Option<(u32, String)>,
}

struct Fake {
    base: Url,
    chat: Arc<Mutex<Chat>>,
}

impl Fake {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let chat = Arc::new(Mutex::new(Chat {
            commanded: stage_id(1),
            staging: true,
            ..Chat::default()
        }));
        let (shared, origin, mover) = (chat.clone(), base.clone(), public_key(&MOVER_KEY));
        std::thread::spawn(move || {
            for connection in listener.incoming() {
                serve(connection.unwrap(), &shared, &origin, &mover);
            }
        });
        Self {
            base: Url::parse(&base).unwrap(),
            chat,
        }
    }

    fn client(&self) -> MigrationsClient {
        self.client_with(MOVER_KEY)
    }

    fn client_with(&self, key: [u8; 32]) -> MigrationsClient {
        MigrationsClient::new(self.base.clone(), Zeroizing::new(key)).unwrap()
    }

    fn chat(&self) -> MutexGuard<'_, Chat> {
        self.chat.lock().unwrap()
    }

    /// The calls taken since the last look.
    fn calls(&self) -> Vec<String> {
        std::mem::take(&mut self.chat().calls)
    }
}

/// One HTTP/1.1 request on its own connection.
fn serve(stream: TcpStream, chat: &Mutex<Chat>, origin: &str, mover: &NostrPublicKey) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut words = line.split_whitespace();
    let (method, target) = (
        words.next().unwrap_or("").to_string(),
        words.next().unwrap_or("").to_string(),
    );
    let mut headers = BTreeMap::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').unwrap();
        headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
    }
    let length: usize = headers
        .get("content-length")
        .map_or(0, |value| value.parse().unwrap());
    let cut = {
        let mut chat = chat.lock().unwrap();
        let part = target
            .split('?')
            .next()
            .and_then(|path| path.rsplit_once("/parts/"))
            .and_then(|(_, n)| n.parse::<u32>().ok());
        let cut = method == "PUT" && part.is_some() && chat.cut_part == part;
        if cut {
            chat.cut_part = None;
        }
        cut
    };
    let mut body = vec![0; if cut { length / 2 } else { length }];
    reader.read_exact(&mut body).unwrap();
    chat.lock().unwrap().received.extend_from_slice(&body);
    if cut {
        // The connection drops mid-part: nothing of it is kept.
        let _ = stream.shutdown(std::net::Shutdown::Both);
        return;
    }
    let (status, answer) = route(
        &mut chat.lock().unwrap(),
        &method,
        &target,
        &headers,
        &body,
        origin,
        mover,
    );
    let answer = answer.to_string();
    let mut stream = stream;
    let _ = write!(
        stream,
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
        answer.len()
    );
}

fn route(
    chat: &mut Chat,
    method: &str,
    target: &str,
    headers: &BTreeMap<String, String>,
    body: &[u8],
    origin: &str,
    mover: &NostrPublicKey,
) -> (u16, Value) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut expected = HttpAuthValidation::new(method, format!("{origin}{target}"), now, 60)
        .with_expected_signer(*mover);
    if !body.is_empty() {
        expected = expected.with_body(body.to_vec());
    }
    let signed = headers
        .get("authorization")
        .and_then(|header| decode_http_auth_header(header).ok())
        .is_some_and(|event| validate_http_auth_event(&event, &expected).is_ok());
    if !signed {
        return (401, json!({"error": "unauthorized"}));
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let Some(rest) = path.strip_prefix("/api/migrations/mover/stages/") else {
        return (404, json!({}));
    };
    let (stage, action) = rest.split_once('/').unwrap();
    if stage != chat.commanded {
        return (403, json!({"error": "forbidden"}));
    }
    if !chat.staging {
        return (409, json!({"error": "conflict"}));
    }
    let stage = stage.to_string();
    match (method, action) {
        ("GET", "parts") => {
            chat.calls.push("GET parts".into());
            let parts: Vec<Value> = chat
                .parts
                .get(&stage)
                .into_iter()
                .flatten()
                .map(|(n, part)| part_json(*n, part))
                .collect();
            (200, json!({ "parts": parts }))
        }
        ("PUT", action) if action.starts_with("parts/") => {
            let n: u32 = action["parts/".len()..].parse().unwrap();
            chat.calls.push(format!("PUT {n}"));
            if !(1..=10_000).contains(&n) || body.len() > 64 * MIB {
                return (400, json!({}));
            }
            if query != format!("sha256={}", sha_hex(body)) {
                return (400, json!({"error": "the part is not its SHA-256"}));
            }
            chat.parts
                .entry(stage.clone())
                .or_default()
                .insert(n, body.to_vec());
            if let Some((after, next)) = chat.replace_after.clone()
                && after == n
            {
                chat.commanded = next;
            }
            (200, part_json(n, body))
        }
        ("POST", "complete") => {
            chat.calls.push("POST complete".into());
            let request: Value = serde_json::from_slice(body).unwrap();
            let manifest = serde_json::to_vec(&request["manifest"]).unwrap();
            let zip_bytes = request["manifest"]["zip_bytes"].as_u64().unwrap();
            let parts = chat.parts.get(&stage).cloned().unwrap_or_default();
            let mut zip = Vec::new();
            let mut n = 1;
            while (zip.len() as u64) < zip_bytes {
                let Some(part) = parts.get(&n) else {
                    return (400, json!({"error": "a part is missing"}));
                };
                let last = zip.len() as u64 + part.len() as u64 >= zip_bytes;
                if !last && part.len() < PART_BYTES_MIN {
                    return (400, json!({"error": "a part is too small"}));
                }
                zip.extend_from_slice(part);
                n += 1;
            }
            if zip.len() as u64 != zip_bytes {
                return (400, json!({"error": "the parts are not zip_bytes"}));
            }
            let answer = json!({"manifest_sha256": sha_hex(&manifest), "zip_bytes": zip_bytes});
            chat.completed.insert(stage, (zip, manifest));
            (200, answer)
        }
        _ => (404, json!({})),
    }
}

fn part_json(n: u32, body: &[u8]) -> Value {
    let sha256 = sha_hex(body);
    json!({"n": n, "bytes": body.len(), "etag": format!("etag-{}", &sha256[..8]), "sha256": sha256})
}

fn scratch() -> PathBuf {
    tempfile::Builder::new()
        .prefix("mover-finite-chat-")
        .tempdir()
        .unwrap()
        .keep()
}

fn write(root: &Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// The host reader's `stream` of a stopped tree: about 12 MiB, three
/// 5 MiB parts. Its `.env` stays behind on the host.
fn host_stage() -> Vec<u8> {
    let work_root = scratch();
    let root = work_root.join("kata/runtime_mover");
    write(&root, "agent/hermes-home/SOUL.md", b"kind and brief\n");
    write(&root, "agent/hermes-home/.env", ENV);
    write(&root, "workspace/notes.md", b"notes\n");
    let photo: Vec<u8> = (0..12 * MIB).map(|index| (index % 251) as u8).collect();
    write(&root, "workspace/photos/cat.jpg", &photo);
    let mut out = Vec::new();
    run(
        &work_root,
        parse(["stream", "runtime_mover"]).unwrap(),
        &mut out,
    )
    .unwrap();
    out
}

/// A zip no host writes: `entries`, then a host-shaped summary.
fn hand_stage(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut stream = ZipStream::new(Vec::new());
    for (name, body) in entries {
        stream
            .entry(name, 0o644, 0, body.len() as u64, &mut &body[..])
            .unwrap();
    }
    let summary = br#"{"left":[],"left_more":{"files":0,"bytes":0}}"#;
    stream
        .entry(
            SUMMARY_ENTRY,
            0o644,
            0,
            summary.len() as u64,
            &mut &summary[..],
        )
        .unwrap();
    stream.finish().unwrap()
}

fn names(zip: &[u8]) -> Vec<String> {
    let archive = zip::ZipArchive::new(Cursor::new(zip)).unwrap();
    let mut names: Vec<String> = archive.file_names().map(str::to_string).collect();
    names.sort();
    names
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// A stream that fails after `limit` bytes, as a dropped ssh would.
struct Breaks<'a> {
    bytes: &'a [u8],
    limit: usize,
}

impl Read for Breaks<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.limit == 0 {
            return Err(io::Error::new(io::ErrorKind::ConnectionReset, "ssh closed"));
        }
        let length = buffer.len().min(self.limit).min(self.bytes.len());
        buffer[..length].copy_from_slice(&self.bytes[..length]);
        self.bytes = &self.bytes[length..];
        self.limit -= length;
        Ok(length)
    }
}

#[test]
fn a_stage_goes_up_in_parts_and_completes_with_its_manifest() {
    let fake = Fake::new();
    let bytes = host_stage();
    let (manifest, completed) = upload_stage(
        &fake.client(),
        &target(1),
        &bytes[..],
        PART_BYTES_MIN,
        &scratch(),
    )
    .unwrap();
    assert_eq!(
        fake.calls(),
        ["GET parts", "PUT 1", "PUT 2", "PUT 3", "POST complete"]
    );
    let chat = fake.chat();
    let (zip, stored) = &chat.completed[&stage_id(1)];
    assert_eq!(zip, &bytes, "the zip is the stream, byte for byte");
    assert_eq!(
        names(zip),
        [
            ".finite-stage/summary.json",
            "agent/hermes-home/SOUL.md",
            "workspace/notes.md",
            "workspace/photos/cat.jpg",
        ]
    );
    assert_eq!(completed.manifest_sha256, sha_hex(stored));
    assert_eq!(completed.zip_bytes, bytes.len() as u64);
    let stored: Value = serde_json::from_slice(stored).unwrap();
    assert_eq!(stored, serde_json::to_value(&manifest).unwrap());
    assert_eq!(stored["format"], "finite-stage/1");
    assert_eq!(stored["old"], OLD);
    assert_eq!(stored["phase"], "p1");
    assert_eq!(stored["attempt"], 1);
    assert_eq!(stored["layout"], "finite-computer");
    assert_eq!(stored["zip_sha256"], sha_hex(&bytes));
    assert_eq!(stored["files"], 3);
    assert_eq!(stored["bytes"], 15 + 6 + 12 * MIB as u64);
    assert_eq!(
        stored["left"],
        json!([{"path": "agent/hermes-home/.env", "bytes": ENV.len(), "why": "secret"}])
    );
    assert!(stored.get("left_more").is_none());
}

#[test]
fn a_cut_stream_resumes_without_resending_its_parts() {
    let fake = Fake::new();
    let bytes = host_stage();
    let broken = Breaks {
        bytes: &bytes,
        limit: 2 * PART_BYTES_MIN + 100,
    };
    let error = upload_stage(
        &fake.client(),
        &target(1),
        broken,
        PART_BYTES_MIN,
        &scratch(),
    )
    .unwrap_err();
    assert!(matches!(error, StageError::Read(_)), "{error}");
    assert_eq!(fake.calls(), ["GET parts", "PUT 1", "PUT 2"]);
    assert!(fake.chat().completed.is_empty(), "nothing is staged yet");

    // The mover runs `stream` again and the upload picks up where it was.
    upload_stage(
        &fake.client(),
        &target(1),
        &bytes[..],
        PART_BYTES_MIN,
        &scratch(),
    )
    .unwrap();
    assert_eq!(fake.calls(), ["GET parts", "PUT 3", "POST complete"]);
    assert_eq!(fake.chat().completed[&stage_id(1)].0, bytes);
}

#[test]
fn a_cut_connection_loses_only_the_part_it_carried() {
    let fake = Fake::new();
    let bytes = host_stage();
    fake.chat().cut_part = Some(2);
    let error = upload_stage(
        &fake.client(),
        &target(1),
        &bytes[..],
        PART_BYTES_MIN,
        &scratch(),
    )
    .unwrap_err();
    assert!(
        matches!(error, StageError::Sink(SinkError::Unreachable { .. })),
        "{error}"
    );
    assert_eq!(
        fake.calls(),
        ["GET parts", "PUT 1"],
        "part 2 was never taken"
    );
    upload_stage(
        &fake.client(),
        &target(1),
        &bytes[..],
        PART_BYTES_MIN,
        &scratch(),
    )
    .unwrap();
    assert_eq!(
        fake.calls(),
        ["GET parts", "PUT 2", "PUT 3", "POST complete"]
    );
    assert_eq!(fake.chat().completed[&stage_id(1)].0, bytes);
}

#[test]
fn a_part_that_differs_on_resume_is_sent_again() {
    let fake = Fake::new();
    let junk = vec![9; PART_BYTES_MIN];
    fake.chat().parts.insert(
        stage_id(1),
        BTreeMap::from([(1, junk.clone()), (7, junk.clone())]),
    );
    let bytes = host_stage();
    upload_stage(
        &fake.client(),
        &target(1),
        &bytes[..],
        PART_BYTES_MIN,
        &scratch(),
    )
    .unwrap();
    assert_eq!(
        fake.calls(),
        ["GET parts", "PUT 1", "PUT 2", "PUT 3", "POST complete"]
    );
    // Part 7, from an older and longer stream, is not part of the zip.
    assert_eq!(fake.chat().completed[&stage_id(1)].0, bytes);
}

#[test]
fn a_stale_attempt_sends_nothing_and_a_replaced_command_stops_the_run() {
    let fake = Fake::new();
    let bytes = host_stage();
    fake.chat().commanded = stage_id(2);
    let error = upload_stage(
        &fake.client(),
        &target(1),
        &bytes[..],
        PART_BYTES_MIN,
        &scratch(),
    )
    .unwrap_err();
    assert!(
        matches!(error, StageError::Sink(SinkError::NotCommanded)),
        "{error}"
    );
    assert!(
        fake.calls().is_empty(),
        "a stale attempt is refused at its first call"
    );
    assert!(fake.chat().received.is_empty());

    // Attempt 2 runs while staff replace its command after part 1.
    fake.chat().replace_after = Some((1, stage_id(3)));
    let error = upload_stage(
        &fake.client(),
        &target(2),
        &bytes[..],
        PART_BYTES_MIN,
        &scratch(),
    )
    .unwrap_err();
    assert!(
        matches!(error, StageError::Sink(SinkError::NotCommanded)),
        "{error}"
    );
    assert_eq!(fake.calls(), ["GET parts", "PUT 1"]);
    assert!(fake.chat().completed.is_empty());

    // A move that is not staging takes nothing either.
    let mut chat = fake.chat();
    chat.commanded = stage_id(3);
    chat.staging = false;
    drop(chat);
    let error = upload_stage(
        &fake.client(),
        &target(3),
        &bytes[..],
        PART_BYTES_MIN,
        &scratch(),
    )
    .unwrap_err();
    assert!(
        matches!(error, StageError::Sink(SinkError::NotStaging)),
        "{error}"
    );
}

#[test]
fn a_secret_path_refuses_the_whole_stage_before_its_bytes_leave() {
    for secret in [
        "workspace/app/.env",
        "agent/identity/identity.json",
        "agent/hermes-home/config.yaml",
    ] {
        let fake = Fake::new();
        let photo = vec![1; 6 * MIB];
        let bytes = hand_stage(&[
            ("workspace/photos/cat.jpg", &photo),
            (secret, SECRET_MARKER),
            ("workspace/after.md", b"after"),
        ]);
        let error = upload_stage(
            &fake.client(),
            &target(1),
            &bytes[..],
            PART_BYTES_MIN,
            &scratch(),
        )
        .unwrap_err();
        assert!(
            matches!(&error, StageError::Refused { path } if path == secret),
            "{error}"
        );
        // Part 1 is the photo's; part 2 begins the secret and never goes.
        assert_eq!(fake.calls(), ["GET parts", "PUT 1"]);
        let chat = fake.chat();
        assert!(
            !contains(&chat.received, SECRET_MARKER),
            "no secret byte arrived"
        );
        assert!(!contains(&chat.received, secret.as_bytes()));
        assert!(chat.completed.is_empty(), "no stage, no manifest");
    }

    // A secret in the first part: nothing is sent at all.
    let fake = Fake::new();
    let bytes = hand_stage(&[("agent/hermes-home/.env", SECRET_MARKER)]);
    let error = upload_stage(
        &fake.client(),
        &target(1),
        &bytes[..],
        PART_BYTES_MIN,
        &scratch(),
    )
    .unwrap_err();
    assert!(matches!(error, StageError::Refused { .. }), "{error}");
    assert_eq!(fake.calls(), ["GET parts"]);
    assert!(fake.chat().received.is_empty());
}

/// A central-directory signature where a local header belongs: the
/// scanner reads no further, so nothing after it may leave unchecked.
#[test]
fn a_forged_directory_signature_carries_nothing_out() {
    let forged = [&0x0201_4b50u32.to_le_bytes()[..], &[0; 26]].concat();
    // At the very start: no part is sent at all.
    let fake = Fake::new();
    let bytes = [
        forged.clone(),
        hand_stage(&[("workspace/.env", SECRET_MARKER)]),
    ]
    .concat();
    let error = upload_stage(
        &fake.client(),
        &target(1),
        &bytes[..],
        PART_BYTES_MIN,
        &scratch(),
    )
    .unwrap_err();
    assert!(matches!(error, StageError::Unreadable(_)), "{error}");
    assert_eq!(fake.calls(), ["GET parts"]);
    assert!(fake.chat().received.is_empty());

    // After a real entry: its part goes, the rest never does.
    let fake = Fake::new();
    let photo = vec![1; 6 * MIB];
    let honest = hand_stage(&[
        ("workspace/photos/cat.jpg", &photo),
        ("workspace/.env", SECRET_MARKER),
    ]);
    let at = honest
        .windows(14)
        .position(|window| window == b"workspace/.env")
        .unwrap()
        - 30;
    let bytes = [&honest[..at], &forged, &honest[at..]].concat();
    let error = upload_stage(
        &fake.client(),
        &target(1),
        &bytes[..],
        PART_BYTES_MIN,
        &scratch(),
    )
    .unwrap_err();
    assert!(matches!(error, StageError::Unreadable(_)), "{error}");
    assert_eq!(fake.calls(), ["GET parts", "PUT 1"]);
    let chat = fake.chat();
    assert!(
        !contains(&chat.received, SECRET_MARKER),
        "no secret byte arrived"
    );
    assert!(!contains(&chat.received, b"workspace/.env"));
    assert!(chat.completed.is_empty());
    drop(chat);

    // In a part's last 29 bytes, where the scanner can't yet tell a header
    // from a directory record: that part waits, and nothing goes.
    let fake = Fake::new();
    let name = "workspace/photo.jpg";
    let header = 30 + name.len() + 20;
    let photo = vec![1; PART_BYTES_MIN - 29 - header - 24];
    let honest = hand_stage(&[(name, &photo)]);
    let end = header + photo.len() + 24;
    assert_eq!(end, PART_BYTES_MIN - 29);
    let forged = [&0x0201_4b50u32.to_le_bytes()[..], SECRET_MARKER, &[0; 2]].concat();
    assert_eq!(forged.len(), 30);
    let bytes = [&honest[..end], &forged, &honest[end..]].concat();
    let error = upload_stage(
        &fake.client(),
        &target(1),
        &bytes[..],
        PART_BYTES_MIN,
        &scratch(),
    )
    .unwrap_err();
    assert!(matches!(error, StageError::Unreadable(_)), "{error}");
    assert_eq!(fake.calls(), ["GET parts"]);
    assert!(
        !contains(&fake.chat().received, SECRET_MARKER),
        "no byte past the signature"
    );
}

/// A local header split across two parts: the first part waits for the
/// second, then both go, in order.
#[test]
fn a_header_split_across_parts_holds_its_part_until_decided() {
    let fake = Fake::new();
    let first = vec![1; PART_BYTES_MIN - 30 - "workspace/a.bin".len() - 20 - 24 - 10];
    let bytes = hand_stage(&[
        ("workspace/a.bin", &first),
        ("workspace/second.md", b"second"),
    ]);
    let name = bytes
        .windows(19)
        .position(|window| window == b"workspace/second.md")
        .unwrap();
    // Its header starts 10 bytes before part 1's end.
    assert_eq!(name - 30, PART_BYTES_MIN - 10);
    upload_stage(
        &fake.client(),
        &target(1),
        &bytes[..],
        PART_BYTES_MIN,
        &scratch(),
    )
    .unwrap();
    assert_eq!(
        fake.calls(),
        ["GET parts", "PUT 1", "PUT 2", "POST complete"]
    );
    assert_eq!(fake.chat().completed[&stage_id(1)].0, bytes);
}

#[test]
fn unsafe_names_and_unreadable_stages_are_never_completed() {
    let fake = Fake::new();
    let upload = |bytes: &[u8]| {
        upload_stage(
            &fake.client(),
            &target(1),
            bytes,
            PART_BYTES_MIN,
            &scratch(),
        )
        .unwrap_err()
    };
    let error = upload(&hand_stage(&[("workspace/../x", b"1")]));
    assert!(matches!(error, StageError::UnsafeName { .. }), "{error}");
    let error = upload(&hand_stage(&[
        ("workspace/a.md", b"1"),
        ("workspace/a.md", b"2"),
    ]));
    assert!(
        matches!(&error, StageError::UnsafeName { path } if path == "workspace/a.md"),
        "{error}"
    );

    let mut no_summary = ZipStream::new(Vec::new());
    no_summary
        .entry("workspace/a.md", 0o644, 0, 1, &mut &b"a"[..])
        .unwrap();
    let error = upload(&no_summary.finish().unwrap());
    assert!(matches!(error, StageError::NoSummary), "{error}");

    let whole = host_stage();
    let error = upload(&whole[..PART_BYTES_MIN + 10]);
    assert!(matches!(error, StageError::Malformed(_)), "{error}");
    let error = upload(b"");
    assert!(matches!(error, StageError::Empty), "{error}");
    let error = upload(b"not a zip at all");
    assert!(matches!(error, StageError::Malformed(_)), "{error}");

    assert!(
        !fake.calls().contains(&"POST complete".to_string()),
        "nothing unchecked is ever completed"
    );
}

#[test]
fn a_signature_by_another_key_is_refused() {
    let fake = Fake::new();
    let bytes = host_stage();
    let error = upload_stage(
        &fake.client_with([0x22; 32]),
        &target(1),
        &bytes[..],
        PART_BYTES_MIN,
        &scratch(),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            StageError::Sink(SinkError::Refused { status: 401, .. })
        ),
        "{error}"
    );
    assert!(fake.calls().is_empty());
}

#[test]
fn a_second_run_of_the_same_agent_waits_its_turn() {
    let fake = Fake::new();
    let locks = scratch();
    let held = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(locks.join("p1-proj_old1.lock"))
        .unwrap();
    held.lock().unwrap();
    let bytes = host_stage();
    let error = upload_stage(
        &fake.client(),
        &target(1),
        &bytes[..],
        PART_BYTES_MIN,
        &locks,
    )
    .unwrap_err();
    assert!(matches!(error, StageError::Busy), "{error}");
    assert!(fake.calls().is_empty(), "a busy run calls nothing");
    held.unlock().unwrap();
    upload_stage(
        &fake.client(),
        &target(1),
        &bytes[..],
        PART_BYTES_MIN,
        &locks,
    )
    .unwrap();
}
