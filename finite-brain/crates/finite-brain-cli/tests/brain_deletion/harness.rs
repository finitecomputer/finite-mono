//! Process harness for the Brain deletion acceptance tests: a restartable
//! file-backed Brain server, imported identities, `fbrain` runners, row
//! dumps and the evidence report.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::mpsc;
use std::thread;

use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub(super) const ADMIN_SECRET: &str =
    "00000000000000000000000000000000000000000000000000000000000000a1";
pub(super) const AGENT_SECRET: &str =
    "00000000000000000000000000000000000000000000000000000000000000a2";
pub(super) const MEMBER_SECRET: &str =
    "00000000000000000000000000000000000000000000000000000000000000a3";
pub(super) const INVITEE_SECRET: &str =
    "00000000000000000000000000000000000000000000000000000000000000a4";
pub(super) const OUTSIDER_SECRET: &str =
    "00000000000000000000000000000000000000000000000000000000000000a5";

/// A real Brain server on a file-backed SQLite store, restartable on the
/// same address so clients keep their configured server.
pub(super) struct Server {
    pub(super) url: String,
    address: SocketAddr,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    pub(super) fn start(database: &Path, address: Option<SocketAddr>) -> Self {
        let database = database.to_path_buf();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let thread = thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let bind = address.unwrap_or_else(|| "127.0.0.1:0".parse().unwrap());
                let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
                let address = listener.local_addr().unwrap();
                let url = format!("http://{address}");
                let store = finite_brain_store::BrainStore::open(&database).unwrap();
                let state = finite_brain_server::ServerState::new(store, url.clone());
                ready_tx.send((url, address)).unwrap();
                axum::serve(listener, finite_brain_server::router_with_state(state))
                    .with_graceful_shutdown(async move {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .unwrap();
            });
        });
        let (url, address) = ready_rx.recv().unwrap();
        Self {
            url,
            address,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
        }
    }

    pub(super) fn stop(mut self) -> SocketAddr {
        self.halt();
        self.address
    }

    fn halt(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.halt();
    }
}

pub(super) struct Home {
    pub(super) dir: PathBuf,
    pub(super) npub: String,
}

pub(super) fn home(scratch: &Path, name: &str, secret: &str) -> Home {
    let dir = scratch.join(name);
    fs::create_dir_all(&dir).unwrap();
    let secret_file = scratch.join(format!("{name}.secret"));
    fs::write(&secret_file, format!("{secret}\n")).unwrap();
    let imported = command(&dir, &dir, "http://127.0.0.1:9")
        .args(["auth", "import", "--file", secret_file.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(imported.status.success(), "{}", stderr(&imported));
    fs::remove_file(secret_file).unwrap();
    let signer = command(&dir, &dir, "http://127.0.0.1:9")
        .args(["signer", "public-key", "--json"])
        .output()
        .unwrap();
    let npub = serde_json::from_slice::<Value>(&signer.stdout).unwrap()["npub"]
        .as_str()
        .unwrap()
        .to_owned();
    Home { dir, npub }
}

pub(super) fn command(home: &Path, cwd: &Path, server: &str) -> Command {
    command_with(Path::new(env!("CARGO_BIN_EXE_fbrain")), home, cwd, server)
}

pub(super) fn command_with(binary: &Path, home: &Path, cwd: &Path, server: &str) -> Command {
    let mut command = Command::new(binary);
    command
        .current_dir(cwd)
        .env_clear()
        .env("HOME", home)
        .env("FINITE_HOME", home.join("finite-home"))
        .env("FBRAIN_CONFIG_DIR", home.join("fbrain-config"))
        .env(
            "FBRAIN_NOW",
            OffsetDateTime::now_utc().format(&Rfc3339).unwrap(),
        )
        .env("FINITE_BRAIN_SERVER_URL", server)
        .env("FINITE_BRAIN_PUBLIC_BASE_URL", server);
    command
}

pub(super) fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Run a command that must succeed; returns its `--json` output.
pub(super) fn ok(home: &Home, cwd: &Path, server: &str, args: &[&str]) -> Value {
    let output = command(&home.dir, cwd, server)
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}: {}", stderr(&output));
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Run a command that must fail; returns its error text.
pub(super) fn refused(home: &Home, cwd: &Path, server: &str, args: &[&str]) -> String {
    let output = command(&home.dir, cwd, server).args(args).output().unwrap();
    assert!(
        !output.status.success(),
        "{args:?} unexpectedly succeeded: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    stderr(&output)
}

/// Every row in every table that names `brain_id`, outside the permanent
/// deletion record, as sorted text. The retired email-delegation tables are
/// always empty for Organization Brains; the store tests cover them.
pub(super) fn brain_rows(database: &Path, brain_id: &str) -> Vec<String> {
    let conn = rusqlite::Connection::open(database).unwrap();
    let tables = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut rows = Vec::new();
    for table in tables {
        if table == "deleted_brains" || table == "deleted_brain_audience" {
            continue;
        }
        let columns = conn
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let keyed = columns
            .iter()
            .filter(|column| {
                matches!(
                    column.as_str(),
                    "brain_id" | "source_brain_id" | "destination_brain_id"
                ) || (table == "brains" && column.as_str() == "id")
            })
            .map(|column| format!("\"{column}\" = ?1"))
            .collect::<Vec<_>>();
        if keyed.is_empty() {
            continue;
        }
        let mut stmt = conn
            .prepare(&format!(
                "SELECT * FROM \"{table}\" WHERE {}",
                keyed.join(" OR ")
            ))
            .unwrap();
        let width = stmt.column_count();
        let mut found = stmt.query([brain_id]).unwrap();
        while let Some(row) = found.next().unwrap() {
            let values = (0..width)
                .map(|index| match row.get_ref(index).unwrap() {
                    rusqlite::types::ValueRef::Text(text) => {
                        String::from_utf8_lossy(text).into_owned()
                    }
                    other => format!("{other:?}"),
                })
                .collect::<Vec<_>>();
            rows.push(format!("{table}|{}", values.join("|")));
        }
    }
    rows.sort();
    rows
}

/// Another Brain keeps every row except the revoked Mount records that named
/// V1 (removed so no stale offer can restore access) and its derived
/// capacity counters. Returns how many retired records were removed.
pub(super) fn assert_only_retired_mount_records_changed(
    before: &[String],
    after: &[String],
) -> usize {
    let retired = |row: &String| {
        (row.starts_with("folder_mounts|") || row.starts_with("shared_folder_"))
            && row.split('|').any(|field| field == "v1")
    };
    let derived = |row: &String| row.starts_with("brain_capacity_counts|");
    let kept = |rows: &[String]| {
        rows.iter()
            .filter(|row| !retired(row) && !derived(row))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(kept(after), kept(before));
    assert!(!after.iter().any(retired), "{after:#?}");
    before.iter().filter(|row| retired(row)).count()
}

pub(super) fn copy_database(from: &Path, to: &Path) {
    fs::create_dir_all(to.parent().unwrap()).unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let source = PathBuf::from(format!("{}{suffix}", from.display()));
        if source.exists() {
            fs::copy(&source, format!("{}{suffix}", to.display())).unwrap();
        }
    }
}

/// Boundaries that passed, written as JSON when the report path is set.
pub(super) struct Evidence {
    pub(super) path: Option<PathBuf>,
    pub(super) facts: serde_json::Map<String, Value>,
}

impl Evidence {
    pub(super) fn pass(&mut self, boundary: &str, detail: Value) {
        self.facts.insert(boundary.to_owned(), detail);
    }
}

impl Drop for Evidence {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let report = json!({
                "format": "finite.brain.organization-brain-deletion-acceptance.v1",
                "status": if thread::panicking() { "failed" } else { "passed" },
                "passedBoundaries": self.facts,
                "syntheticOnly": true,
            });
            fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
    }
}

pub(super) struct ChildGuard(pub(super) std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn wait_until(what: &str, log: &Path, mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}: {}",
            fs::read_to_string(log).unwrap_or_default()
        );
        thread::sleep(std::time::Duration::from_millis(100));
    }
}
