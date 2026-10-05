use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

// Synthetic fixture credential, never loaded from the developer's
// environment or used against an external service.
pub(super) const CORE_TOKEN: &str = "synthetic-core-description-credential";
const DESCRIPTIONS_PATH: &str = "/api/core/internal/v1/brain-identity-descriptions";
const DESCRIPTIONS_VERSION: &str = "finite-core-brain-identity-descriptions-v1";

/// Exact-key descriptions the Core stub returns as `resolved`; everything
/// else is `notShared`. `calls` records each request's requested keys.
#[derive(Clone, Default)]
pub(super) struct CoreFixture {
    pub(super) known: Arc<Mutex<BTreeMap<String, Value>>>,
    pub(super) calls: Arc<Mutex<Vec<Vec<String>>>>,
}

impl CoreFixture {
    pub(super) fn human(&self, hex: &str, email: &str) {
        self.known.lock().unwrap().insert(
            hex.to_owned(),
            json!({
                "publicKeyHex": hex, "state": "resolved", "kind": "human", "accountEmail": email,
                "source": {"kind": "hostedDeviceObservation", "observedAt": "2026-05-01T00:00:00Z", "revision": "fixture"},
            }),
        );
    }

    pub(super) fn requested(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .cloned()
            .collect()
    }
}

pub(super) struct Server {
    pub(super) url: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
    child: Option<Child>,
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
fn serve(build: impl FnOnce(&str) -> axum::Router + Send + 'static) -> Server {
    let (address_tx, address_rx) = mpsc::channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let thread = thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let url = format!("http://{}", listener.local_addr().unwrap());
                let router = build(&url);
                address_tx.send(url).unwrap();
                tokio::select! {
                    result = axum::serve(listener, router) => result.unwrap(),
                    _ = shutdown_rx => {}
                }
            });
    });
    Server {
        url: address_rx.recv_timeout(Duration::from_secs(30)).unwrap(),
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
        child: None,
    }
}
pub(super) fn brain(database: &Path, core: Option<&str>) -> Server {
    let database = database.to_owned();
    let core = core.map(str::to_owned);
    serve(move |url| {
        let mut state = finite_brain_server::server_state_with_sqlite_path(database, url)
            .unwrap()
            .with_rate_limit(10_000, 60);
        if let Some(core) = core {
            state = state
                .with_core_identity_descriptions(&core, CORE_TOKEN)
                .unwrap();
        }
        finite_brain_server::router_with_state(state)
    })
}
/// A loopback stub speaking Core's private description protocol. It checks
/// the dedicated credential and answers exactly the requested keys.
pub(super) fn core_stub(fixture: CoreFixture) -> Server {
    serve(move |_| {
        axum::Router::new().route(
            DESCRIPTIONS_PATH,
            axum::routing::post(
                move |headers: axum::http::HeaderMap, axum::Json(request): axum::Json<Value>| {
                    let fixture = fixture.clone();
                    async move {
                        if headers
                            .get("x-finite-brain-description-credential")
                            .and_then(|value| value.to_str().ok())
                            != Some(CORE_TOKEN)
                        {
                            return Err(axum::http::StatusCode::UNAUTHORIZED);
                        }
                        let keys = request["publicKeysHex"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|key| key.as_str().unwrap().to_owned())
                            .collect::<Vec<_>>();
                        fixture.calls.lock().unwrap().push(keys.clone());
                        let known = fixture.known.lock().unwrap().clone();
                        let results = keys
                            .iter()
                            .map(|key| {
                                known.get(key).cloned().unwrap_or_else(
                                    || json!({"publicKeyHex": key, "state": "notShared"}),
                                )
                            })
                            .collect::<Vec<_>>();
                        Ok(axum::Json(json!({
                            "version": DESCRIPTIONS_VERSION,
                            "brainId": request["brainId"],
                            "checkedAt": "2026-05-02T00:00:00Z",
                            "results": results,
                        })))
                    }
                },
            ),
        )
    })
}
pub(super) fn candidate_binary() -> PathBuf {
    let built = PathBuf::from(env!("CARGO_BIN_EXE_fbrain"));
    if let Some(copy) = std::env::var_os("FBRAIN_TEST_CANDIDATE_BINARY").map(PathBuf::from) {
        assert_eq!(
            Sha256::digest(fs::read(&built).unwrap()),
            Sha256::digest(fs::read(&copy).unwrap()),
            "candidate override must match Cargo-built CLI"
        );
        copy
    } else {
        built
    }
}
pub(super) fn execute(
    binary: &Path,
    home: &Path,
    cwd: &Path,
    url: &str,
    args: &[&str],
    json: bool,
) -> Output {
    // Files avoid pipe-capacity deadlocks while enforcing a finite child deadline.
    let stdout = tempfile::NamedTempFile::new().unwrap();
    let stderr = tempfile::NamedTempFile::new().unwrap();
    let mut command = Command::new(binary);
    command
        .current_dir(cwd)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("FINITE_HOME", home)
        .env("FBRAIN_CONFIG_DIR", home.join("fbrain"))
        .env("FINITE_BRAIN_SERVER_URL", url)
        .env("FINITE_BRAIN_PUBLIC_BASE_URL", url)
        .args(args)
        .args(["--server", url]);
    if json {
        command.arg("--json");
    }
    let mut child = command
        .stdout(stdout.reopen().unwrap())
        .stderr(stderr.reopen().unwrap())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "CLI deadline exceeded for {args:?}: {}",
                fs::read_to_string(stderr.path()).unwrap()
            );
        }
        thread::sleep(Duration::from_millis(10));
    };
    Output {
        status,
        stdout: fs::read(stdout.path()).unwrap(),
        stderr: fs::read(stderr.path()).unwrap(),
    }
}
pub(super) fn run(binary: &Path, home: &Path, cwd: &Path, url: &str, args: &[&str]) -> Value {
    let output = execute(binary, home, cwd, url, args, true);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
pub(super) fn home(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    fs::create_dir_all(&path).unwrap();
    path
}
pub(super) fn reference(binary: &Path, database: &Path) -> Server {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let url = format!("http://{address}");
    let child = Command::new(binary)
        .env_clear()
        .env("FINITE_BRAIN_ADDR", address.to_string())
        .env("FINITE_BRAIN_PUBLIC_BASE_URL", &url)
        .env("FINITE_BRAIN_DB", database)
        .env("FINITE_BRAIN_PROTECTED_RATE_LIMIT", "10000:60")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let authority = Server {
        url,
        shutdown: None,
        thread: None,
        child: Some(child),
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(1))
        .build();
    while agent
        .get(&format!("{}/health", authority.url))
        .call()
        .is_err()
    {
        assert!(
            Instant::now() < deadline,
            "reference server did not become healthy"
        );
        thread::sleep(Duration::from_millis(25));
    }
    authority
}
pub(super) fn schema_version(database: &Path) -> u32 {
    // This is a stopped, synthetic test database, never a snapshot or user DB.
    let connection =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    connection
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap()
}
