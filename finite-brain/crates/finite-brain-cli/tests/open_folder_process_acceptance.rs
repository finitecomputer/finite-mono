//! Built `fbrain` processes against a real Brain server: an Organization
//! admin opens a Restricted Folder to every Member. Existing and later
//! Members decrypt its existing pages, its explicit Guest keeps access, and
//! its restricted child stays closed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::mpsc;
use std::thread;

use finite_brain_core::{
    BrainId, EncryptedFolderObjectEnvelope, FolderId, FolderKey, FolderObjectAad, ObjectId,
    decode_sync_payload, open_folder_object,
};
use finite_nostr::{GiftWrapValidation, NostrPublicKey, open_gift_wrap};
use nostr::Keys;
use serde_json::{Value, json};
use tempfile::TempDir;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const BRAIN: &str = "roundtrip-org";

struct Harness {
    server_url: String,
    database: PathBuf,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Harness {
    /// A file-backed server holding one Organization Brain owned by `admin`.
    fn start(scratch: &Path, admin: &str) -> Self {
        let database = scratch.join("brain.sqlite3");
        let (url_tx, url_rx) = mpsc::channel();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let admin = admin.to_owned();
        let path = database.clone();
        let thread = thread::spawn(move || {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async move {
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let url = format!("http://{}", listener.local_addr().unwrap());
                    let mut store = finite_brain_store::BrainStore::open(&path).unwrap();
                    let organization = finite_brain_core::bootstrap_organization_brain(
                        BRAIN,
                        "Roundtrip Org",
                        &admin,
                    )
                    .unwrap();
                    store.create_brain_bootstrap(&organization, &[]).unwrap();
                    let state = finite_brain_server::ServerState::new(store, url.clone());
                    url_tx.send(url).unwrap();
                    axum::serve(listener, finite_brain_server::router_with_state(state))
                        .with_graceful_shutdown(async move {
                            let _ = shutdown_rx.await;
                        })
                        .await
                        .unwrap();
                });
        });
        Self {
            server_url: url_rx.recv().unwrap(),
            database,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
        }
    }

    fn fbrain(&self, home: &Path, cwd: &Path, args: &[&str]) -> Output {
        Command::new(PathBuf::from(env!("CARGO_BIN_EXE_fbrain")))
            .current_dir(cwd)
            .env_clear()
            .env("HOME", home)
            .env("FINITE_HOME", home.join("finite-home"))
            .env("FBRAIN_CONFIG_DIR", home.join("fbrain-config"))
            .env(
                "FBRAIN_NOW",
                OffsetDateTime::now_utc().format(&Rfc3339).unwrap(),
            )
            .env("FINITE_BRAIN_SERVER_URL", &self.server_url)
            .env("FINITE_BRAIN_PUBLIC_BASE_URL", &self.server_url)
            .args(args)
            .arg("--json")
            .output()
            .unwrap()
    }

    fn json(&self, home: &Path, cwd: &Path, args: &[&str]) -> Value {
        let output = self.fbrain(home, cwd, args);
        assert!(
            output.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn identity(&self, scratch: &Path, name: &str) -> (PathBuf, Keys, String) {
        let home = scratch.join(name);
        fs::create_dir_all(&home).unwrap();
        let keys = Keys::generate();
        let secret = home.join("import-key");
        fs::write(&secret, keys.secret_key().to_secret_hex()).unwrap();
        self.json(
            &home,
            &home,
            &["auth", "import", "--file", secret.to_str().unwrap()],
        );
        fs::remove_file(secret).unwrap();
        let npub = NostrPublicKey::from_protocol(keys.public_key())
            .to_npub()
            .unwrap();
        (home, keys, npub)
    }

    fn open_tree(&self, home: &Path) -> PathBuf {
        let tree = home.join("tree");
        self.json(home, home, &["open", BRAIN, tree.to_str().unwrap()]);
        self.json(home, &tree, &["sync", "now"]);
        tree
    }

    fn query<T: rusqlite::types::FromSql>(&self, sql: &str, params: &[&str]) -> Vec<T> {
        let connection = rusqlite::Connection::open(&self.database).unwrap();
        let mut statement = connection.prepare(sql).unwrap();
        statement
            .query_map(rusqlite::params_from_iter(params), |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn grant_recipients(&self, folder_id: &str) -> Vec<String> {
        self.query(
            "SELECT recipient_npub FROM folder_key_grants
             WHERE brain_id = 'roundtrip-org' AND folder_id = ?1 AND key_version = 1
             ORDER BY recipient_npub",
            &[folder_id],
        )
    }

    fn record_count(&self) -> i64 {
        self.query::<i64>(
            "SELECT COUNT(*) FROM brain_record_index WHERE brain_id = 'roundtrip-org'",
            &[],
        )[0]
    }

    /// Open `recipient`'s stored wrap of the version-1 Folder Key, the way that
    /// recipient's own client does, and decrypt every live page's Markdown.
    fn decrypt_folder_as(&self, recipient: &Keys, folder_id: &str) -> Vec<String> {
        let recipient_key = NostrPublicKey::from_protocol(recipient.public_key());
        let wrapped = self.query::<String>(
            "SELECT wrapped_event_json FROM folder_key_grants
             WHERE brain_id = 'roundtrip-org' AND folder_id = ?1 AND key_version = 1
               AND recipient_npub = ?2",
            &[folder_id, &recipient_key.to_npub().unwrap()],
        );
        assert_eq!(wrapped.len(), 1, "one current wrap for {folder_id}");
        let event = nostr::Event::from_json(&wrapped[0]).unwrap();
        let opened =
            open_gift_wrap(recipient, &event, &GiftWrapValidation::new(recipient_key)).unwrap();
        let grant: Value = serde_json::from_str(&opened.rumor.content).unwrap();
        let key = FolderKey::from_base64(grant["folderKey"].as_str().unwrap()).unwrap();
        let objects = self.query::<String>(
            "SELECT object_id || char(10) || payload_json FROM current_encrypted_brain_objects
             WHERE brain_id = 'roundtrip-org' AND folder_id = ?1 AND deleted = 0",
            &[folder_id],
        );
        assert!(!objects.is_empty(), "{folder_id} has live pages");
        objects
            .iter()
            .map(|row| {
                let (object_id, payload) = row.split_once('\n').unwrap();
                let envelope = EncryptedFolderObjectEnvelope::from_json(
                    &decode_sync_payload(payload).ciphertext_or_raw(payload),
                )
                .unwrap();
                let aad = FolderObjectAad {
                    brain_id: BrainId::new(BRAIN).unwrap(),
                    folder_id: FolderId::new(folder_id).unwrap(),
                    object_id: ObjectId::new(object_id).unwrap(),
                    key_version: 1,
                };
                let page: Value =
                    serde_json::from_slice(&open_folder_object(&key, &aad, &envelope).unwrap())
                        .unwrap();
                page["markdown"].as_str().unwrap().to_owned()
            })
            .collect()
    }
}

fn folder<'a>(metadata: &'a Value, folder_id: &str) -> &'a Value {
    metadata["folders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|folder| folder["id"] == folder_id)
        .unwrap_or_else(|| panic!("no Folder {folder_id} in {metadata}"))
}

#[test]
fn built_fbrain_admin_opens_a_restricted_folder_to_current_and_future_members() {
    let scratch = TempDir::new().unwrap();
    let probe = Keys::generate();
    let admin_npub = NostrPublicKey::from_protocol(probe.public_key())
        .to_npub()
        .unwrap();
    let harness = Harness::start(scratch.path(), &admin_npub);
    let admin_home = scratch.path().join("admin");
    fs::create_dir_all(&admin_home).unwrap();
    let secret = admin_home.join("import-key");
    fs::write(&secret, probe.secret_key().to_secret_hex()).unwrap();
    harness.json(
        &admin_home,
        &admin_home,
        &["auth", "import", "--file", secret.to_str().unwrap()],
    );
    fs::remove_file(secret).unwrap();
    let (member_home, member_keys, member) = harness.identity(scratch.path(), "member");
    let (guest_home, _, guest) = harness.identity(scratch.path(), "guest");
    let (later_home, later_keys, later) = harness.identity(scratch.path(), "later");

    // A Member with an open, already-synced Working Tree.
    let admin_tree = harness.open_tree(&admin_home);
    harness.json(
        &admin_home,
        &admin_tree,
        &[
            "admin", "member", "add", "--target", &member, "--brain", BRAIN,
        ],
    );
    let member_tree = harness.open_tree(&member_home);

    // A Restricted Folder with a Guest and a Restricted child.
    for args in [
        vec![
            "folder",
            "create",
            "research",
            "--access",
            "restricted",
            "--name",
            "Research",
            "--path",
            "Research",
        ],
        vec![
            "folder",
            "create",
            "inner",
            "--access",
            "restricted",
            "--name",
            "Inner",
            "--parent",
            "research",
            "--path",
            "Research/Inner",
        ],
    ] {
        harness.json(&admin_home, &admin_tree, &args);
    }
    harness.json(
        &admin_home,
        &admin_tree,
        &[
            "admin",
            "folder-access",
            "grant",
            "--folder",
            "research",
            "--target",
            &guest,
            "--brain",
            BRAIN,
        ],
    );
    harness.json(&admin_home, &admin_tree, &["sync", "now"]);
    fs::write(admin_tree.join("Research/plan.md"), "# Plan\n").unwrap();
    fs::write(admin_tree.join("Research/Inner/secret.md"), "# Secret\n").unwrap();
    harness.json(&admin_home, &admin_tree, &["sync", "now"]);

    harness.json(&member_home, &member_tree, &["sync", "now"]);
    assert!(!member_tree.join("Research/plan.md").exists());
    let guest_tree = harness.open_tree(&guest_home);
    assert_eq!(
        fs::read_to_string(guest_tree.join("Research/plan.md")).unwrap(),
        "# Plan\n"
    );
    let mut before = vec![admin_npub.clone(), guest.clone()];
    before.sort();
    assert_eq!(harness.grant_recipients("research"), before);
    let records_before = harness.record_count();

    let opened = harness.json(
        &admin_home,
        &admin_tree,
        &[
            "admin",
            "folder-access",
            "open",
            "--folder",
            "research",
            "--brain",
            BRAIN,
        ],
    );
    assert_eq!(opened["outcome"], "opened", "{opened}");
    assert_eq!(opened["grantedRecipients"], json!([member]));
    // One new grant and one signed audit record, nothing for current holders.
    assert_eq!(harness.record_count(), records_before + 2);
    // The grant and the signed audit record are the two newest records.
    let audit = harness.query::<String>(
        "SELECT payload_json FROM brain_record_index
         WHERE brain_id = 'roundtrip-org' ORDER BY sequence DESC LIMIT 1",
        &[],
    );
    let audit: Value = serde_json::from_str(&audit[0]).unwrap();
    assert_eq!(audit["action"], "set-folder-access-mode");
    assert_eq!(audit["folderId"], "research");
    assert_eq!(audit["keyVersion"], 1);
    assert_eq!(audit["note"], "all_members");

    // The existing Member's routine sync materializes the opened Folder and
    // its own wrap decrypts the stored pages. The child stays closed.
    harness.json(&member_home, &member_tree, &["sync", "now"]);
    assert_eq!(
        fs::read_to_string(member_tree.join("Research/plan.md")).unwrap(),
        "# Plan\n"
    );
    assert!(!member_tree.join("Research/Inner/secret.md").exists());
    assert_eq!(
        harness.decrypt_folder_as(&member_keys, "research"),
        vec!["# Plan\n"]
    );
    let mut child_readers = vec![admin_npub.clone()];
    child_readers.sort();
    assert_eq!(harness.grant_recipients("inner"), child_readers);

    // The Guest keeps explicit access; the child keeps its own mode.
    let metadata = harness.json(
        &admin_home,
        &admin_tree,
        &["brain", "metadata", "--brain", BRAIN],
    );
    assert_eq!(folder(&metadata, "research")["access"], "all_members");
    assert_eq!(
        folder(&metadata, "research")["accessUserIds"],
        json!([guest])
    );
    assert_eq!(folder(&metadata, "research")["currentKeyVersion"], 1);
    assert_eq!(folder(&metadata, "inner")["access"], "restricted");
    harness.json(&guest_home, &guest_tree, &["sync", "now"]);
    assert_eq!(
        fs::read_to_string(guest_tree.join("Research/plan.md")).unwrap(),
        "# Plan\n"
    );
    assert!(!guest_tree.join("Research/Inner/secret.md").exists());

    // Repeating the command changes nothing.
    let again = harness.json(
        &admin_home,
        &admin_tree,
        &[
            "admin",
            "folder-access",
            "open",
            "--folder",
            "research",
            "--brain",
            BRAIN,
        ],
    );
    assert_eq!(again["outcome"], "alreadyOpen", "{again}");
    assert_eq!(harness.record_count(), records_before + 2);

    // A later Member is entitled on joining and receives the key through the
    // normal pending-wrap delivery when a key holder syncs.
    harness.json(
        &admin_home,
        &admin_tree,
        &[
            "admin", "member", "add", "--target", &later, "--brain", BRAIN,
        ],
    );
    let later_tree = harness.open_tree(&later_home);
    harness.json(&admin_home, &admin_tree, &["sync", "now"]);
    harness.json(&later_home, &later_tree, &["sync", "now"]);
    assert_eq!(
        fs::read_to_string(later_tree.join("Research/plan.md")).unwrap(),
        "# Plan\n"
    );
    assert!(!later_tree.join("Research/Inner/secret.md").exists());
    assert_eq!(
        harness.decrypt_folder_as(&later_keys, "research"),
        vec!["# Plan\n"]
    );
}
