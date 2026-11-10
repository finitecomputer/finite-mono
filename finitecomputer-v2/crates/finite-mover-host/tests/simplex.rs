//! The SimpleX carry: `stream` stages the old agent's SimpleX conversation
//! for the bound agent (`.finite-stage/simplex/`) only when exactly one
//! owner was approved and both identity databases are plain, settled
//! SQLite under the cap; otherwise nothing of it, and the summary says why.
//! Once carried, the old SimpleX is sealed so the old core never runs
//! again: its folder renamed aside and managed SimpleX turned off.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use finite_mover_host::classify::{BrainScope, Class, EntryKind, classify};
use finite_mover_host::command::{parse, run};
use finite_mover_host::stage::SUMMARY_ENTRY;
use serde_json::{Value, json};

const ID: &str = "runtime_sx";
const SQLITE: &[u8] = b"SQLite format 3\0";
const CHAT: &str = ".finite-stage/simplex/chat.db";
const AGENT: &str = ".finite-stage/simplex/agent.db";
const OWNER: &str = ".finite-stage/simplex/owner.json";
const APPROVED: &str = "agent/hermes-home/platforms/pairing/simplex-approved.json";
const LEGACY_APPROVED: &str = "agent/hermes-home/pairing/simplex-approved.json";
const CONFIG: &str = "agent/hermes-home/config.yaml";
const STAGE: &str = "proj_a.p1.1";
const SEALED: &str = "agent/simplex.moved-proj_a.p1.1";
/// The Hermes config, managed SimpleX on, beside settings that must stay.
const CONFIG_ON: &str = "model:\n  default: gpt-x\ntimezone: America/Chicago\ngateway:\n  platforms:\n    simplex:\n      enabled: true\n      extra:\n        finite_managed: true\n    telegram:\n      enabled: true\n";

struct Tree {
    _dir: tempfile::TempDir,
    work_root: PathBuf,
    root: PathBuf,
}

impl Tree {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("mover-simplex-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let work_root = dir.path().join("work");
        let root = work_root.join("kata").join(ID);
        let tree = Self {
            _dir: dir,
            work_root,
            root,
        };
        tree.write("workspace/notes.md", b"notes\n");
        tree
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn path(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    /// A SimpleX bot as finite.computer ran it, stopped, with one owner.
    fn simplex(&self) {
        self.write("agent/simplex/identity_chat.db", &db(b"chat rows"));
        self.write("agent/simplex/identity_agent.db", &db(b"agent queues"));
        self.write(
            "agent/simplex/address.json",
            br#"{"address":"https://simplex.chat/a#x"}"#,
        );
        self.write("agent/simplex/files/photo.jpg", b"jpeg");
        self.write("agent/simplex/smp-relays.json", b"[]");
        self.write(
            APPROVED,
            br#"{"3": {"user_name": "Owner", "approved_at": 1}}"#,
        );
        self.write(CONFIG, CONFIG_ON.as_bytes());
    }

    /// `stream`, for the stage `stage` (the mover always names it).
    fn stream_for(&self, stage: Option<&str>) -> Vec<u8> {
        let mut out = Vec::new();
        let words = match stage {
            Some(stage) => format!("stream {ID} --stage {stage}"),
            None => format!("stream {ID}"),
        };
        run(
            &self.work_root,
            parse(words.split_whitespace()).unwrap(),
            &mut out,
        )
        .unwrap();
        out
    }

    fn stream(&self) -> Vec<u8> {
        self.stream_for(Some(STAGE))
    }

    fn read(&self, path: &str) -> Vec<u8> {
        fs::read(self.path(path)).unwrap()
    }
}

fn db(rows: &[u8]) -> Vec<u8> {
    [SQLITE, rows].concat()
}

fn entries(stage: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(stage)).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut entry = archive.by_index(index).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            (entry.name().to_owned(), bytes)
        })
        .collect()
}

fn summary(stage: &BTreeMap<String, Vec<u8>>) -> Value {
    serde_json::from_slice(&stage[SUMMARY_ENTRY]).unwrap()
}

/// Nothing of `agent/simplex/` and nothing under `.finite-stage/simplex/`.
fn carried_nothing(stage: &BTreeMap<String, Vec<u8>>) -> bool {
    stage.keys().all(|name| {
        !name.starts_with("agent/simplex") && !name.starts_with(".finite-stage/simplex")
    })
}

#[test]
fn a_lone_owners_conversation_is_carried_and_nothing_else_of_simplex() {
    let tree = Tree::new();
    tree.simplex();
    let stage = entries(&tree.stream());
    assert_eq!(stage[CHAT], db(b"chat rows"));
    assert_eq!(stage[AGENT], db(b"agent queues"));
    let owner: Value = serde_json::from_slice(&stage[OWNER]).unwrap();
    assert_eq!(owner, json!({"contact": 3}));
    let simplex: Vec<&String> = stage
        .keys()
        .filter(|name| name.contains("simplex"))
        .collect();
    assert_eq!(simplex, [AGENT, CHAT, OWNER], "no address, files or relays");
    let summary = summary(&stage);
    assert_eq!(
        summary["simplex"],
        json!({"carried": true, "sealed": SEALED})
    );
    let bytes = (stage[CHAT].len() + stage[AGENT].len() + stage[OWNER].len()) as u64;
    assert_eq!(
        summary["classes"]["simplex"],
        json!({"files": 3, "bytes": bytes})
    );
    // The source stays behind, as every secret does.
    let left = summary["left"].as_array().unwrap();
    assert!(
        left.iter()
            .any(|line| line["path"] == "agent/simplex" && line["why"] == "simplex")
    );
}

#[test]
fn a_stage_seals_the_old_simplex_where_every_later_start_sees_it() {
    let tree = Tree::new();
    tree.simplex();
    let before: BTreeMap<String, Vec<u8>> = [
        "identity_chat.db",
        "identity_agent.db",
        "address.json",
        "files/photo.jpg",
    ]
    .iter()
    .map(|name| {
        (
            name.to_string(),
            tree.read(&format!("agent/simplex/{name}")),
        )
    })
    .collect();
    tree.stream();
    // Renamed aside whole, nothing deleted.
    assert!(!tree.path("agent/simplex").exists());
    for (name, bytes) in &before {
        assert_eq!(&tree.read(&format!("{SEALED}/{name}")), bytes, "{name}");
    }
    // Managed SimpleX off; every other setting as it was.
    let config: serde_yaml::Value = serde_yaml::from_slice(&tree.read(CONFIG)).unwrap();
    let mut expected: serde_yaml::Value = serde_yaml::from_str(CONFIG_ON).unwrap();
    expected["gateway"]["platforms"]["simplex"]["enabled"] = serde_yaml::Value::Bool(false);
    assert_eq!(config, expected);
}

#[test]
fn a_second_stage_carries_it_again_and_seals_nothing_more() {
    use std::os::unix::fs::MetadataExt;
    let tree = Tree::new();
    tree.simplex();
    let first = entries(&tree.stream());
    let sealed_config = tree.read(CONFIG);
    let config_before = fs::metadata(tree.path(CONFIG)).unwrap();
    // Staff retried: a new attempt's stage, from the sealed folder.
    let second = tree.stream_for(Some("proj_a.p1.2"));
    let stage = entries(&second);
    for name in [CHAT, AGENT, OWNER] {
        assert_eq!(stage[name], first[name], "{name}");
    }
    assert_eq!(
        summary(&stage)["simplex"],
        json!({"carried": true, "sealed": SEALED})
    );
    assert!(!tree.path("agent/simplex.moved-proj_a.p1.2").exists());
    assert!(!tree.path("agent/simplex").exists());
    assert_eq!(tree.read(CONFIG), sealed_config);
    let config_after = fs::metadata(tree.path(CONFIG)).unwrap();
    assert_eq!(
        (
            config_after.ino(),
            config_after.mtime_nsec(),
            config_after.mtime()
        ),
        (
            config_before.ino(),
            config_before.mtime_nsec(),
            config_before.mtime()
        ),
        "the config isn't written again"
    );
    // From then on, every stage of the tree is the same.
    assert_eq!(tree.stream_for(Some("proj_a.p1.2")), second);
}

#[test]
fn a_carry_refused_or_unnamed_seals_nothing() {
    type Spoil = fn(&Tree);
    let cases: [(&str, Option<&str>, Spoil); 3] = [
        ("several-contacts", Some(STAGE), |tree| {
            tree.write(APPROVED, br#"{"3": {}, "4": {}}"#);
        }),
        ("journal-present", Some(STAGE), |tree| {
            tree.write("agent/simplex/identity_chat.db-journal", b"");
        }),
        // Without a stage id there is nothing to name the seal by.
        ("no-stage-id", None, |_| {}),
    ];
    for (why, stage, spoil) in cases {
        let tree = Tree::new();
        tree.simplex();
        spoil(&tree);
        let stage = entries(&tree.stream_for(stage));
        assert!(carried_nothing(&stage), "{why}");
        assert_eq!(
            summary(&stage)["simplex"],
            json!({"carried": false, "why": why})
        );
        assert!(
            tree.path("agent/simplex/identity_chat.db").exists(),
            "{why}"
        );
        assert!(!tree.path(SEALED).exists(), "{why}");
        assert_eq!(tree.read(CONFIG), CONFIG_ON.as_bytes(), "{why}");
    }
}

/// `finite-agentd/simplex_runtime.py`, the old runtime's SimpleX supervisor,
/// run on a tree with a stand-in `simplex-chat` that records every start.
/// Hermes' own modules, which it imports but doesn't need here, are stubs.
struct OldRuntime {
    dir: tempfile::TempDir,
}

impl OldRuntime {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("old-runtime-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let file = |path: &str, contents: &str| {
            let path = dir.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            path
        };
        file("stubs/websockets.py", "");
        file("stubs/gateway/__init__.py", "");
        file(
            "stubs/gateway/pairing.py",
            "class PairingStore:\n    @staticmethod\n    def looks_like_request_id(value):\n        return False\n\n    def list_pending(self, platform):\n        return []\n",
        );
        let core = file(
            "bin/simplex-chat",
            "#!/bin/sh\necho \"$@\" >> \"$CORE_STARTS\"\nexec sleep 30\n",
        );
        fs::set_permissions(&core, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        Self { dir }
    }

    fn command(&self, tree: &Tree, operation: &str) -> Command {
        let script = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../finite-agentd/simplex_runtime.py"
        );
        let path = format!(
            "{}:{}",
            self.dir.path().join("bin").display(),
            std::env::var("PATH").unwrap()
        );
        let mut command = Command::new("python3");
        command
            .arg(script)
            .arg(operation)
            .env("PYTHONPATH", self.dir.path().join("stubs"))
            .env("PATH", path)
            .env("CORE_STARTS", self.starts())
            .env("HERMES_HOME", tree.path("agent/hermes-home"))
            .env("FINITECHAT_HOME", tree.path("agent"))
            .stdin(Stdio::null());
        command
    }

    fn starts(&self) -> PathBuf {
        self.dir.path().join("core-starts")
    }

    /// Run `supervise` until a core starts or `quiet` passes, then stop it
    /// as the runtime does (TERM). Whether a core started.
    fn supervise(&self, tree: &Tree, quiet: Duration) -> bool {
        let mut child = self
            .command(tree, "supervise")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        while !self.starts().exists() && started.elapsed() < quiet {
            std::thread::sleep(Duration::from_millis(50));
        }
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap();
        child.wait().unwrap();
        self.starts().exists()
    }

    fn status(&self, tree: &Tree) -> Value {
        let out = self.command(tree, "status").output().unwrap();
        serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
            panic!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        })
    }
}

#[test]
fn a_sealed_old_agent_starts_no_simplex_core() {
    // Unsealed, the same harness sees the old core start: the control.
    let unsealed = Tree::new();
    unsealed.simplex();
    let runtime = OldRuntime::new();
    assert!(
        runtime.supervise(&unsealed, Duration::from_secs(20)),
        "the control's core never started"
    );
    assert_eq!(runtime.status(&unsealed)["enabled"], true);

    // Staged and sealed: a start of the old agent starts no core through
    // the supervisor's first checks (every 2 s), and its status says
    // SimpleX is off.
    let sealed = Tree::new();
    sealed.simplex();
    sealed.stream();
    let runtime = OldRuntime::new();
    assert!(
        !runtime.supervise(&sealed, Duration::from_secs(5)),
        "a sealed agent started its core"
    );
    let status = runtime.status(&sealed);
    assert_eq!(status["enabled"], false, "{status}");
    assert!(status.get("error").is_none(), "{status}");
}

/// Each way a carry is refused, and why the summary gives.
#[test]
fn a_conversation_that_cant_be_carried_safely_carries_nothing_and_says_why() {
    type Spoil = fn(&Tree);
    let cases: [(&str, Spoil); 10] = [
        ("several-contacts", |tree| {
            tree.write(APPROVED, br#"{"3": {}, "4": {}}"#);
        }),
        ("several-contacts", |tree| {
            tree.write(LEGACY_APPROVED, br#"{"5": {}}"#);
        }),
        ("no-owner", |tree| tree.write(APPROVED, b"{}")),
        ("pairing-unreadable", |tree| tree.write(APPROVED, b"[3]")),
        ("pairing-unreadable", |tree| {
            tree.write(APPROVED, br#"{"03": {}}"#);
        }),
        ("database-missing", |tree| {
            fs::remove_file(tree.path("agent/simplex/identity_agent.db")).unwrap();
        }),
        ("database-not-plain-sqlite", |tree| {
            tree.write("agent/simplex/identity_chat.db", b"\x8f\x01encrypted page");
        }),
        ("journal-present", |tree| {
            tree.write("agent/simplex/identity_chat.db-journal", b"");
        }),
        ("journal-present", |tree| {
            tree.write("agent/simplex/identity_agent.db-wal", b"wal");
        }),
        ("too-large", |tree| {
            let file = fs::OpenOptions::new()
                .write(true)
                .open(tree.path("agent/simplex/identity_chat.db"))
                .unwrap();
            // Sparse: past the cap without writing it.
            file.set_len(1 << 30).unwrap();
        }),
    ];
    for (why, spoil) in cases {
        let tree = Tree::new();
        tree.simplex();
        spoil(&tree);
        let stage = entries(&tree.stream());
        assert!(tree.path("agent/simplex").exists(), "{why}: nothing sealed");
        assert!(carried_nothing(&stage), "{why}: {:?}", stage.keys());
        assert_eq!(
            summary(&stage)["simplex"],
            json!({"carried": false, "why": why}),
            "{why}"
        );
    }
}

#[test]
fn an_agent_without_simplex_says_so() {
    let tree = Tree::new();
    let stage = entries(&tree.stream());
    assert!(carried_nothing(&stage));
    assert_eq!(
        summary(&stage)["simplex"],
        json!({"carried": false, "why": "no-simplex"})
    );
    assert!(!Path::new(&tree.path("agent/simplex")).exists());
}

#[test]
fn the_uploaders_second_check_lets_the_carried_entries_through() {
    // finite-mover's uploader refuses any entry the classifier calls secret.
    for name in [CHAT, AGENT, OWNER] {
        let parts: Vec<&[u8]> = name.split('/').map(str::as_bytes).collect();
        let class = classify(&parts, EntryKind::File, BrainScope::None).class;
        assert_ne!(class, Class::Secret, "{name}");
    }
}
