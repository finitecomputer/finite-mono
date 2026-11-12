//! The owned brains reader (FIN-499) against finite-brain's own schema
//! (`finite-brain-store`), its database open in WAL mode as finite-brain
//! keeps it: `finite-mover-brains` lists exactly the brains with an owner,
//! sees what is still only in the WAL, and changes nothing; the mover
//! keeps each agent's own.

use std::path::Path;
use std::process::Command;

use finite_brain_store::BrainStore;
use finite_mover::brains::{BrainsReader, OwnedBrain};
use nostr::ToBech32;

fn npub(byte: u8) -> String {
    let keys = nostr::Keys::new(nostr::SecretKey::from_slice(&[byte; 32]).unwrap());
    keys.public_key().to_bech32().unwrap()
}

/// finite-brain's database, as its server keeps it open, with one
/// Personal Brain owned by each of two agents' keys and an Organization
/// Brain, which has no owner.
fn brain_database(path: &Path) -> (BrainStore, rusqlite::Connection) {
    let store = BrainStore::open(path).unwrap();
    let writer = rusqlite::Connection::open(path).unwrap();
    writer
        .execute_batch(&format!(
            "INSERT INTO brains (id, kind, name, owner_user_id, created_at) VALUES
               ('brain-agent-a', 'personal', 'Agent A''s notes', '{}', '2026-11-01T00:00:00Z'),
               ('brain-agent-b', 'personal', 'B', '{}', '2026-11-01T00:00:00Z'),
               ('brain-team', 'organization', 'Team', NULL, '2026-11-01T00:00:00Z');",
            npub(1),
            npub(2)
        ))
        .unwrap();
    (store, writer)
}

fn files_of(dir: &Path) -> Vec<(String, u64, std::time::SystemTime)> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let metadata = entry.metadata().unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                metadata.len(),
                metadata.modified().unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}

#[test]
fn the_reader_lists_each_owned_brain_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("finite-brain.sqlite3");
    let (_store, _writer) = brain_database(&path);
    assert!(
        dir.path().join("finite-brain.sqlite3-wal").exists(),
        "the rows are in the WAL, as on a live server"
    );
    let before = files_of(dir.path());

    let run = Command::new(env!("CARGO_BIN_EXE_finite-mover-brains"))
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let listed: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(
        listed,
        serde_json::json!({ "brains": [
            { "id": "brain-agent-a", "name": "Agent A's notes", "owner": npub(1) },
            { "id": "brain-agent-b", "name": "B", "owner": npub(2) },
        ]}),
        "no Organization Brain"
    );
    assert_eq!(files_of(dir.path()), before, "the reader wrote nothing");

    // The mover, as the module runs it: each agent's key gets its own.
    let reader = BrainsReader::new(&[
        env!("CARGO_BIN_EXE_finite-mover-brains").to_string(),
        path.display().to_string(),
    ]);
    let owners = reader.read().unwrap();
    assert_eq!(
        owners.of(Some(&npub(1))),
        [OwnedBrain {
            id: "brain-agent-a".into(),
            name: "Agent A's notes".into()
        }]
    );
    assert!(owners.of(Some(&npub(3))).is_empty());
}

#[test]
fn a_reader_that_fails_is_the_fleets_error_never_no_brains() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.sqlite3");
    let reader = BrainsReader::new(&[
        env!("CARGO_BIN_EXE_finite-mover-brains").to_string(),
        missing.display().to_string(),
    ]);
    let error = reader.read().unwrap_err().to_string();
    assert!(error.contains("finite-brain"), "{error}");
    assert!(!missing.exists(), "a missing database is never made");
    assert!(BrainsReader::new(&["false".to_string()]).read().is_err());
    assert!(
        BrainsReader::new(&["echo".to_string(), "not json".to_string()])
            .read()
            .is_err()
    );
}
