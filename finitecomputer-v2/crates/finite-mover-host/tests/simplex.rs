//! The SimpleX carry: `stream` stages the old agent's SimpleX conversation
//! for the bound agent (`.finite-stage/simplex/`) only when exactly one
//! owner was approved and both identity databases are plain, settled
//! SQLite under the cap; otherwise nothing of it, and the summary says why.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

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
    }

    fn stream(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let operation = parse(format!("stream {ID}").split_whitespace()).unwrap();
        run(&self.work_root, operation, &mut out).unwrap();
        out
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
    assert_eq!(summary["simplex"], json!({"carried": true}));
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
fn staging_twice_gives_the_same_bytes() {
    let tree = Tree::new();
    tree.simplex();
    let first = tree.stream();
    assert!(entries(&first).contains_key(CHAT));
    assert_eq!(tree.stream(), first);
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
