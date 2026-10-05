use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha256};

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

pub(super) fn server(database: &Path) -> Server {
    let database = database.to_owned();
    let (address_tx, address_rx) = mpsc::channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let thread = thread::spawn(move || {
        tokio::runtime::Runtime::new().unwrap().block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let state = finite_brain_server::server_state_with_sqlite_path(database, &url)
                .unwrap()
                .with_rate_limit(10_000, 60);
            address_tx.send(url).unwrap();
            tokio::select! {
                result = axum::serve(listener, finite_brain_server::router_with_state(state)) => result.unwrap(),
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

pub(super) fn execute(binary: &Path, home: &Path, cwd: &Path, url: &str, args: &[&str]) -> Output {
    execute_at(binary, home, cwd, url, url, args)
}

pub(super) fn execute_at(
    binary: &Path,
    home: &Path,
    cwd: &Path,
    transport: &str,
    origin: &str,
    args: &[&str],
) -> Output {
    Command::new(binary)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("FINITE_HOME", home)
        .env("FBRAIN_CONFIG_DIR", home.join("fbrain"))
        .env("FINITE_BRAIN_SERVER_URL", transport)
        .env("FINITE_BRAIN_PUBLIC_BASE_URL", origin)
        .args(args)
        .args(["--server", transport, "--json"])
        .output()
        .unwrap()
}

/// Forward authenticated requests unchanged, but drop the first successful
/// Member-removal response after the authority has committed it.
pub(super) struct LostResponseProxy {
    pub(super) url: String,
    pub(super) dropped: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl LostResponseProxy {
    pub(super) fn start(origin: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let destination = origin.strip_prefix("http://").unwrap().to_owned();
        let dropped = Arc::new(AtomicBool::new(false));
        let stopping = Arc::new(AtomicBool::new(false));
        let was_dropped = dropped.clone();
        let stop = stopping.clone();
        let thread = thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let (mut client, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("proxy accept: {error}"),
                };
                // Darwin inherits O_NONBLOCK from the listening socket.
                client.set_nonblocking(false).unwrap();
                client
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                client
                    .set_write_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut request = Vec::new();
                let header_end = loop {
                    let mut chunk = [0_u8; 8192];
                    let read = client.read(&mut chunk).unwrap();
                    assert!(read > 0, "proxy request ended before headers");
                    request.extend_from_slice(&chunk[..read]);
                    assert!(request.len() <= 8 * 1024 * 1024);
                    if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let headers = String::from_utf8(request[..header_end].to_vec()).unwrap();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                assert!(length <= 8 * 1024 * 1024);
                while request.len() < header_end + length {
                    let mut chunk = [0_u8; 8192];
                    let read = client.read(&mut chunk).unwrap();
                    assert!(read > 0, "proxy request ended before body");
                    request.extend_from_slice(&chunk[..read]);
                }
                let mut upstream = TcpStream::connect(&destination).unwrap();
                upstream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                upstream
                    .set_write_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                for line in headers.lines().filter(|line| {
                    !line.is_empty() && !line.to_ascii_lowercase().starts_with("connection:")
                }) {
                    write!(upstream, "{line}\r\n").unwrap();
                }
                upstream.write_all(b"Connection: close\r\n\r\n").unwrap();
                upstream
                    .write_all(&request[header_end..header_end + length])
                    .unwrap();
                let mut response = Vec::new();
                upstream
                    .take(8 * 1024 * 1024)
                    .read_to_end(&mut response)
                    .unwrap();
                let is_removal = headers.lines().next().is_some_and(|line| {
                    line.starts_with("DELETE /v1/admin/brains/revocation/members/")
                });
                if is_removal
                    && response.starts_with(b"HTTP/1.1 200")
                    && !was_dropped.swap(true, Ordering::AcqRel)
                {
                    continue;
                }
                client.write_all(&response).unwrap();
            }
        });
        Self {
            url,
            dropped,
            stopping,
            thread: Some(thread),
        }
    }
}

impl Drop for LostResponseProxy {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}

pub(super) fn run(binary: &Path, home: &Path, cwd: &Path, url: &str, args: &[&str]) -> Value {
    let output = execute(binary, home, cwd, url, args);
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

pub(super) fn candidate_binary() -> PathBuf {
    // Local qualification may use a byte-identical copy to avoid host launch
    // delays. CI uses the exact binary Cargo built for this integration test.
    let built = PathBuf::from(env!("CARGO_BIN_EXE_fbrain"));
    if let Some(copy) = std::env::var_os("FBRAIN_TEST_CANDIDATE_BINARY").map(PathBuf::from) {
        assert_eq!(
            Sha256::digest(fs::read(&built).unwrap()),
            Sha256::digest(fs::read(&copy).unwrap()),
            "candidate override must be byte-identical to this harness's Cargo-built CLI"
        );
        copy
    } else {
        built
    }
}

pub(super) fn create_fixture(binary: &Path, owner: &Path, target: &str, url: &str) {
    run(
        binary,
        owner,
        owner,
        url,
        &[
            "brain",
            "create",
            "revocation",
            "--kind",
            "organization",
            "--name",
            "Revocation",
        ],
    );
    run(
        binary,
        owner,
        owner,
        url,
        &[
            "collaborator",
            "ensure-admin",
            "--brain",
            "revocation",
            "--target",
            target,
        ],
    );
}

pub(super) fn reference_authority(binary: &Path, database: &Path) -> Server {
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
    while ureq::get(&format!("{}/health", authority.url))
        .call()
        .is_err()
    {
        assert!(
            Instant::now() < deadline,
            "reference server did not become healthy"
        );
        thread::sleep(Duration::from_millis(50));
    }
    authority
}
