//! The reader against a fixture tree holding every secret path of
//! FIN-463 §4: `stream` holds none of them, `sizes` equals what `stream`
//! stages in each class, the config copy holds no token, and `secrets`
//! reads exactly the three values.

mod fixture;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;
use std::process::Command;

use finite_mover_host::classify::{BrainScope, Class, EntryKind, classify};
use finite_mover_host::command::{Operation, parse, run};
use finite_mover_host::error::HostError;
use finite_mover_host::stage::SUMMARY_ENTRY;
use finite_mover_host::summary::Tally;
use fixture::{
    Fixture, ID, MARKER, OPENROUTER_KEY, OWNED_BRAIN, SECRET_HEX, STAGED, TELEGRAM_TOKEN,
};
use serde_json::Value;

const CONFIG_COPY: &str = "agent/hermes-home/config.public.yaml";

fn op(words: &str) -> Operation {
    parse(words.split_whitespace()).unwrap()
}

fn run_bytes(fixture: &Fixture, words: &str) -> Result<Vec<u8>, HostError> {
    let mut out = Vec::new();
    run(&fixture.work_root, op(words), &mut out)?;
    Ok(out)
}

fn stream(fixture: &Fixture) -> Vec<u8> {
    run_bytes(fixture, &format!("stream {ID} --brain {OWNED_BRAIN}")).unwrap()
}

fn sizes(fixture: &Fixture) -> Value {
    let out = run_bytes(fixture, &format!("sizes {ID} --brain {OWNED_BRAIN}")).unwrap();
    serde_json::from_slice(&out).unwrap()
}

/// Every entry of the stage, by name.
fn entries(stage: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(stage)).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut entry = archive.by_index(index).unwrap();
            assert_eq!(entry.compression(), zip::CompressionMethod::Stored);
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            (entry.name().to_owned(), bytes)
        })
        .collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn the_stage_holds_no_secret() {
    let fixture = fixture::build();
    let stage = stream(&fixture);
    for needle in [
        MARKER,
        TELEGRAM_TOKEN,
        OPENROUTER_KEY,
        SECRET_HEX,
        "ghp_",
        "PRIVATE KEY",
        "root:",
    ] {
        assert!(
            !contains(&stage, needle.as_bytes()),
            "the stage holds {needle}"
        );
    }
    for name in entries(&stage).keys() {
        let parts: Vec<&[u8]> = name.split('/').map(str::as_bytes).collect();
        let verdict = classify(&parts, EntryKind::File, BrainScope::None);
        assert_ne!(verdict.class, Class::Secret, "{name} is a secret path");
    }
}

#[test]
fn the_stage_is_exactly_history_work_and_the_config_copy() {
    let fixture = fixture::build();
    let entries = entries(&stream(&fixture));
    let mut expected: BTreeSet<&str> = STAGED.iter().map(|(path, _)| *path).collect();
    expected.extend([CONFIG_COPY, SUMMARY_ENTRY]);
    assert_eq!(
        entries.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        expected
    );
    for (path, _) in STAGED {
        let source = std::fs::read(fixture.tree.join(path)).unwrap();
        assert_eq!(entries[*path], source, "{path} is staged byte for byte");
    }
}

#[test]
fn stream_stages_exactly_its_summary_and_sizes_estimates_it() {
    let fixture = fixture::build();
    let measured = sizes(&fixture);
    let stage = stream(&fixture);
    let entries = entries(&stage);
    let embedded: Value = serde_json::from_slice(&entries[SUMMARY_ENTRY]).unwrap();
    let summary: BTreeMap<String, Tally> =
        serde_json::from_value(embedded["classes"].clone()).unwrap();
    let mut streamed: BTreeMap<String, Tally> = BTreeMap::new();
    for (name, bytes) in &entries {
        if name == SUMMARY_ENTRY {
            continue;
        }
        let class = if name == CONFIG_COPY {
            "config"
        } else {
            STAGED.iter().find(|(path, _)| path == name).unwrap().1
        };
        let tally = streamed.entry(class.to_owned()).or_default();
        tally.files += 1;
        tally.bytes += bytes.len() as u64;
    }
    for class in ["config", "history", "work"] {
        assert_eq!(summary[class], streamed[class], "{class}");
    }
    let staged: Tally = serde_json::from_value(embedded["staged"].clone()).unwrap();
    assert_eq!(
        staged.bytes,
        streamed.values().map(|tally| tally.bytes).sum::<u64>()
    );
    // Every secret path is classified secret, not merely left out.
    let secrets = (fixture::SECRET_FILES.len() + fixture::SECRET_TEXT.len()) as u64;
    assert_eq!(summary["secret"].files, secrets);
    // What stayed behind is counted too, and named with its reason.
    for class in ["secret", "cache", "runtime"] {
        assert!(summary[class].files > 0, "{class} counted");
    }
    let left: BTreeSet<(String, String)> = embedded["left"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| {
            (
                line["path"].as_str().unwrap().to_owned(),
                line["why"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    for (path, why) in [
        ("agent/identity", "nostr-identity"),
        ("agent/hermes-home/config.yaml", "hermes-config"),
        ("workspace/notes/keys.md", "token-in-text"),
        ("workspace/app/node_modules", "cache-dir"),
        ("workspace/app/dist", "build-output"),
        ("workspace/.config", "hidden-folder"),
        ("workspace/finite-export", "earlier-export"),
        ("agent/agentd", "agentd"),
        ("workspace/finitebrain/brain-org", "not-owned-brain"),
        ("workspace/finitebrain/broken", "not-owned-brain"),
        ("workspace/passwd", "not-a-file"),
        ("workspace/other", "not-a-file"),
        ("workspace/pipe", "not-a-file"),
        ("workspace/report.txt", "not-a-file"),
        ("workspace/bad\u{fffd}name.txt", "uncarried-name"),
        ("agent/hermes-home/config.public.yaml", "replaced-by-copy"),
        ("agent/hermes-home/logs", "runtime"),
    ] {
        assert!(
            left.contains(&(path.to_owned(), why.to_owned())),
            "left lists {path} ({why})"
        );
    }

    // `sizes` reads no file, so it differs from the stage in exactly two
    // ways: a text file holding a credential counts in its path's class,
    // and there is no config copy. Everything else is the same.
    let estimate: BTreeMap<String, Tally> =
        serde_json::from_value(measured["classes"].clone()).unwrap();
    let in_text: Vec<u64> = fixture::SECRET_TEXT
        .iter()
        .map(|(path, _)| std::fs::metadata(fixture.tree.join(path)).unwrap().len())
        .collect();
    let text = Tally {
        files: in_text.len() as u64,
        bytes: in_text.iter().sum(),
    };
    for class in ["cache", "runtime"] {
        assert_eq!(estimate[class], summary[class], "{class}");
    }
    assert_eq!(estimate["config"], Tally::default());
    assert_eq!(
        estimate["secret"].files,
        summary["secret"].files - text.files
    );
    assert_eq!(
        estimate["secret"].bytes,
        summary["secret"].bytes - text.bytes
    );
    let staged_by_path = |tallies: &BTreeMap<String, Tally>| {
        (
            tallies["history"].files + tallies["work"].files,
            tallies["history"].bytes + tallies["work"].bytes,
        )
    };
    let (files, bytes) = staged_by_path(&summary);
    assert_eq!(
        staged_by_path(&estimate),
        (files + text.files, bytes + text.bytes)
    );
}

/// Every regular file in `root`, made unreadable (or readable again).
fn set_readable(root: &Path, readable: bool) {
    use std::os::unix::fs::PermissionsExt;
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                let mode = if readable { 0o644 } else { 0o000 };
                std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(mode))
                    .unwrap();
            }
        }
    }
}

#[test]
fn sizes_opens_no_regular_file() {
    let fixture = fixture::build();
    let before = sizes(&fixture);
    set_readable(&fixture.tree, false);
    if run_bytes(&fixture, &format!("stream {ID}")).is_ok() {
        // As root, a file's mode does not stop a read: nothing to prove.
        set_readable(&fixture.tree, true);
        eprintln!("skipped: unreadable files can still be read here (root)");
        return;
    }
    // Not one file can be opened, and the estimate is unchanged.
    let after = run_bytes(&fixture, &format!("sizes {ID} --brain {OWNED_BRAIN}"));
    set_readable(&fixture.tree, true);
    let after: Value =
        serde_json::from_slice(&after.expect("sizes answers without opening a file")).unwrap();
    assert_eq!(after, before);
}

#[test]
fn the_config_copy_holds_the_allow_list_and_no_token() {
    let fixture = fixture::build();
    let entries = entries(&stream(&fixture));
    let copy = &entries[CONFIG_COPY];
    assert!(!contains(copy, MARKER.as_bytes()) && !contains(copy, TELEGRAM_TOKEN.as_bytes()));
    let copy: serde_yaml::Value = serde_yaml::from_slice(copy).unwrap();
    let expected: serde_yaml::Value = serde_yaml::from_str(
        "timezone: America/Chicago\n\
         agent: {personality: kind and brief}\n\
         model: {default: glm-5-3-flash}\n\
         gateway: {platforms: {telegram: {enabled: true}}}\n",
    )
    .unwrap();
    assert_eq!(copy, expected);
}

#[test]
fn only_brains_named_as_owned_are_staged() {
    let fixture = fixture::build();
    let out = run_bytes(&fixture, &format!("stream {ID}")).unwrap();
    assert!(
        entries(&out)
            .keys()
            .all(|name| !name.starts_with("workspace/finitebrain/")),
        "no brain without --brain"
    );
}

/// The `left` lines of a stage's summary, as (path, why).
fn left_of(stage: &[u8]) -> BTreeSet<(String, String)> {
    let embedded: Value = serde_json::from_slice(&entries(stage)[SUMMARY_ENTRY]).unwrap();
    embedded["left"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| {
            (
                line["path"].as_str().unwrap().to_owned(),
                line["why"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// FIN-499: an owned brain comes whole, but for its `.finitebrain/`
/// control folder, with the name file fragment's import reads; a brain
/// the person doesn't own stays, and so does an old file where the name
/// file goes.
#[test]
fn an_owned_brain_comes_whole_with_its_name_file_and_no_control_folder() {
    let fixture = fixture::build();
    let brains = fixture.tree.join("workspace/finitebrain");
    std::fs::create_dir_all(brains.join("brain-owned/wiki")).unwrap();
    std::fs::write(brains.join("brain-owned/wiki/topic.md"), "# Topic\n").unwrap();
    std::fs::write(brains.join("brain-owned/.finitebrain/state.json"), "{}").unwrap();
    std::fs::write(brains.join("brain-owned.json"), r#"{"name":"stale"}"#).unwrap();
    let name = "Owned notes: ünïcode & spaces";
    let stage = run_bytes(
        &fixture,
        &format!(
            "stream {ID} --brain {OWNED_BRAIN}:{}",
            hex::encode(name.as_bytes())
        ),
    )
    .unwrap();
    let files = entries(&stage);
    let brain_entries: Vec<&str> = files
        .keys()
        .filter(|name| name.starts_with("workspace/finitebrain/"))
        .map(String::as_str)
        .collect();
    assert_eq!(
        brain_entries,
        [
            "workspace/finitebrain/brain-owned.json",
            "workspace/finitebrain/brain-owned/page.md",
            "workspace/finitebrain/brain-owned/wiki/topic.md",
        ],
        "the owned brain without .finitebrain/, its name file, and no other brain"
    );
    let named: Value =
        serde_json::from_slice(&files["workspace/finitebrain/brain-owned.json"]).unwrap();
    assert_eq!(named, serde_json::json!({ "name": name }));
    let left = left_of(&stage);
    for (path, why) in [
        (
            "workspace/finitebrain/brain-owned/.finitebrain",
            "cache-dir",
        ),
        ("workspace/finitebrain/brain-org", "not-owned-brain"),
        ("workspace/finitebrain/broken", "not-owned-brain"),
        ("workspace/finitebrain/brain-owned.json", "replaced-by-copy"),
    ] {
        assert!(
            left.contains(&(path.to_owned(), why.to_owned())),
            "left lists {path} ({why}): {left:?}"
        );
    }
    // The summary counts the name file with the work it came with.
    let embedded: Value = serde_json::from_slice(&files[SUMMARY_ENTRY]).unwrap();
    let staged: u64 = files
        .iter()
        .filter(|(name, _)| name.as_str() != SUMMARY_ENTRY)
        .map(|(_, bytes)| bytes.len() as u64)
        .sum();
    assert_eq!(embedded["staged"]["bytes"].as_u64(), Some(staged));

    // Named but not sent a name: the brain comes, and the old file at
    // its name file's path is the brain's work as before.
    let unnamed =
        entries(&run_bytes(&fixture, &format!("stream {ID} --brain {OWNED_BRAIN}")).unwrap());
    assert_eq!(
        unnamed["workspace/finitebrain/brain-owned.json"],
        br#"{"name":"stale"}"#
    );
}

/// A brain id whose name file would be named like a credential gets no
/// name file, and the stage still goes (the uploader would refuse it).
#[test]
fn a_name_file_never_takes_a_credentials_name() {
    let fixture = fixture::build();
    let brain = fixture.tree.join("workspace/finitebrain/identity");
    std::fs::create_dir_all(brain.join(".finitebrain")).unwrap();
    std::fs::write(
        brain.join(".finitebrain/brain-directory.json"),
        r#"{"version":"1","brain":{"id":"identity","kind":"personal","name":"identity","owner_npub":null}}"#,
    )
    .unwrap();
    std::fs::write(brain.join("page.md"), "a page\n").unwrap();
    let stage = run_bytes(
        &fixture,
        &format!("stream {ID} --brain identity:{}", hex::encode("Mine")),
    )
    .unwrap();
    let entries = entries(&stage);
    assert!(entries.contains_key("workspace/finitebrain/identity/page.md"));
    assert!(!entries.contains_key("workspace/finitebrain/identity.json"));
}

#[test]
fn a_running_agent_is_measured_but_never_staged_or_read_for_secrets() {
    let fixture = fixture::build();
    let lease = File::open(fixture.tree.join("agent/client.sqlite3.writer-lease")).unwrap();
    lease.lock().unwrap();
    assert!(matches!(
        run_bytes(&fixture, &format!("stream {ID}")),
        Err(HostError::LeaseHeld)
    ));
    assert!(matches!(
        run_bytes(&fixture, &format!("secrets {ID} nostr_nsec")),
        Err(HostError::LeaseHeld)
    ));
    assert!(run_bytes(&fixture, &format!("sizes {ID}")).is_ok());
    lease.unlock().unwrap();
    assert!(run_bytes(&fixture, &format!("stream {ID}")).is_ok());
}

/// Writes the stage, and the first time bytes reach it, appends to a file
/// the walk has already copied: an old agent restarted mid-stream.
struct Restarted {
    file: std::path::PathBuf,
    tampered: bool,
}

impl std::io::Write for Restarted {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if !self.tampered {
            let mut file = std::fs::OpenOptions::new().append(true).open(&self.file)?;
            file.write_all(b"written after it was copied\n")?;
            self.tampered = true;
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_tree_written_while_it_is_streamed_is_refused() {
    let fixture = fixture::build();
    // Past the stream's buffer, so bytes reach the writer mid-walk.
    std::fs::write(
        fixture.tree.join("workspace/photos/zz-big.jpg"),
        vec![0u8; 3 << 20],
    )
    .unwrap();
    let out = Restarted {
        file: fixture.tree.join("agent/hermes-home/SOUL.md"),
        tampered: false,
    };
    let result = run(&fixture.work_root, op(&format!("stream {ID}")), out);
    assert!(
        matches!(&result, Err(HostError::TreeChanged(path)) if path.ends_with("SOUL.md")),
        "{result:?}"
    );
}

/// Writes the stage, and once a megabyte of it has arrived, while a large
/// text file is being copied, puts a token into the rest of that file.
struct RacingWriter {
    file: std::path::PathBuf,
    received: Vec<u8>,
}

impl std::io::Write for RacingWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        use std::io::{Seek, SeekFrom};
        let before = self.received.len();
        self.received.extend_from_slice(buffer);
        if before < (1 << 20) && self.received.len() >= (1 << 20) {
            let mut file = std::fs::OpenOptions::new().write(true).open(&self.file)?;
            file.seek(SeekFrom::Start(3 << 20))?;
            file.write_all(b" ghp_abcdefghij0123456789 ")?;
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_text_file_written_while_it_is_streamed_never_reaches_the_stage() {
    let dir = tempfile::Builder::new()
        .prefix("mover-host-race-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let tree = dir.path().join("kata/rt_race");
    std::fs::create_dir_all(tree.join("workspace")).unwrap();
    let big = tree.join("workspace/big.txt");
    std::fs::write(&big, "clean notes line\n".repeat(4 << 16)).unwrap();
    let mut out = RacingWriter {
        file: big,
        received: Vec::new(),
    };
    let result = run(dir.path(), op("stream rt_race"), &mut out);
    assert!(
        matches!(&result, Err(HostError::TreeChanged(path)) if path.ends_with("big.txt")),
        "{result:?}"
    );
    assert!(
        !contains(&out.received, b"ghp_"),
        "the token written after the check never left the host"
    );
}

#[test]
fn a_stopped_tree_streams_the_same_bytes_every_time() {
    // The uploader resumes by comparing parts of a second stream with the
    // first: an unchanged tree must stream identically.
    let fixture = fixture::build();
    assert_eq!(stream(&fixture), stream(&fixture));
}

#[test]
fn secrets_reads_the_three_values() {
    let fixture = fixture::build();
    let out = run_bytes(
        &fixture,
        &format!("secrets {ID} telegram_bot_token,openrouter_key,nostr_nsec"),
    )
    .unwrap();
    let answer: Value = serde_json::from_slice(&out).unwrap();
    let values: BTreeMap<&str, &str> = answer["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["kind"].as_str().unwrap(),
                item["value"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(values["telegram_bot_token"], TELEGRAM_TOKEN);
    assert_eq!(values["openrouter_key"], OPENROUTER_KEY);
    let (hrp, secret) = bech32::decode(values["nostr_nsec"]).unwrap();
    assert_eq!(
        (hrp.as_str(), hex::encode(secret)),
        ("nsec", SECRET_HEX.to_owned())
    );
    assert_eq!(answer["absent"], serde_json::json!([]));

    // A key file that is a hard link is not read.
    let identity = fixture.tree.join("agent/identity/identity.json");
    std::fs::hard_link(&identity, fixture.work_root.join("identity-elsewhere.json")).unwrap();
    let out = run_bytes(&fixture, &format!("secrets {ID} nostr_nsec")).unwrap();
    let answer: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(answer["absent"], serde_json::json!(["nostr_nsec"]));
    std::fs::remove_file(fixture.work_root.join("identity-elsewhere.json")).unwrap();

    // An agent with none of them reports each as absent.
    for path in [
        "agent/identity/identity.json",
        "agent/hermes-home/.env",
        "agent/hermes-home/config.yaml",
    ] {
        std::fs::remove_file(fixture.tree.join(path)).unwrap();
    }
    let out = run_bytes(
        &fixture,
        &format!("secrets {ID} telegram_bot_token,openrouter_key,nostr_nsec"),
    )
    .unwrap();
    let answer: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(answer["items"], serde_json::json!([]));
    assert_eq!(
        answer["absent"],
        serde_json::json!(["telegram_bot_token", "openrouter_key", "nostr_nsec"])
    );
}

#[test]
fn a_tree_reached_through_a_link_is_refused() {
    let fixture = fixture::build();
    let link = fixture.work_root.join("kata").join("linked");
    std::os::unix::fs::symlink(&fixture.tree, &link).unwrap();
    assert!(matches!(
        run_bytes(&fixture, "sizes linked"),
        Err(HostError::NoTree)
    ));
    assert!(matches!(
        run_bytes(&fixture, "sizes missing"),
        Err(HostError::NoTree)
    ));
}

/// The forced command: the operation arrives in `SSH_ORIGINAL_COMMAND`.
fn ssh(work_root: &Path, original: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_finite-mover-host"))
        .args([
            "--work-root".as_ref(),
            work_root.as_os_str(),
            "--ssh".as_ref(),
        ])
        .env("SSH_ORIGINAL_COMMAND", original)
        .output()
        .unwrap()
}

#[test]
fn the_forced_command_runs_only_its_operations() {
    let fixture = fixture::build();
    let sized = ssh(
        &fixture.work_root,
        &format!("sizes {ID} --brain {OWNED_BRAIN}"),
    );
    assert!(sized.status.success());
    let summary: Value = serde_json::from_slice(&sized.stdout).unwrap();
    assert_eq!(summary["id"], ID);
    let streamed = ssh(
        &fixture.work_root,
        &format!("stream {ID} --brain {OWNED_BRAIN}"),
    );
    assert!(streamed.status.success());
    assert!(entries(&streamed.stdout).contains_key(SUMMARY_ENTRY));
    for (original, code) in [
        ("cat /etc/passwd", 64),
        (&format!("stream ../{ID}") as &str, 65),
        ("sizes x;id", 65),
        (&format!("sizes {ID} --brain a/b") as &str, 64),
        ("sizes no_such_agent", 65),
    ] {
        let output = ssh(&fixture.work_root, original);
        assert_eq!(output.status.code(), Some(code), "{original}");
        assert!(output.stdout.is_empty(), "{original}");
    }
}

/// Zip64 at real sizes: a file over 4 GiB, an entry after it at an offset
/// over 4 GiB, and more than 65,535 entries. Writes about 4.5 GB; run by
/// name with `--ignored`.
#[test]
#[ignore = "writes a 4.5 GB stage"]
fn zip64_carries_a_file_over_4_gib_and_70k_entries() {
    let fixture = fixture::build();
    let big = File::create(fixture.tree.join("workspace/big.img")).unwrap();
    big.set_len(4_500_000_000).unwrap();
    let many = fixture.tree.join("workspace/zz-many");
    std::fs::create_dir(&many).unwrap();
    for index in 0..70_000 {
        std::fs::write(many.join(format!("{index:05}.md")), b"x").unwrap();
    }
    std::fs::write(
        fixture.tree.join("workspace/zzz-after.md"),
        b"after the big file",
    )
    .unwrap();
    let stage_path = fixture.work_root.join("stage.zip");
    let out = std::io::BufWriter::new(File::create(&stage_path).unwrap());
    run(&fixture.work_root, op(&format!("stream {ID}")), out).unwrap();
    let mut archive = zip::ZipArchive::new(File::open(&stage_path).unwrap()).unwrap();
    assert!(archive.len() > 70_000);
    assert_eq!(
        archive.by_name("workspace/big.img").unwrap().size(),
        4_500_000_000
    );
    let mut after = archive.by_name("workspace/zzz-after.md").unwrap();
    assert!(after.data_start() > u64::from(u32::MAX));
    let mut text = String::new();
    after.read_to_string(&mut text).unwrap();
    assert_eq!(text, "after the big file");
}

/// export.py's `build --raw` reference (tools/finite-raw-export), run on
/// the same tree.
fn raw_exporter() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tools/finite-raw-export")
}

#[test]
fn the_raw_exporter_matches_the_classifier() {
    let fixture = fixture::build();
    let staged = entries(&stream(&fixture));
    let out = fixture.work_root.join("raw.zip");
    let run = Command::new("python3")
        .arg("-I")
        .arg(raw_exporter().join("raw_export.py"))
        .args(["--root".as_ref(), fixture.tree.as_os_str()])
        .args(["--out".as_ref(), out.as_os_str()])
        .args(["--brain", OWNED_BRAIN, "--id", ID])
        .output()
        .expect("python3 runs the raw-folder exporter");
    assert!(
        run.status.success(),
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let raw = entries(&std::fs::read(&out).unwrap());
    // The same names, the same bytes: the config copy and the summary too.
    assert_eq!(
        raw.keys().collect::<Vec<_>>(),
        staged.keys().collect::<Vec<_>>()
    );
    for (name, bytes) in &staged {
        if name == SUMMARY_ENTRY {
            let mut ours: Value = serde_json::from_slice(bytes).unwrap();
            let theirs: Value = serde_json::from_slice(&raw[name]).unwrap();
            // The raw export runs in the live agent, beside its running
            // SimpleX core: it never carries SimpleX, so it has no record of
            // it. Streamed here without a stage id, nothing is carried.
            let carry = ours.as_object_mut().unwrap().remove("simplex");
            assert_eq!(
                carry,
                Some(serde_json::json!({"carried": false, "why": "no-stage-id"}))
            );
            assert_eq!(theirs, ours, "the summaries");
        } else {
            assert_eq!(&raw[name], bytes, "{name}");
        }
    }
    let raw_bytes = std::fs::read(&out).unwrap();
    for needle in [MARKER, TELEGRAM_TOKEN, OPENROUTER_KEY, SECRET_HEX] {
        assert!(
            !contains(&raw_bytes, needle.as_bytes()),
            "the raw zip holds {needle}"
        );
    }
}

#[test]
fn the_raw_exporters_own_tests_pass() {
    let run = Command::new("python3")
        .args(["-I", "-m", "unittest", "discover", "-s"])
        .arg(raw_exporter())
        .args(["-p", "test_raw_export.py"])
        .output()
        .expect("python3 runs the raw-folder exporter's tests");
    assert!(
        run.status.success(),
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}
