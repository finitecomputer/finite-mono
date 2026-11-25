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

use finite_mover_host::classify::{Class, EntryKind, classify};
use finite_mover_host::command::{Operation, parse, run};
use finite_mover_host::error::HostError;
use finite_mover_host::stage::SUMMARY_ENTRY;
use finite_mover_host::summary::Tally;
use fixture::{
    AGENT_KEY, BRAINS, Fixture, ID, KEY_PAYLOAD, MARKER, OPENROUTER_KEY, PERSON_KEY, REDACTED_TEXT,
    SECRET_HEX, STAGED, TELEGRAM_TOKEN, name_file,
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

/// The keys as the mover names them on `stream`.
fn keys() -> String {
    format!(
        "--agent-key {} --person-key {}",
        hex::encode(AGENT_KEY),
        hex::encode(PERSON_KEY)
    )
}

fn stream(fixture: &Fixture) -> Vec<u8> {
    run_bytes(fixture, &format!("stream {ID} {}", keys())).unwrap()
}

fn sizes(fixture: &Fixture) -> Value {
    let out = run_bytes(fixture, &format!("sizes {ID}")).unwrap();
    serde_json::from_slice(&out).unwrap()
}

/// Every brain's name file, in the stage.
fn name_files() -> BTreeSet<String> {
    BRAINS
        .iter()
        .map(|(folder, _, _)| name_file(folder))
        .collect()
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
        KEY_PAYLOAD,
        "root:",
    ] {
        assert!(
            !contains(&stage, needle.as_bytes()),
            "the stage holds {needle}"
        );
    }
    for name in entries(&stage).keys() {
        let parts: Vec<&[u8]> = name.split('/').map(str::as_bytes).collect();
        let verdict = classify(&parts, EntryKind::File, None);
        assert_ne!(verdict.class, Class::Secret, "{name} is a secret path");
    }
}

#[test]
fn the_stage_is_exactly_history_work_the_config_copy_and_name_files() {
    let fixture = fixture::build();
    let entries = entries(&stream(&fixture));
    let names = name_files();
    let mut expected: BTreeSet<&str> = STAGED.iter().map(|(path, _)| *path).collect();
    expected.extend([CONFIG_COPY, SUMMARY_ENTRY]);
    expected.extend(names.iter().map(String::as_str));
    expected.extend(REDACTED_TEXT.iter().map(|(path, _, _)| *path));
    assert_eq!(
        entries.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        expected
    );
    for (path, _, redacted) in REDACTED_TEXT {
        assert_eq!(entries[*path], redacted.as_bytes(), "{path} is redacted");
    }
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
        } else if name_files().contains(name) {
            "work"
        } else if REDACTED_TEXT.iter().any(|(path, _, _)| path == name) {
            "history"
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
        ("workspace/finitebrain/brain-org/.finitebrain", "cache-dir"),
        ("workspace/finitebrain/broken/.finitebrain", "cache-dir"),
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

    // `sizes` reads no file, so it differs from the stage in exactly four
    // ways: a work text file holding a credential counts in its path's
    // class, a history one counts its bytes before redaction, and there is
    // no config copy and no name file. Everything else is the same.
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
    let named: Vec<u64> = name_files()
        .iter()
        .map(|name| entries[name].len() as u64)
        .collect();
    let redaction: u64 = REDACTED_TEXT
        .iter()
        .map(|(_, original, redacted)| (original.len() - redacted.len()) as u64)
        .sum();
    assert_eq!(
        staged_by_path(&estimate),
        (
            files + text.files - named.len() as u64,
            bytes + text.bytes - named.iter().sum::<u64>() + redaction
        )
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
    let after = run_bytes(&fixture, &format!("sizes {ID}"));
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

/// FIN-499: every brain working tree comes, whoever owns it, even one
/// whose manifest can't be read; its `.finitebrain/` never does.
#[test]
fn every_brain_is_staged_but_its_control_folder() {
    let fixture = fixture::build();
    let stage = run_bytes(&fixture, &format!("stream {ID}")).unwrap();
    let files = entries(&stage);
    for (folder, _, _) in BRAINS {
        let page = format!("workspace/finitebrain/{folder}/page.md");
        assert_eq!(
            files.get(&page).map(Vec::as_slice),
            Some(std::fs::read(fixture.tree.join(&page)).unwrap().as_slice()),
            "{page}"
        );
    }
    assert!(
        files.keys().all(|name| !name.contains(".finitebrain")),
        "no control folder: {:?}",
        files.keys().collect::<Vec<_>>()
    );
    assert!(left_of(&stage).contains(&(
        "workspace/finitebrain/brain-org/.finitebrain".to_owned(),
        "cache-dir".to_owned()
    )));
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

/// A brain's name file, as staged.
fn named(files: &BTreeMap<String, Vec<u8>>, folder: &str) -> Value {
    serde_json::from_slice(&files[&name_file(folder)]).unwrap()
}

/// Each brain's name file holds its manifest's name, kind and owner, and
/// whose it is to the old agent: `agent`, `person`, `other` or `unknown`
/// (FIN-499). An old file where it goes stays behind.
#[test]
fn each_name_file_says_whose_its_brain_is() {
    let fixture = fixture::build();
    let brains = fixture.tree.join("workspace/finitebrain");
    std::fs::write(brains.join("brain-agent.json"), r#"{"name":"stale"}"#).unwrap();
    std::fs::create_dir_all(brains.join("brain-agent/wiki")).unwrap();
    std::fs::write(brains.join("brain-agent/wiki/topic.md"), "# Topic\n").unwrap();
    let stage = stream(&fixture);
    let files = entries(&stage);
    for (folder, manifest, owner) in BRAINS {
        let expected = match manifest {
            Some((kind, key)) => serde_json::json!({
                "name": format!("{folder} notes"),
                "kind": kind,
                "owner_npub": key.map(fixture::npub),
                "owner": owner,
                "versions": null,
            }),
            None => serde_json::json!({
                "name": null, "kind": null, "owner_npub": null, "owner": "unknown", "versions": null,
            }),
        };
        assert_eq!(named(&files, folder), expected, "{folder}");
    }
    assert_eq!(
        files["workspace/finitebrain/brain-agent/wiki/topic.md"],
        b"# Topic\n"
    );
    assert!(left_of(&stage).contains(&(
        "workspace/finitebrain/brain-agent.json".to_owned(),
        "replaced-by-copy".to_owned()
    )));

    // Without its person's keys, a brain neither key owns can't be placed:
    // `unknown`, never a guess. A team's brain is still a team's.
    let agent_only = run_bytes(
        &fixture,
        &format!("stream {ID} --agent-key {}", hex::encode(AGENT_KEY)),
    )
    .unwrap();
    let files = entries(&agent_only);
    for (folder, owner) in [
        ("brain-agent", "agent"),
        ("brain-person", "unknown"),
        ("brain-else", "unknown"),
        ("brain-org", "other"),
        ("broken", "unknown"),
    ] {
        assert_eq!(named(&files, folder)["owner"], owner, "{folder}");
    }
}

/// Write `pages` into the brain in `workspace/finitebrain/<folder>/`, and
/// its `working-tree-state.json` listing each under its top folder, with
/// `Leadership` a folder the old agent could not read (FIN-773).
fn brain_with_versions(fixture: &fixture::Fixture, folder: &str, pages: &[(&str, &str, &str)]) {
    let brain = fixture.tree.join(format!("workspace/finitebrain/{folder}"));
    let mut objects = Vec::new();
    for (root, path, text) in pages {
        let file = brain.join(root).join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, text).unwrap();
        objects.push(serde_json::json!({
            "folderId": root.to_lowercase().replace(' ', "-"), "path": path, "objectId": "obj_1",
            "revision": 3, "keyVersion": 1, "contentType": "text/markdown", "contentHash": sha256(text.as_bytes()),
        }));
    }
    let state = serde_json::json!({
        "version": "finite-brain-working-tree-state-v1",
        "folderRoots": [
            {"folderId": "field-notes", "path": "Field Notes", "canRead": true, "metadataOnly": false},
            {"folderId": "leadership", "path": "Leadership", "canRead": false, "metadataOnly": true},
        ],
        "objects": objects,
        "sync": {"latestSequence": 41},
    });
    std::fs::write(
        brain.join(".finitebrain/working-tree-state.json"),
        state.to_string(),
    )
    .unwrap();
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}

/// FIN-773: a brain's name file carries its working tree's own record of
/// what it holds, by each page's path in the stage, and the stage still
/// holds no control folder. The same tree staged again gives the same
/// name file; a brain without the record has `versions` null.
#[test]
fn a_name_file_carries_its_trees_versions() {
    let fixture = fixture::build();
    brain_with_versions(&fixture, "brain-org", &[("Field Notes", "a.md", "# A\n")]);
    let stage = stream(&fixture);
    let files = entries(&stage);
    let page = "workspace/finitebrain/brain-org/Field Notes/a.md";
    assert_eq!(files[page], b"# A\n");
    assert_eq!(
        named(&files, "brain-org")["versions"],
        serde_json::json!({
            "sequence": 41,
            "folders": [{"path": "Field Notes", "read": true}, {"path": "Leadership", "read": false}],
            "pages": [{"path": "Field Notes/a.md", "object": "obj_1", "revision": 3, "sha256": sha256(&files[page])}],
        })
    );
    assert_eq!(named(&files, "brain-org")["owner"], "other");
    assert!(files.keys().all(|name| !name.contains(".finitebrain")));
    for (folder, _, _) in BRAINS
        .iter()
        .filter(|(folder, _, _)| *folder != "brain-org")
    {
        assert_eq!(named(&files, folder)["versions"], Value::Null, "{folder}");
    }
    let again = entries(&stream(&fixture));
    assert_eq!(
        again[&name_file("brain-org")],
        files[&name_file("brain-org")]
    );
}

/// A page whose path has a credential's shape costs the name file its
/// `versions` alone, never its name or owner; a record that isn't JSON
/// gives `versions` null and the stage still goes.
#[test]
fn a_bad_record_costs_only_the_versions() {
    let fixture = fixture::build();
    brain_with_versions(
        &fixture,
        "brain-org",
        &[(
            "Field Notes",
            "ghp_0123456789abcdefghijABCDEFGHIJ.md",
            "# Token-shaped name\n",
        )],
    );
    std::fs::write(
        fixture
            .tree
            .join("workspace/finitebrain/brain-agent/.finitebrain/working-tree-state.json"),
        "not json",
    )
    .unwrap();
    let files = entries(&stream(&fixture));
    let org = named(&files, "brain-org");
    assert_eq!(
        (org["owner"].as_str(), org.get("versions")),
        (Some("other"), Some(&Value::Null))
    );
    assert!(!contains(&files[&name_file("brain-org")], b"ghp_"));
    let agent = named(&files, "brain-agent");
    assert_eq!(
        (agent["owner"].as_str(), agent.get("versions")),
        (Some("agent"), Some(&Value::Null))
    );
}

/// An older mover names its person's owned brains, with their names in
/// hex, on `sizes` and `stream`. A host updated first serves it the same
/// stage, with every brain, as a mover that names none.
#[test]
fn an_older_movers_brains_are_ignored() {
    let fixture = fixture::build();
    let older = format!(
        "--brain brain-agent:{} --brain brain-org",
        hex::encode("Old notes: été")
    );
    assert_eq!(
        entries(&run_bytes(&fixture, &format!("stream {ID} {older}")).unwrap()),
        entries(&run_bytes(&fixture, &format!("stream {ID}")).unwrap())
    );
    let sized: Value =
        serde_json::from_slice(&run_bytes(&fixture, &format!("sizes {ID} {older}")).unwrap())
            .unwrap();
    assert_eq!(sized, sizes(&fixture));
}

/// The operator's overrides (`--carry`, `--leave`): a runtime folder
/// comes, a work folder stays, the most specific path decides, and a
/// credential never comes; `sizes` estimates the same.
#[test]
fn the_operators_overrides_carry_and_leave_but_never_a_secret() {
    let fixture = fixture::build();
    let words = [
        ("--carry", "agent/hermes-home"),
        ("--carry", "workspace/app/node_modules"),
        ("--leave", "workspace/photos"),
        ("--carry", "workspace/photos/cat.jpg"),
        ("--leave", "workspace/notes"),
        ("--carry", "workspace/.git-credentials"),
    ]
    .map(|(flag, path)| format!("{flag} {}", hex::encode(path)))
    .join(" ");
    let stage = run_bytes(&fixture, &format!("stream {ID} {} {words}", keys())).unwrap();
    let files = entries(&stage);
    for carried in [
        "agent/hermes-home/logs/gateway.log",
        "workspace/app/node_modules/pkg/index.js",
        "workspace/photos/cat.jpg",
    ] {
        assert!(files.contains_key(carried), "{carried} is carried");
    }
    for left in [
        "workspace/notes/today.md",
        "agent/hermes-home/.env",
        "agent/hermes-home/config.yaml",
        "agent/hermes-home/SOUL-link.md",
        "workspace/.git-credentials",
    ] {
        assert!(!files.contains_key(left), "{left} is left");
    }
    assert!(
        left_of(&stage).contains(&("workspace/notes".to_owned(), "left-by-operator".to_owned()))
    );
    for needle in [MARKER, TELEGRAM_TOKEN, OPENROUTER_KEY, SECRET_HEX] {
        assert!(
            !contains(&stage, needle.as_bytes()),
            "the stage holds {needle}"
        );
    }
    let measured: Value =
        serde_json::from_slice(&run_bytes(&fixture, &format!("sizes {ID} {words}")).unwrap())
            .unwrap();
    let streamed: Value = serde_json::from_slice(&files[SUMMARY_ENTRY]).unwrap();
    for class in ["cache", "runtime"] {
        assert_eq!(
            measured["classes"][class], streamed["classes"][class],
            "{class}"
        );
    }
}

/// A carried cache or build folder keeps the link and name rules: its
/// regular files come, and a symbolic link, a file with a second name and
/// a name a zip can't carry stay behind, listed; the stream completes.
#[test]
fn a_carried_cache_or_build_folder_still_leaves_links_and_odd_names() {
    use std::os::unix::ffi::OsStrExt;
    let fixture = fixture::build();
    let folders = [
        ("workspace/app/node_modules", "pkg/lib.js"),
        ("workspace/app/dist", "chunk.js"),
        ("workspace/py/.venv", "lib/site.py"),
    ];
    for (index, (folder, regular)) in folders.iter().enumerate() {
        let root = fixture.tree.join(folder);
        let file = root.join(regular);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, format!("{folder} code\n")).unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::os::unix::fs::symlink(&file, root.join("bin/tool")).unwrap();
        std::os::unix::fs::symlink("/usr/bin/python3", root.join("bin/python3")).unwrap();
        let outside = fixture.work_root.join(format!("outside-{index}.txt"));
        std::fs::write(&outside, format!("{MARKER}-hard-{index}\n")).unwrap();
        std::fs::hard_link(&outside, root.join("hard.js")).unwrap();
        for name in [&b"bad\xffname.js"[..], b"new\nline.js"] {
            std::fs::write(root.join(std::ffi::OsStr::from_bytes(name)), b"odd\n").unwrap();
        }
    }
    let carry = folders
        .iter()
        .map(|(folder, _)| format!("--carry {}", hex::encode(folder)))
        .collect::<Vec<_>>()
        .join(" ");
    let stage = run_bytes(&fixture, &format!("stream {ID} {} {carry}", keys()))
        .expect("the stream completes");
    let files = entries(&stage);
    let left = left_of(&stage);
    for (folder, regular) in folders {
        let path = format!("{folder}/{regular}");
        assert_eq!(
            files[&path],
            format!("{folder} code\n").as_bytes(),
            "{path}"
        );
        for (name, why) in [
            ("bin/tool", "not-a-file"),
            ("bin/python3", "not-a-file"),
            ("hard.js", "not-a-file"),
            ("bad\u{fffd}name.js", "uncarried-name"),
            ("new\nline.js", "uncarried-name"),
        ] {
            let path = format!("{folder}/{name}");
            assert!(!files.contains_key(&path), "{path:?} is staged");
            assert!(
                left.contains(&(path.clone(), why.to_owned())),
                "{path:?}: {left:?}"
            );
        }
    }
    assert!(!contains(&stage, format!("{MARKER}-hard").as_bytes()));
    assert!(files.keys().all(|name| !name.contains('\n')));
    // `sizes` walks the same rules and completes too.
    let measured: Value =
        serde_json::from_slice(&run_bytes(&fixture, &format!("sizes {ID} {carry}")).unwrap())
            .unwrap();
    let streamed: Value = serde_json::from_slice(&files[SUMMARY_ENTRY]).unwrap();
    assert_eq!(
        measured["classes"]["runtime"],
        streamed["classes"]["runtime"]
    );
}

/// FIN-499: a history text file holding a credential comes with it
/// redacted and every other byte as it was, as phase 0's export did; work
/// text holding one stays behind, a binary file is not read for one, and a
/// left path shaped like one is redacted in the summary.
#[test]
fn history_text_holding_a_credential_is_staged_redacted() {
    let fixture = fixture::build();
    let write = |path: &str, bytes: &[u8]| {
        let path = fixture.tree.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    let soul =
        "I am kind and brief.\nDeploy with ghp_abcdefghij0123456789 if asked.\nNever share it.\n";
    let jobs = r#"{"jobs":[{"id":"daily","prompt":"post to https://bot:hunter22secret@example.org/hook with key sk-proj4abcdefghijklmnop","schedule":{"kind":"cron","expr":"0 9 * * *"}}]}"#;
    write("agent/hermes-home/SOUL.md", soul.as_bytes());
    write("agent/hermes-home/cron/jobs.json", jobs.as_bytes());
    let binary = [&b"\0\x01blob"[..], b" ghp_abcdefghij0123456789"].concat();
    write("agent/hermes-home/memories/blob.bin", &binary);
    write(
        "workspace/ghp_abcdefghij0123456789/key.txt",
        b"ghp_zyxwvutsrq9876543210\n",
    );
    let stage = stream(&fixture);
    let files = entries(&stage);
    assert_eq!(
        String::from_utf8(files["agent/hermes-home/SOUL.md"].clone()).unwrap(),
        "I am kind and brief.\nDeploy with [REDACTED] if asked.\nNever share it.\n"
    );
    assert_eq!(
        String::from_utf8(files["agent/hermes-home/cron/jobs.json"].clone()).unwrap(),
        r#"{"jobs":[{"id":"daily","prompt":"post to [REDACTED]example.org/hook with key [REDACTED]","schedule":{"kind":"cron","expr":"0 9 * * *"}}]}"#
    );
    assert_eq!(
        files["agent/hermes-home/memories/blob.bin"], binary,
        "binary: as it is"
    );
    assert!(!files.contains_key("workspace/ghp_abcdefghij0123456789/key.txt"));
    let left = left_of(&stage);
    assert!(
        left.contains(&(
            "workspace/[REDACTED]/key.txt".to_owned(),
            "token-in-text".to_owned()
        )),
        "{left:?}"
    );
    assert!(!contains(&files[SUMMARY_ENTRY], b"ghp_"));
}

/// A brain folder whose name file would be named like a credential gets
/// no name file, and the stage still goes (the uploader would refuse it).
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
    let stage = stream(&fixture);
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
    let sized = ssh(&fixture.work_root, &format!("sizes {ID}"));
    assert!(sized.status.success());
    let summary: Value = serde_json::from_slice(&sized.stdout).unwrap();
    assert_eq!(summary["id"], ID);
    let streamed = ssh(&fixture.work_root, &format!("stream {ID} {}", keys()));
    assert!(streamed.status.success());
    assert!(entries(&streamed.stdout).contains_key(SUMMARY_ENTRY));
    // An older mover's command, as it comes over ssh.
    let older = ssh(
        &fixture.work_root,
        &format!(
            "stream {ID} --stage proj_a.p1.1 --brain brain-agent:{}",
            hex::encode("Old notes")
        ),
    );
    assert!(older.status.success());
    assert!(entries(&older.stdout).contains_key(&name_file("brain-org")));
    for (original, code) in [
        ("cat /etc/passwd", 64),
        (&format!("stream ../{ID}") as &str, 65),
        ("sizes x;id", 65),
        (&format!("stream {ID} --agent-key npub1x") as &str, 64),
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
        // An older caller's owned brain: ignored, as the host ignores it.
        .args(["--brain", "brain-agent", "--id", ID])
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
    // Brains' name files are the host's alone: the raw export's caller
    // writes its own (`--fact`), as phase 0's export did.
    let names = name_files();
    let named = names.iter().map(|name| staged[name].len() as u64);
    let (named_files, named_bytes) = (names.len() as u64, named.sum::<u64>());
    let staged: BTreeMap<String, Vec<u8>> = staged
        .into_iter()
        .filter(|(name, _)| !names.contains(name))
        .collect();
    assert_eq!(
        raw.keys().collect::<Vec<_>>(),
        staged.keys().collect::<Vec<_>>()
    );
    for (name, bytes) in &staged {
        if name == SUMMARY_ENTRY {
            let mut ours: Value = serde_json::from_slice(bytes).unwrap();
            for tally in ["/classes/work", "/staged"] {
                let tally = ours.pointer_mut(tally).unwrap();
                tally["files"] = (tally["files"].as_u64().unwrap() - named_files).into();
                tally["bytes"] = (tally["bytes"].as_u64().unwrap() - named_bytes).into();
            }
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
    for needle in [
        MARKER,
        TELEGRAM_TOKEN,
        OPENROUTER_KEY,
        SECRET_HEX,
        KEY_PAYLOAD,
    ] {
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
