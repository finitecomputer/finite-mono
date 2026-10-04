use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use finite_identity::authority::{
    AuthorityConfig, AuthorityState, DevMailer, IdentityStore, SystemClock,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

// Synthetic fixture credentials, distinct from one another and never loaded
// from the developer's environment or used against an external service.
pub(super) const LOOKUP_TOKEN: &str = "synthetic-read-only-name-lookup";
pub(super) const OPERATOR_TOKEN: &str = "synthetic-directory-operator";

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
pub(super) fn brain(database: &Path, directory: Option<&str>) -> Server {
    let database = database.to_owned();
    let directory = directory.map(str::to_owned);
    serve(move |url| {
        let mut state = finite_brain_server::server_state_with_sqlite_path(database, url)
            .unwrap()
            .with_rate_limit(10_000, 60);
        if let Some(directory) = directory {
            state = state
                .with_directory_name_lookup(&directory, LOOKUP_TOKEN)
                .unwrap();
        }
        finite_brain_server::router_with_state(state)
    })
}
pub(super) fn directory(store: IdentityStore, public: bool) -> Server {
    serve(move |url| {
        let state = AuthorityState::new(
            store,
            Arc::new(DevMailer),
            SystemClock,
            AuthorityConfig {
                external_base_url: url.to_owned(),
                finite_vip_domain: "fixture.invalid".to_owned(),
                email_challenge_ttl_seconds: 600,
                operator_token: Some(OPERATOR_TOKEN.to_owned()),
            },
        )
        .with_name_lookup_token(Some(LOOKUP_TOKEN.to_owned()));
        if public {
            finite_identity::authority::public_router(state)
        } else {
            finite_identity::authority::router(state)
        }
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
