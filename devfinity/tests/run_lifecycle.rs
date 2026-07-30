#![cfg(all(unix, debug_assertions))]

use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn sigterm_before_readiness_cleans_processes_port_and_temporary_state() {
    let fixture = tempfile::tempdir().unwrap();
    let state_root = fixture.path().join("state");
    let readiness_barrier = fixture.path().join("services-ready");
    let command_marker = fixture.path().join("command-ran");
    let child = Command::new(env!("CARGO_BIN_EXE_devfinity"))
        .args(["--state-dir"])
        .arg(&state_root)
        .args(["run", "--", "sh", "-c", "printf ran > \"$1\"", "test"])
        .arg(&command_marker)
        .env("DEVFINITY_TEST_HOLD_BEFORE_READY_FILE", &readiness_barrier)
        .env("DEVFINITY_READY_TIMEOUT_SECS", "60")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard::new(child);

    wait_for_path(
        child.child_mut(),
        &readiness_barrier,
        Duration::from_secs(60),
    );
    let attempt_dir = only_child_dir(&state_root);
    let run_dir = attempt_dir.join("runs/default");
    let process_compose_pid = read_pid(&run_dir.join("pids/process-compose.pid"));
    let postgres_wrapper_pid = read_pid(&run_dir.join("pids/postgres.pid"));
    let postmaster_pid = read_pid(&run_dir.join("postgres/data/postmaster.pid"));
    let postgres_address = postgres_address(&run_dir.join("urls.txt"));
    assert!(
        TcpStream::connect_timeout(&postgres_address, Duration::from_secs(1)).is_ok(),
        "managed Postgres should be accepting connections at the readiness barrier"
    );
    assert!(
        !command_marker.exists(),
        "the wrapped command must not start before devfinity reports readiness"
    );

    let signal = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(signal.success());
    let output = child.wait_with_output().unwrap();

    assert_eq!(
        output.status.code(),
        Some(143),
        "devfinity did not preserve SIGTERM exit status\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !command_marker.exists(),
        "the wrapped command unexpectedly ran"
    );
    assert!(!attempt_dir.exists(), "temporary run state was not removed");
    assert_eq!(fs::read_dir(&state_root).unwrap().count(), 0);
    for (label, pid) in [
        ("process-compose", process_compose_pid),
        ("Postgres wrapper", postgres_wrapper_pid),
        ("Postgres postmaster", postmaster_pid),
    ] {
        assert!(
            !process_alive(pid),
            "{label} pid {pid} survived interrupted startup"
        );
    }
    assert!(
        TcpStream::connect_timeout(&postgres_address, Duration::from_millis(250)).is_err(),
        "managed Postgres port {postgres_address} remained open after cleanup"
    );
}

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().unwrap()
    }

    fn id(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    fn wait_with_output(mut self) -> std::io::Result<std::process::Output> {
        self.child.take().unwrap().wait_with_output()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        let _ = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status();
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(15) {
            if child.try_wait().is_ok_and(|status| status.is_some()) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn wait_for_path(child: &mut Child, path: &Path, timeout: Duration) {
    let started = Instant::now();
    loop {
        if path.exists() {
            return;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "devfinity exited with {status} before reaching the readiness barrier {}",
                path.display()
            );
        }
        assert!(
            started.elapsed() < timeout,
            "timed out waiting for readiness barrier {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn only_child_dir(root: &Path) -> PathBuf {
    let entries = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 1, "expected one isolated run in {root:?}");
    entries.into_iter().next().unwrap()
}

fn read_pid(path: &Path) -> u32 {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
        .lines()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

fn postgres_address(path: &Path) -> SocketAddr {
    let urls = fs::read_to_string(path).unwrap();
    urls.lines()
        .find_map(|line| line.strip_prefix("postgres="))
        .unwrap()
        .parse()
        .unwrap()
}

fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}
