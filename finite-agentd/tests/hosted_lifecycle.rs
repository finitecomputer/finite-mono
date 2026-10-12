//! The daemon, supervisor and OS signals are real. Shell children qualify
//! lifecycle only; native Hermes authentication has a separate component proof.
//!
//! The daemon and everything it starts see a PATH of the test's own directory
//! and the system directories only, so no program is ever resolved from the
//! host's PATH. Every wait for another process is a bounded poll for a
//! positive signal, and a failing test stops everything it started before it
//! panics.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::net::TcpListener;
use tokio::process::{Child, Command};

/// How long a test waits for another process before it fails.
const WAIT: Duration = Duration::from_secs(30);
/// The system directories the stand-in scripts need (`sleep`).
const SYSTEM_PATH: &str = "/usr/bin:/bin";

#[tokio::test]
async fn stop_during_bridge_warmup_joins_hosted_child() {
    bridge_warmup_exit(true).await;
}

#[tokio::test]
async fn bridge_deadline_failure_joins_hosted_child() {
    bridge_warmup_exit(false).await;
}

#[tokio::test]
async fn a_missing_stand_in_is_never_resolved_from_the_host_path() {
    let lab = Lab::new();
    // The stand-in is gone: the hosted child must fail to exec, and nothing
    // else named `hermes` may run in its place.
    std::fs::remove_file(lab.home().join("hermes")).unwrap();
    let stderr = lab.home().join("daemon.stderr");
    let daemon = lab.start(true, Some(&stderr)).await;
    let failure = "hosted Hermes could not start `hermes serve`";
    poll("the hosted child to fail to exec, twice", || {
        std::fs::read_to_string(&stderr)
            .unwrap_or_default()
            .matches(failure)
            .count()
            >= 2
    })
    .await;
    assert!(
        !lab.home().join("native.pid").exists(),
        "nothing else started"
    );
    daemon.stop().await;
}

async fn bridge_warmup_exit(signal: bool) {
    let lab = Lab::new();
    let mut daemon = lab.start(signal, None).await;
    let hosted_pid = poll_value("the optional child to start", || {
        assert!(
            daemon.child.try_wait().unwrap().is_none(),
            "daemon exited before optional child started"
        );
        complete_pid(&lab.home().join("native.pid"))
    })
    .await;
    if signal {
        rustix::process::kill_process(
            rustix::process::Pid::from_raw(daemon.pid as i32).unwrap(),
            rustix::process::Signal::TERM,
        )
        .unwrap();
    }
    // Without SIGTERM the daemon exits when its 30 s bridge deadline fails,
    // then stops its children.
    let exit = tokio::time::timeout(WAIT * 3, daemon.child.wait())
        .await
        .expect("the daemon exits")
        .unwrap();
    assert_eq!(exit.success(), signal);
    assert!(
        rustix::process::test_kill_process(rustix::process::Pid::from_raw(hosted_pid).unwrap())
            .is_err(),
        "optional child outlived daemon"
    );
    daemon.stopped = true;
}

/// The test's directory, stand-ins, and fake Core.
struct Lab {
    directory: tempfile::TempDir,
    core_url: String,
    core_server: tokio::task::JoinHandle<()>,
}

impl Lab {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        std::fs::write(
            home.join("config.json"),
            r#"{"account_id":"test-account","device_id":"test-device"}"#,
        )
        .unwrap();
        let pids = home.join("started.pids");
        write_script(
            home,
            "sleep-service",
            &format!(
                "#!/bin/sh\necho $$ >> '{}'\nexec sleep 60\n",
                pids.display()
            ),
        );
        write_script(home, "prepare", "#!/bin/sh\nexit 0\n");
        write_script(home, "bridge", "#!/bin/sh\nexit 1\n");
        write_script(
            home,
            "hermes",
            "#!/bin/sh\necho $$ > \"$HERMES_HOME/native.pid\"\nexec sleep 60\n",
        );
        let (core_url, core_server) = fake_core();
        Self {
            directory,
            core_url,
            core_server,
        }
    }

    fn home(&self) -> &Path {
        self.directory.path()
    }

    /// Starts `finite-agentd serve` with a bridge that never becomes ready: a
    /// 60 s bridge deadline when the test stops it with SIGTERM, 30 s when the
    /// test waits for the deadline to fail it.
    async fn start(&self, signal: bool, stderr: Option<&Path>) -> Daemon {
        let home = self.home();
        let reservation = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = reservation.local_addr().unwrap();
        drop(reservation);
        let sleeper = home.join("sleep-service");
        let stderr = match stderr {
            Some(path) => Stdio::from(std::fs::File::create(path).unwrap()),
            None => Stdio::null(),
        };
        let child = Command::new(env!("CARGO_BIN_EXE_finite-agentd"))
            .arg("serve")
            .env("FINITE_CORE_URL", &self.core_url)
            .env("FINITE_CORE_CREDENTIAL", "a".repeat(64))
            .env("PATH", format!("{}:{SYSTEM_PATH}", home.display()))
            .env("FINITECHAT_HOME", home)
            .env("HERMES_HOME", home)
            .env("FINITECHAT_BIN", home.join("bridge"))
            .env("FINITE_AGENTD_PREPARE_COMMAND", home.join("prepare"))
            .env("FINITE_AGENTD_HERMES_COMMAND", &sleeper)
            .env("FINITE_AGENTD_HEALTH_PYTHON", &sleeper)
            .env("FINITE_AGENTD_SIMPLEX_SCRIPT", &sleeper)
            .env("FINITE_AGENTD_BRIDGE_ADDR", address.to_string())
            .env(
                "FINITE_AGENTD_BRIDGE_READY_TIMEOUT_SECS",
                if signal { "60" } else { "30" },
            )
            .env("FINITE_AGENTD_HOSTED_HERMES_ENABLED", "1")
            .env("FINITE_AGENTD_HOSTED_HERMES_BIND_ADDR", "127.0.0.1:8642")
            .env(
                "HERMES_DASHBOARD_PUBLIC_URL",
                "https://agents.example.test/runtimes/test/",
            )
            .env("HERMES_DASHBOARD_BASIC_AUTH_USERNAME", "test-user")
            .env("HERMES_DASHBOARD_BASIC_AUTH_PASSWORD", "test-password")
            .env("HERMES_DASHBOARD_BASIC_AUTH_SECRET", "test-signing-secret")
            .stdout(Stdio::null())
            .stderr(stderr)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        Daemon {
            pid: child.id().unwrap(),
            child,
            home: home.to_owned(),
            stopped: false,
        }
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        self.core_server.abort();
    }
}

/// The daemon under test. If it is dropped before the test marked it stopped
/// (a failed assertion), it stops the daemon, which stops its children, and
/// then kills every process a stand-in recorded, with its process group. The
/// `Lab` (and so the stand-ins) outlives it, so no child can exec a program
/// after its stand-in was deleted.
struct Daemon {
    pid: u32,
    child: Child,
    home: PathBuf,
    stopped: bool,
}

impl Daemon {
    /// Stops the daemon with SIGTERM and waits (bounded) for it and every
    /// recorded child to be gone.
    async fn stop(mut self) {
        signal(self.pid, rustix::process::Signal::TERM);
        tokio::time::timeout(WAIT, self.child.wait())
            .await
            .expect("the daemon exits on SIGTERM")
            .unwrap();
        let recorded = recorded_pids(&self.home);
        poll("every recorded child to exit", || {
            recorded.iter().all(|pid| !alive(*pid))
        })
        .await;
        self.stopped = true;
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if self.stopped {
            return;
        }
        signal(self.pid, rustix::process::Signal::TERM);
        let deadline = Instant::now() + WAIT;
        let mut exited = false;
        while !exited && Instant::now() < deadline {
            exited = self.child.try_wait().ok().flatten().is_some();
            std::thread::sleep(Duration::from_millis(50));
        }
        if !exited {
            signal(self.pid, rustix::process::Signal::KILL);
        }
        for pid in recorded_pids(&self.home) {
            if let Some(pid) = rustix::process::Pid::from_raw(pid) {
                let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
                let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
            }
        }
    }
}

fn write_script(home: &Path, name: &str, source: &str) {
    let path = home.join(name);
    std::fs::write(&path, source).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A pid from a file that holds one complete line, never a partial write.
fn complete_pid(path: &Path) -> Option<i32> {
    let text = std::fs::read_to_string(path).ok()?;
    text.strip_suffix('\n')?.trim().parse().ok()
}

/// Every pid the stand-ins recorded, from complete lines only.
fn recorded_pids(home: &Path) -> Vec<i32> {
    let mut pids = std::fs::read_to_string(home.join("started.pids"))
        .unwrap_or_default()
        .split_inclusive('\n')
        .filter(|line| line.ends_with('\n'))
        .filter_map(|line| line.trim().parse().ok())
        .collect::<Vec<_>>();
    pids.extend(complete_pid(&home.join("native.pid")));
    pids
}

fn signal(pid: u32, signal: rustix::process::Signal) {
    if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
        let _ = rustix::process::kill_process(pid, signal);
    }
}

fn alive(pid: i32) -> bool {
    rustix::process::Pid::from_raw(pid)
        .is_some_and(|pid| rustix::process::test_kill_process(pid).is_ok())
}

async fn poll(what: &str, mut condition: impl FnMut() -> bool) {
    poll_value(what, || condition().then_some(())).await;
}

/// Polls every 20 ms, for at most `WAIT`, until `value` returns something.
async fn poll_value<T>(what: &str, mut value: impl FnMut() -> Option<T>) -> T {
    tokio::time::timeout(WAIT, async {
        loop {
            if let Some(value) = value() {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out after {WAIT:?} waiting for {what}"))
}

/// The daemon starts the child only through Core's desired-state path.
fn fake_core() -> (String, tokio::task::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let listener = TcpListener::from_std(listener).unwrap();
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let mut request = [0; 8192];
            let _ = socket.read(&mut request).await;
            let body = serde_json::json!({"runtimeId":"test", "generation":1, "enabled":true,
                "publicUrl":"https://agents.example.test/runtimes/test/", "username":"synthetic-user",
                "password":"synthetic-password", "signingSecret":"synthetic-signing-secret", "accessTtlSeconds":60}).to_string();
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(reply.as_bytes()).await;
        }
    });
    (url, server)
}
