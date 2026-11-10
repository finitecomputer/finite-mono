//! The mover (M3) against a fake `Migrations` that follows fragment's rules
//! (`docs/migrations.md` on fragment #1190): NIP-98 by the mover's key on
//! every route; the mailbox holds each old agent's one command until a
//! report on it is taken; a report on another command is 409; a `seq` not
//! past the last taken is `taken: false`; the stage routes take only the
//! commanded stage while it stages. Core's CLI and the host reader are
//! stand-in programs that record what they were asked, and the stage is
//! the host reader's real `stream` of a small tree.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use finite_mover::contract::{Has, Roster, RosterAgent, RosterPerson, Sizes};
use finite_mover::core_cli::CoreCli;
use finite_mover::fleet::{Binding, FileFleet, FleetFile};
use finite_mover::hosts::HostCommands;
use finite_mover::migrations::MigrationsClient;
use finite_mover::mover::{Mover, SeqClock, Worked};
use finite_mover::stage::PART_BYTES_MIN;
use finite_mover_host::command::{parse, run};
use finite_nostr::{
    HttpAuthValidation, NostrPublicKey, decode_http_auth_header, validate_http_auth_event,
};
use reqwest::Url;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const MIB: usize = 1024 * 1024;
const MOVER_KEY: [u8; 32] = [0x21; 32];
const OLD: &str = "proj_a";

fn public_key(secret: &[u8; 32]) -> NostrPublicKey {
    let keys = nostr::Keys::new(nostr::SecretKey::from_slice(secret).unwrap());
    NostrPublicKey::from_protocol(keys.public_key())
}

fn sha_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// One old agent's move, as the fake keeps it.
#[derive(Default, Clone)]
struct Move {
    state: String,
    attempt: u32,
    command: Option<Value>,
    seq: u64,
}

#[derive(Default)]
struct Platform {
    snapshot: Option<String>,
    rosters: Vec<Value>,
    moves: BTreeMap<String, Move>,
    /// Every report, with whether it was taken.
    reports: Vec<(Value, bool)>,
    parts: BTreeMap<String, BTreeMap<u32, Vec<u8>>>,
    completed: BTreeMap<String, (Vec<u8>, Vec<u8>)>,
    /// When it takes a report of this kind, the command moves on (staff
    /// retried: a new attempt's).
    replace_on: Option<String>,
}

impl Platform {
    fn command(&self, old: &str) -> Option<&Value> {
        self.moves.get(old)?.command.as_ref()
    }
}

struct Fake {
    base: Url,
    platform: Arc<Mutex<Platform>>,
}

impl Fake {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let platform = Arc::new(Mutex::new(Platform::default()));
        let (shared, origin, mover) = (platform.clone(), base.clone(), public_key(&MOVER_KEY));
        std::thread::spawn(move || {
            for connection in listener.incoming() {
                serve(connection.unwrap(), &shared, &origin, &mover);
            }
        });
        Self {
            base: Url::parse(&base).unwrap(),
            platform,
        }
    }

    fn platform(&self) -> MutexGuard<'_, Platform> {
        self.platform.lock().unwrap()
    }

    /// Staff, or the person's Yes: `old` is told `command`.
    fn tell(&self, old: &str, state: &str, attempt: u32, command: Value) {
        let mut platform = self.platform();
        let one = platform.moves.entry(old.to_string()).or_default();
        *one = Move {
            state: state.into(),
            attempt,
            command: Some(command),
            seq: 0,
        };
    }

    fn state(&self, old: &str) -> String {
        self.platform()
            .moves
            .get(old)
            .map(|m| m.state.clone())
            .unwrap_or_default()
    }

    fn kinds(&self) -> Vec<String> {
        self.platform()
            .reports
            .iter()
            .map(|(report, taken)| {
                format!(
                    "{}{}",
                    report["kind"].as_str().unwrap(),
                    if *taken { "" } else { " (not taken)" }
                )
            })
            .collect()
    }
}

fn serve(stream: TcpStream, platform: &Mutex<Platform>, origin: &str, mover: &NostrPublicKey) {
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
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let (status, answer) = route(
        &mut platform.lock().unwrap(),
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
    platform: &mut Platform,
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
    let Some(rest) = path.strip_prefix("/api/migrations/mover/") else {
        return (404, json!({}));
    };
    match (method, rest) {
        ("PUT", "roster") => {
            let roster: Value = serde_json::from_slice(body).unwrap();
            let snapshot = roster["snapshot"].as_str().unwrap().to_string();
            let replay = platform.snapshot.as_deref() == Some(snapshot.as_str());
            for person in roster["people"].as_array().unwrap() {
                for agent in person["agents"].as_array().unwrap() {
                    let old = agent["old"].as_str().unwrap().to_string();
                    platform.moves.entry(old).or_insert_with(|| Move {
                        state: "listed".into(),
                        ..Move::default()
                    });
                }
            }
            platform.snapshot = Some(snapshot);
            platform.rosters.push(roster);
            (200, json!({"bound": 0, "withdrawn": 0, "replay": replay}))
        }
        ("GET", "mailbox") => {
            let commands: Vec<Value> = platform
                .moves
                .values()
                .filter_map(|m| m.command.clone())
                .collect();
            (200, json!({"commands": commands}))
        }
        ("POST", rest) if rest.starts_with("agents/") && rest.ends_with("/report") => {
            let old = &rest["agents/".len()..rest.len() - "/report".len()];
            let report: Value = serde_json::from_slice(body).unwrap();
            let Some(one) = platform.moves.get(old).cloned() else {
                return (404, json!({}));
            };
            if one.command.as_ref().map(|c| &c["id"]) != Some(&report["command"]) {
                return (409, json!({"error": "conflict"}));
            }
            let seq = report["seq"].as_u64().unwrap();
            if seq <= one.seq {
                platform.reports.push((report, false));
                return (
                    200,
                    json!({"state": one.state, "attempt": one.attempt, "taken": false}),
                );
            }
            let kind = report["kind"].as_str().unwrap().to_string();
            let mut next = Move { seq, ..one };
            match kind.as_str() {
                "stopped" => next.state = "staging".into(),
                "progress" => {}
                "staged" => (next.state, next.command, next.seq) = ("importing".into(), None, 0),
                "failed" => (next.state, next.command, next.seq) = ("held".into(), None, 0),
                "retired" => (next.state, next.command, next.seq) = ("moved".into(), None, 0),
                "restarted" => (next.state, next.command, next.seq) = ("open".into(), None, 0),
                _ => return (400, json!({"error": "a report of another kind"})),
            }
            if platform.replace_on.as_deref() == Some(kind.as_str()) {
                // Staff retried: a new attempt's command replaces this one.
                next.attempt += 1;
                next.command = Some(
                    json!({"id": format!("{old}:{}:stop_and_stage", next.attempt), "old": old, "attempt": next.attempt, "kind": "stop_and_stage", "phase": "p1", "stage": format!("{old}.p1.{}", next.attempt)}),
                );
                next.seq = 0;
            }
            let answer = json!({"state": next.state, "attempt": next.attempt, "taken": true});
            platform.moves.insert(old.to_string(), next);
            platform.reports.push((report, true));
            (200, answer)
        }
        (method, rest) if rest.starts_with("stages/") => {
            let rest = &rest["stages/".len()..];
            let (stage, action) = rest.split_once('/').unwrap();
            let old = stage.split('.').next().unwrap();
            let commanded = platform.command(old).and_then(|c| c["stage"].as_str()) == Some(stage);
            if !commanded {
                return (403, json!({"error": "forbidden"}));
            }
            if platform.moves[old].state != "staging" {
                return (409, json!({"error": "conflict"}));
            }
            let stage = stage.to_string();
            match (method, action) {
                ("GET", "parts") => {
                    let parts: Vec<Value> = platform
                        .parts
                        .get(&stage)
                        .into_iter()
                        .flatten()
                        .map(|(n, part)| part_json(*n, part))
                        .collect();
                    (200, json!({"parts": parts}))
                }
                ("PUT", action) if action.starts_with("parts/") => {
                    let n: u32 = action["parts/".len()..].parse().unwrap();
                    if query != format!("sha256={}", sha_hex(body)) {
                        return (400, json!({}));
                    }
                    platform
                        .parts
                        .entry(stage)
                        .or_default()
                        .insert(n, body.to_vec());
                    (200, part_json(n, body))
                }
                ("POST", "complete") => {
                    let request: Value = serde_json::from_slice(body).unwrap();
                    let manifest = serde_json::to_vec(&request["manifest"]).unwrap();
                    let zip_bytes = request["manifest"]["zip_bytes"].as_u64().unwrap();
                    let parts = platform.parts.get(&stage).cloned().unwrap_or_default();
                    let mut zip = Vec::new();
                    let mut n = 1;
                    while (zip.len() as u64) < zip_bytes {
                        let Some(part) = parts.get(&n) else {
                            return (400, json!({}));
                        };
                        zip.extend_from_slice(part);
                        n += 1;
                    }
                    let answer =
                        json!({"manifest_sha256": sha_hex(&manifest), "zip_bytes": zip_bytes});
                    platform.completed.insert(stage, (zip, manifest));
                    (200, answer)
                }
                _ => (404, json!({})),
            }
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
        .prefix("mover-m3-")
        .tempdir()
        .unwrap()
        .keep()
}

fn write(root: &Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// The host reader's real `stream` of a stopped tree (about 12 MiB).
fn host_stage(dir: &Path) -> Vec<u8> {
    let work_root = dir.join("work");
    let root = work_root.join("kata/runtime_a");
    write(&root, "agent/hermes-home/SOUL.md", b"kind and brief\n");
    write(
        &root,
        "agent/hermes-home/.env",
        b"OPENROUTER_API_KEY=sk-or-v1-0123456789abcdef0123\n",
    );
    write(&root, "workspace/notes.md", b"notes\n");
    let photo: Vec<u8> = (0..12 * MIB).map(|index| (index % 251) as u8).collect();
    write(&root, "workspace/photos/cat.jpg", &photo);
    let mut out = Vec::new();
    run(
        &work_root,
        parse(["stream", "runtime_a"]).unwrap(),
        &mut out,
    )
    .unwrap();
    out
}

/// Everything around the mover: the fake platform, a fleet file, Core's
/// CLI and the host reader as stand-ins that log their arguments.
struct World {
    fake: Fake,
    dir: PathBuf,
    stage: Vec<u8>,
    fleet: FileFleet,
    core: CoreCli,
    hosts: HostCommands,
    platform: MigrationsClient,
    locks: PathBuf,
}

impl World {
    fn new() -> Self {
        let fake = Fake::new();
        let dir = scratch();
        let stage = host_stage(&dir);
        std::fs::write(dir.join("stage.zip"), &stage).unwrap();
        std::fs::create_dir_all(dir.join("locks")).unwrap();
        let fleet_file = dir.join("fleet.json");
        let world = Self {
            platform: MigrationsClient::new(fake.base.clone(), Zeroizing::new(MOVER_KEY)).unwrap(),
            fake,
            fleet: FileFleet::new(&fleet_file),
            core: CoreCli {
                program: vec![
                    "sh".into(),
                    "-c".into(),
                    CORE.replace("$MOVER_WORLD", &dir.display().to_string()),
                    "core".into(),
                ],
                admin_email: "mover@finite.test".into(),
                admin_workos_user_id: "user_mover".into(),
                wait_seconds: 60,
            },
            hosts: HostCommands {
                hosts: BTreeMap::from([(
                    "lat-3".to_string(),
                    vec![
                        "sh".into(),
                        "-c".into(),
                        HOST.replace("$MOVER_WORLD", &dir.display().to_string()),
                        "host".into(),
                    ],
                )]),
            },
            stage,
            locks: dir.join("locks"),
            dir,
        };
        world.bindings(Some(None));
        world
    }

    /// The fleet: `proj_a` on lat-3, retired already with `receipt` if
    /// Some(Some(_)), or no binding at all if None.
    fn bindings(&self, retired: Option<Option<&str>>) {
        let roster = Roster {
            snapshot: "s1".into(),
            observed_ms: 1_800_000_000_000,
            people: vec![RosterPerson {
                workos_id: "user_owner".into(),
                email: "owner@finite.test".into(),
                agents: vec![RosterAgent {
                    old: OLD.into(),
                    name: "Old Agent".into(),
                    picture_url: None,
                    last_active_ms: Some(1_799_000_000_000),
                    sizes: Sizes {
                        cache: 1,
                        history: 2,
                        work: 3,
                        staged: 5,
                        total: 6,
                    },
                    brains: vec![],
                    sites: vec![],
                    has: Has {
                        telegram: true,
                        openrouter: false,
                        nostr: true,
                    },
                }],
            }],
        };
        let bindings = match retired {
            None => BTreeMap::new(),
            Some(receipt) => BTreeMap::from([(
                OLD.to_string(),
                Binding {
                    old: OLD.into(),
                    agent_runtime_id: "runtime_a".into(),
                    source_host_id: "lat-3".into(),
                    source_machine_id: "machine_a".into(),
                    durable_state_id: "runtime_a".into(),
                    owned_brains: vec!["brain_x".into()],
                    retired_receipt: receipt.map(str::to_string),
                },
            )]),
        };
        let file = FleetFile { roster, bindings };
        std::fs::write(
            self.dir.join("fleet.json"),
            serde_json::to_vec(&file).unwrap(),
        )
        .unwrap();
    }

    fn mover(&self) -> Mover<'_> {
        let mut mover = Mover::new(
            &self.platform,
            &self.fleet,
            &self.core,
            &self.hosts,
            &self.locks,
        );
        mover.part_bytes = PART_BYTES_MIN;
        mover
    }

    /// What Core's CLI and the host reader were asked, one line per call.
    fn log(&self, name: &str) -> Vec<String> {
        std::fs::read_to_string(self.dir.join(name))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn fail(&self, what: &str) {
        std::fs::write(self.dir.join(format!("fail-{what}")), "").unwrap();
    }

    fn work(&self) -> Vec<Worked> {
        self.mover().work_mailbox().unwrap()
    }
}

/// Core's CLI, standing in: logs `$@`; fails a subcommand whose
/// `fail-<subcommand>` file exists; a retirement prints its request.
const CORE: &str = r#"
dir="$MOVER_WORLD"
printf '%s\n' "$*" >> "$dir/core.log"
if [ -e "$dir/fail-$1" ]; then echo "error: the runtime is not the one bound" >&2; exit 1; fi
case "$1" in runtime-retire-exact) printf '{\n  "id": "rcr_retired",\n  "status": "stopped"\n}\n' ;; esac
"#;

/// The host reader, standing in: logs `$@` and prints the real stage;
/// with `fail-stream`, prints part of it and fails as the reader would.
const HOST: &str = r#"
dir="$MOVER_WORLD"
printf '%s\n' "$*" >> "$dir/host.log"
if [ -e "$dir/fail-stream" ]; then head -c 6000000 "$dir/stage.zip"; echo "finite-mover-host: runtime_a changed while it was read" >&2; exit 75; fi
cat "$dir/stage.zip"
"#;

fn stage_command(attempt: u32) -> Value {
    json!({"id": format!("{OLD}:{attempt}:stop_and_stage"), "old": OLD, "attempt": attempt, "kind": "stop_and_stage", "phase": "p1", "stage": format!("{OLD}.p1.{attempt}")})
}

#[test]
fn a_yes_runs_from_the_mailbox_to_moved() {
    let world = World::new();
    let taken = world.mover().push_roster().unwrap();
    assert_eq!((taken.bound, taken.withdrawn, taken.replay), (0, 0, false));
    assert!(
        world.mover().push_roster().unwrap().replay,
        "the same snapshot again"
    );
    assert_eq!(world.fake.state(OLD), "listed");
    assert!(world.work().is_empty(), "nothing is done unasked");
    assert!(world.log("core.log").is_empty() && world.log("host.log").is_empty());

    // The person said Yes: stop and stage.
    world.fake.tell(OLD, "asked", 1, stage_command(1));
    let mut mover = world.mover();
    mover.progress_every = Duration::ZERO;
    let worked = mover.work_mailbox().unwrap();
    assert_eq!(
        worked
            .iter()
            .map(|w| w.outcome.as_str())
            .collect::<Vec<_>>(),
        ["staged"]
    );
    assert_eq!(
        world.log("core.log"),
        [
            "runtime-stop-exact --project-id proj_a --expected-agent-runtime-id runtime_a --expected-source-host-id lat-3 --expected-source-machine-id machine_a --admin-email mover@finite.test --admin-workos-user-id user_mover --wait-timeout-seconds 60"
        ]
    );
    assert_eq!(world.log("host.log"), ["stream runtime_a --brain brain_x"]);
    let kinds = world.fake.kinds();
    assert_eq!(kinds.first().map(String::as_str), Some("stopped"));
    assert_eq!(kinds.last().map(String::as_str), Some("staged"));
    assert!(
        kinds[1..kinds.len() - 1].iter().all(|k| k == "progress") && kinds.len() >= 4,
        "{kinds:?}"
    );
    {
        let platform = world.fake.platform();
        let (zip, manifest) = &platform.completed["proj_a.p1.1"];
        assert_eq!(zip, &world.stage, "the stage is the host reader's stream");
        let staged = &platform.reports.last().unwrap().0;
        assert_eq!(staged["manifest_sha256"], sha_hex(manifest));
        assert_eq!(staged["files"], 3);
        let seqs: Vec<u64> = platform
            .reports
            .iter()
            .map(|(r, _)| r["seq"].as_u64().unwrap())
            .collect();
        assert!(seqs.windows(2).all(|pair| pair[1] > pair[0]), "seq grows");
        let progress: Vec<u64> = platform
            .reports
            .iter()
            .filter(|(r, _)| r["kind"] == "progress")
            .map(|(r, _)| r["bytes"].as_u64().unwrap())
            .collect();
        assert!(
            progress.windows(2).all(|pair| pair[1] > pair[0]),
            "progress grows: {progress:?}"
        );
    }
    assert_eq!(world.fake.state(OLD), "importing");
    assert!(
        world.work().is_empty(),
        "the mailbox is empty once staged was taken"
    );

    // The import went well: retire it as migrated.
    world.fake.tell(OLD, "retiring", 1, json!({"id": "proj_a:1:retire_migrated", "old": OLD, "attempt": 1, "kind": "retire", "as": "migrated"}));
    let worked = world.work();
    assert_eq!(worked[0].outcome, "retired");
    let core = world.log("core.log");
    assert!(
        core[1].starts_with(
            "runtime-retire-exact --project-id proj_a --expected-agent-runtime-id runtime_a"
        ),
        "{core:?}"
    );
    assert!(
        core[2].starts_with("runtime-mark-moved-exact --project-id proj_a")
            && core[2].ends_with("--moved migrated"),
        "{core:?}"
    );
    assert_eq!(
        world.fake.platform().reports.last().unwrap().0["receipt"],
        "rcr_retired"
    );
    assert_eq!(world.fake.state(OLD), "moved");
}

#[test]
fn a_failed_stop_is_reported_and_nothing_is_streamed() {
    let world = World::new();
    world.fail("runtime-stop-exact");
    world.fake.tell(OLD, "asked", 1, stage_command(1));
    let worked = world.work();
    assert_eq!(
        (worked[0].outcome.as_str(), worked[0].step.as_deref()),
        ("failed", Some("stop"))
    );
    assert!(
        worked[0]
            .why
            .as_deref()
            .unwrap()
            .contains("not the one bound"),
        "{worked:?}"
    );
    assert_eq!(world.fake.kinds(), ["failed"]);
    assert_eq!(world.fake.platform().reports[0].0["step"], "stop");
    assert!(
        world.log("host.log").is_empty(),
        "nothing is read of a running agent"
    );
}

#[test]
fn a_host_reader_that_fails_mid_stream_fails_the_stage_uncompleted() {
    let world = World::new();
    world.fail("stream");
    world.fake.tell(OLD, "asked", 1, stage_command(1));
    let worked = world.work();
    assert_eq!(
        (worked[0].outcome.as_str(), worked[0].step.as_deref()),
        ("failed", Some("stage"))
    );
    assert!(
        worked[0]
            .why
            .as_deref()
            .unwrap()
            .contains("host reader exited"),
        "{worked:?}"
    );
    assert_eq!(world.fake.kinds(), ["stopped", "failed"]);
    assert!(
        world.fake.platform().completed.is_empty(),
        "a cut stream is never completed"
    );
}

#[test]
fn a_command_replaced_meanwhile_is_left_quietly() {
    let world = World::new();
    world.fake.platform().replace_on = Some("stopped".into());
    world.fake.tell(OLD, "asked", 1, stage_command(1));
    // Staff retry as attempt 1's stop is taken: its parts are refused (403).
    let worked = world.work();
    assert_eq!(worked[0].outcome, "stale", "{worked:?}");
    assert_eq!(
        world.fake.kinds(),
        ["stopped"],
        "no failure is reported on attempt 2's behalf"
    );
    assert!(
        world.fake.platform().parts.is_empty(),
        "attempt 1 uploads nothing once replaced"
    );
    assert_eq!(
        world.fake.platform().command(OLD).unwrap()["attempt"],
        2,
        "attempt 2's command stays"
    );
}

#[test]
fn a_command_the_mover_does_not_know_stays_in_the_mailbox() {
    let world = World::new();
    world.fake.tell(
        OLD,
        "asked",
        1,
        json!({"id": "proj_a:1:shrink", "old": OLD, "attempt": 1, "kind": "shrink"}),
    );
    let worked = world.work();
    assert_eq!(worked[0].outcome, "left");
    assert!(world.fake.kinds().is_empty() && world.log("core.log").is_empty());
    assert!(
        world.fake.platform().command(OLD).is_some(),
        "still there, done by no one"
    );
}

#[test]
fn a_retirement_asked_again_marks_without_retiring_twice() {
    let world = World::new();
    world.bindings(Some(Some("rcr_earlier")));
    world.fake.tell(OLD, "retiring", 1, json!({"id": "proj_a:1:retire_archived", "old": OLD, "attempt": 1, "kind": "retire", "as": "archived"}));
    assert_eq!(world.work()[0].outcome, "retired");
    let core = world.log("core.log");
    assert_eq!(core.len(), 1, "{core:?}");
    assert!(
        core[0].starts_with("runtime-mark-moved-exact") && core[0].ends_with("--moved archived")
    );
    assert_eq!(world.fake.platform().reports[0].0["receipt"], "rcr_earlier");
}

#[test]
fn a_restart_starts_it_again() {
    let world = World::new();
    world.fake.tell(
        OLD,
        "held",
        1,
        json!({"id": "proj_a:1:restart", "old": OLD, "attempt": 1, "kind": "restart"}),
    );
    assert_eq!(world.work()[0].outcome, "restarted");
    assert!(world.log("core.log")[0].starts_with("runtime-restart-exact --project-id proj_a"));
    assert_eq!(world.fake.state(OLD), "open");
}

#[test]
fn an_old_agent_core_has_no_runtime_for_fails_its_command() {
    let world = World::new();
    world.bindings(None);
    world.fake.tell(OLD, "asked", 1, stage_command(1));
    let worked = world.work();
    assert_eq!(
        (worked[0].outcome.as_str(), worked[0].step.as_deref()),
        ("failed", Some("binding"))
    );
    assert!(world.log("core.log").is_empty());
}

#[test]
fn a_stage_that_is_not_the_commands_own_is_refused() {
    let world = World::new();
    let mut command = stage_command(2);
    command["stage"] = json!("proj_b.p1.2");
    world.fake.tell(OLD, "asked", 2, command);
    let worked = world.work();
    assert_eq!(
        (worked[0].outcome.as_str(), worked[0].step.as_deref()),
        ("failed", Some("command"))
    );
    assert!(
        world.log("core.log").is_empty(),
        "nothing is stopped for it"
    );
}

#[test]
fn a_mover_restarted_mid_command_is_still_heard() {
    let world = World::new();
    world.fake.tell(OLD, "asked", 1, stage_command(1));
    // The last mover took the stop and reported it, then went away.
    world.fake.platform().moves.get_mut(OLD).unwrap().seq = SeqClock::default().next();
    world.fake.platform().moves.get_mut(OLD).unwrap().state = "staging".into();
    std::thread::sleep(Duration::from_millis(2));
    assert_eq!(world.work()[0].outcome, "staged");
    assert!(
        world
            .fake
            .platform()
            .reports
            .iter()
            .all(|(_, taken)| *taken),
        "{:?}",
        world.fake.kinds()
    );
    assert_eq!(world.fake.state(OLD), "importing");
}

#[test]
fn another_keys_signature_is_refused_everywhere() {
    let world = World::new();
    let other = MigrationsClient::new(world.fake.base.clone(), Zeroizing::new([0x33; 32])).unwrap();
    let mover = Mover::new(
        &other,
        &world.fleet,
        &world.core,
        &world.hosts,
        &world.locks,
    );
    assert!(mover.push_roster().is_err());
    assert!(mover.work_mailbox().is_err());
    assert!(world.fake.platform().rosters.is_empty());
}
