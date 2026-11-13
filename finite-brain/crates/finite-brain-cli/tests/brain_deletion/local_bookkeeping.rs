//! A successful server deletion remains successful when local bookkeeping fails.

use super::*;

#[test]
fn confirmed_deletion_keeps_receipt_and_terminal_state_when_local_scan_fails() {
    for json_output in [true, false] {
        let scratch = TempDir::new().unwrap();
        let admin = home(scratch.path(), "admin", ADMIN_SECRET);
        let database = scratch.path().join("brain.sqlite3");
        let server = Server::start(&database, None);
        let tree = create_tree(&admin, &server);
        let bad_file = tree.join("Knowledge/wiki/invalid.md");
        let bytes = b"# Unsynced local work\n\xff\xfe";
        fs::write(&bad_file, bytes).unwrap();
        let preview = ok(&admin, &tree, &server.url, &["brain", "delete", "v1"]);
        let digest = preview["preview"]["stateDigest"].as_str().unwrap();
        let mut delete = command(&admin.dir, &tree, &server.url);
        delete.args(["brain", "delete", "v1", "--confirm", digest]);
        if json_output {
            delete.arg("--json");
        }
        let output = delete.output().unwrap();

        // The server has already committed, regardless of local bookkeeping.
        assert!(brain_rows(&database, "v1").is_empty());
        assert_eq!(fs::read(&bad_file).unwrap(), bytes);
        assert!(output.status.success(), "{}", stderr(&output));
        let state: Value =
            serde_json::from_slice(&fs::read(tree.join(".finitebrain/agent-state.json")).unwrap())
                .unwrap();
        assert!(state["brainDeleted"]["deletedAt"].is_string(), "{state}");
        assert!(
            state["brainDeleted"]["unsyncedPathsError"].is_string(),
            "{state}"
        );
        if json_output {
            let deleted: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(deleted["state"], "deleted");
            assert_eq!(deleted["receipt"]["brainId"], "v1");
            assert_eq!(deleted["receipt"]["stateDigest"], digest);
            assert_eq!(
                deleted["receipt"]["deletedAt"],
                state["brainDeleted"]["deletedAt"]
            );
            assert!(deleted["receipt"]["deletionEventId"].is_string());
            assert!(
                deleted["warnings"][0]
                    .as_str()
                    .unwrap()
                    .contains("could not list unsynced")
            );
        } else {
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(text.contains("deleted Organization Brain"), "{text}");
            assert!(text.contains("receipt deletionEvent="), "{text}");
            assert!(text.contains("Warning:"), "{text}");
            assert!(text.contains("could not list unsynced"), "{text}");
        }

        drop(server);
        let sync = refused(&admin, &tree, "http://127.0.0.1:9", &["sync", "now"]);
        assert!(
            sync.contains("was permanently deleted on the server"),
            "{sync}"
        );
        assert!(sync.contains("could not list unsynced"), "{sync}");
        assert_eq!(fs::read(&bad_file).unwrap(), bytes);
    }
}

#[test]
fn confirmed_deletion_keeps_receipt_when_local_state_cannot_be_updated() {
    let scratch = TempDir::new().unwrap();
    let admin = home(scratch.path(), "admin", ADMIN_SECRET);
    let database = scratch.path().join("brain.sqlite3");
    let server = Server::start(&database, None);
    let tree = create_tree(&admin, &server);
    let local_file = tree.join("Knowledge/wiki/local.md");
    fs::write(&local_file, "# Keep my local work\n").unwrap();
    let preview = ok(&admin, &tree, &server.url, &["brain", "delete", "v1"]);
    let state_path = tree.join(".finitebrain/agent-state.json");
    fs::write(&state_path, "{broken local state").unwrap();
    let output = command(&admin.dir, &tree, &server.url)
        .args([
            "brain",
            "delete",
            "v1",
            "--confirm",
            preview["preview"]["stateDigest"].as_str().unwrap(),
            "--server",
            &server.url,
            "--json",
        ])
        .output()
        .unwrap();
    assert!(brain_rows(&database, "v1").is_empty());
    assert!(output.status.success(), "{}", stderr(&output));
    let deleted: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(deleted["state"], "deleted");
    assert_eq!(deleted["receipt"]["brainId"], "v1");
    assert!(deleted["receipt"]["deletionEventId"].is_string());
    assert!(
        deleted["warnings"][0]
            .as_str()
            .unwrap()
            .contains("could not record")
    );
    assert_eq!(
        fs::read_to_string(state_path).unwrap(),
        "{broken local state"
    );
    assert_eq!(
        fs::read_to_string(local_file).unwrap(),
        "# Keep my local work\n"
    );
}

fn create_tree(admin: &Home, server: &Server) -> PathBuf {
    ok(
        admin,
        &admin.dir,
        &server.url,
        &[
            "brain",
            "create",
            "v1",
            "--kind",
            "organization",
            "--name",
            "V1",
        ],
    );
    let tree = admin.dir.join("v1");
    ok(
        admin,
        &admin.dir,
        &server.url,
        &["open", "v1", tree.to_str().unwrap()],
    );
    ok(
        admin,
        &tree,
        &server.url,
        &["folder", "create", "Knowledge"],
    );
    ok(admin, &tree, &server.url, &["sync", "now"]);
    tree
}
