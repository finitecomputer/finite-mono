//! Output observations through the real status writer, supervisor and
//! health server.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use finitechat_proto::DeviceRef;

use super::*;
use crate::daemon::{read_status, spawn_status_writer};
use crate::ledger::Ledger;
use crate::supervisor::{ProcessSpec, ProcessState, SupervisorHandle, start_supervisor};

fn sleeping(name: &'static str) -> ProcessSpec {
    ProcessSpec {
        name,
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".to_owned(), "exec sleep 30".to_owned()],
        environment: BTreeMap::new(),
    }
}

async fn stop(supervisor: &SupervisorHandle) {
    supervisor.shutdown().await;
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let processes = supervisor.status().await.processes;
            if processes
                .values()
                .all(|status| matches!(status.state, ProcessState::Stopped))
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("supervised test children must stop");
}

fn stuck_sample(observed_at_ms: u64) -> ContainerOutputStatus {
    ContainerOutputStatus {
        sample_interval_ms: 50,
        stdout: OutputStreamObservation::Unavailable {
            observed_at_ms,
            error: ObservationError::Other,
        },
        stderr: OutputStreamObservation::SamePipeAsStdout,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_status_heartbeat_keeps_writing_while_a_sample_is_stuck() {
    let home = tempfile::tempdir().unwrap();
    let status_path = home.path().join("agentd").join("status.json");
    let ledger = Ledger::open(home.path().join("agentd.sqlite3")).unwrap();
    let supervisor = start_supervisor(
        sleeping("finitechat"),
        sleeping("health"),
        sleeping("hermes"),
        None,
    );
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let (entered_tx, entered_rx) = mpsc::channel::<u64>();
    let mut calls = 0u64;
    let output = spawn_sampler(Duration::from_millis(50), move || {
        calls += 1;
        let _ = entered_tx.send(calls);
        if calls == 2 {
            // Returns early if the test fails and drops the sender.
            let _ = release_rx.recv_timeout(Duration::from_secs(60));
        }
        stuck_sample(calls)
    });
    tokio::task::spawn_blocking(move || {
        assert_eq!(entered_rx.recv_timeout(Duration::from_secs(5)), Ok(1));
        assert_eq!(entered_rx.recv_timeout(Duration::from_secs(5)), Ok(2));
    })
    .await
    .unwrap();

    spawn_status_writer(
        status_path.clone(),
        DeviceRef::new("acct", "dev"),
        ledger,
        supervisor.clone(),
        None,
        output,
    );

    let mut heartbeats = BTreeSet::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        while heartbeats.len() < 3 {
            if let Ok(status) = read_status(&status_path) {
                assert_eq!(status.container_output, Some(stuck_sample(1)));
                heartbeats.insert(status.updated_at_ms);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the heartbeat must keep advancing while the sampler is stuck");

    release_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(status) = read_status(&status_path)
                && status
                    .container_output
                    .is_some_and(|output| output != stuck_sample(1))
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the released sample must reach the next heartbeat");
    stop(&supervisor).await;
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io::{Read, Write};
    use std::os::fd::AsFd;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    use serde_json::{Value, json};

    use super::*;

    const SEAM_CHILD: &str = "output_pipe::seam_tests::linux::seam_child";
    const SEAM_REPORT_ENV: &str = "FINITE_AGENTD_OUTPUT_SEAM_REPORT";
    const HEALTH_SERVER: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../finitechat/containers/agent/health_server.py"
    );
    /// Serve the real health server on loopback and fetch both endpoints.
    const FETCH_HEALTH: &str = r#"
import importlib.util, json, sys, threading, urllib.error, urllib.request
spec = importlib.util.spec_from_file_location("agent_health_server", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
server = module.ThreadingHTTPServer(("127.0.0.1", 0), module.Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
results = {}
for path in ("/healthz", "/contact"):
    url = f"http://127.0.0.1:{server.server_port}{path}"
    try:
        with opener.open(url, timeout=10) as response:
            results[path] = {"status": response.status, "body": json.loads(response.read())}
    except urllib.error.HTTPError as error:
        results[path] = {"status": error.code, "body": json.loads(error.read())}
server.shutdown()
print(json.dumps(results))
"#;

    fn identity(path: impl AsRef<Path>) -> (u64, u64) {
        let metadata = std::fs::metadata(path).unwrap();
        (metadata.dev(), metadata.ino())
    }

    /// The parent half: the child test process gets a pipe this test owns as
    /// stdout and its read end as stdin, so the real observer samples a pipe
    /// that supervised children inherit, without replacing this process's
    /// own stdout.
    #[test]
    fn supervised_output_reaches_the_observer_and_health_without_changing_readiness() {
        let scratch = tempfile::tempdir().unwrap();
        let report_path = scratch.path().join("seam-report.json");
        let (output_reader, output_writer) = std::io::pipe().unwrap();
        let (mut diagnostics, diagnostics_writer) = std::io::pipe().unwrap();
        let mut child = {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", SEAM_CHILD, "--ignored", "--nocapture"])
                .args(["--test-threads", "1"])
                .env(SEAM_REPORT_ENV, &report_path)
                .stdin(output_reader)
                .stdout(output_writer)
                .stderr(diagnostics_writer);
            // Dropping the command closes this process's copies of the ends.
            command.spawn().unwrap()
        };
        let (diagnostics_tx, diagnostics_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = (&mut diagnostics).take(1 << 20).read_to_string(&mut text);
            let _ = diagnostics_tx.send(text);
        });
        let deadline = Instant::now() + Duration::from_secs(120);
        let exit = loop {
            if let Some(exit) = child.try_wait().unwrap() {
                break Some(exit);
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let diagnostics = diagnostics_rx
            .recv_timeout(Duration::from_secs(40))
            .unwrap_or_default();
        let report = std::fs::read_to_string(&report_path).unwrap_or_default();
        assert!(
            exit.is_some_and(|exit| exit.success()),
            "seam child failed or timed out: {exit:?}\n{diagnostics}\nreport: {report}"
        );
        let report: Value = serde_json::from_str(&report)
            .unwrap_or_else(|_| panic!("seam child wrote no report:\n{diagnostics}"));
        assert_eq!(report["completed"], true);
        eprintln!("{report:#}");
    }

    fn latest_stdout_pipe(output: &OutputObserver) -> Option<PipeObservation> {
        match output.latest()?.stdout {
            OutputStreamObservation::Pipe(observation) => Some(observation),
            _ => None,
        }
    }

    async fn wait_for_pipe(
        output: &OutputObserver,
        what: &str,
        mut accept: impl FnMut(&PipeObservation) -> bool,
    ) -> PipeObservation {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let Some(observation) = latest_stdout_pipe(output)
                    && accept(&observation)
                {
                    return observation;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("no stdout observation where {what}"))
    }

    async fn fetch_health(agent_home: &Path, fake_finitechat: &Path) -> Value {
        let mut command = Command::new("python3");
        command
            .args(["-c", FETCH_HEALTH, HEALTH_SERVER])
            .env("FINITECHAT_HOME", agent_home)
            .env("FINITECHAT_BIN", fake_finitechat)
            .env("FINITE_AGENTD_REQUIRED", "1")
            .env_remove("FINITE_AGENT_BOOT_INTENT_JSON")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = tokio::task::spawn_blocking(move || command.output())
            .await
            .unwrap()
            .expect("python3 must be on PATH for the health server seam");
        assert!(
            output.status.success(),
            "health server fetch failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn assert_ready_with_output(health: &Value) -> Value {
        for path in ["/healthz", "/contact"] {
            let response = &health[path];
            assert_eq!(response["status"], 200, "{path}: {response}");
            assert_eq!(response["body"]["ready"], true, "{path}: {response}");
            assert!(response["body"].get("ready_reason").is_none());
            assert_eq!(response["body"]["agentd"]["ok"], true);
            assert_eq!(response["body"]["agent_npub"], "npub1agent");
        }
        assert_eq!(health["/healthz"]["body"], health["/contact"]["body"]);
        let output = health["/contact"]["body"]["agentd"]["container_output"].clone();
        assert_eq!(output["state"], "reported", "{output}");
        output
    }

    /// The child half; it only runs when the parent test launches it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "launched by supervised_output_reaches_the_observer_and_health_..."]
    async fn seam_child() {
        let Some(report_path) = std::env::var_os(SEAM_REPORT_ENV) else {
            return;
        };
        // libtest announced this test on stdout; start from an empty pipe.
        let mut stdin = std::io::stdin();
        let preamble = rustix::io::ioctl_fionread(stdin.as_fd()).unwrap() as usize;
        stdin.read_exact(&mut vec![0u8; preamble]).unwrap();

        let output = OutputObserver::start();
        let home = tempfile::tempdir().unwrap();
        let agent_home = home.path().join("agent");
        std::fs::create_dir_all(agent_home.join("agentd")).unwrap();
        std::fs::write(agent_home.join("config.json"), "{}").unwrap();
        std::fs::write(
            agent_home.join("hermes-bridge-status.json"),
            r#"{"status":"connected","ok":true}"#,
        )
        .unwrap();
        let fake_finitechat = home.path().join("finitechat");
        std::fs::write(
            &fake_finitechat,
            "#!/bin/sh\nprintf '%s\\n' '{\"npub\":\"npub1agent\",\"account_id\":\"acct\"}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake_finitechat, std::fs::Permissions::from_mode(0o755)).unwrap();
        let marker = home.path().join("writer-finished");
        let gateway = ProcessSpec {
            name: "hermes",
            program: PathBuf::from("/bin/sh"),
            args: vec![
                "-c".to_owned(),
                format!(
                    "dd if=/dev/zero bs=65536 count=16 2>/dev/null; echo done > {}; exec sleep 30",
                    marker.display()
                ),
            ],
            environment: BTreeMap::new(),
        };
        let supervisor =
            start_supervisor(sleeping("finitechat"), sleeping("health"), gateway, None);
        let gateway_pid = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(pid) = supervisor
                    .status()
                    .await
                    .processes
                    .get("hermes")
                    .and_then(|status| status.pid())
                {
                    return pid;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();

        // The gateway's inherited stdout, this process's stdout and the read
        // end the test drains are one pipe object.
        let pipe = identity("/proc/self/fd/1");
        assert_eq!(identity(format!("/proc/{gateway_pid}/fd/1")), pipe);
        assert_eq!(identity("/proc/self/fd/0"), pipe);
        // The observer's own copies are close-on-exec: the gateway holds the
        // pipe through its stdout only.
        let mut gateway_pipe_fds: Vec<String> =
            std::fs::read_dir(format!("/proc/{gateway_pid}/fd"))
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    std::fs::metadata(entry.path())
                        .is_ok_and(|metadata| (metadata.dev(), metadata.ino()) == pipe)
                })
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
        gateway_pipe_fds.sort();
        assert_eq!(gateway_pipe_fds, ["1"]);

        let mut previous: Option<u64> = None;
        let full = wait_for_pipe(&output, "the pipe stays at capacity", |sample| {
            let stays_full = sample.buffered_bytes == sample.capacity_bytes
                && previous.is_some_and(|at| at < sample.observed_at_ms);
            if sample.buffered_bytes == sample.capacity_bytes {
                previous.get_or_insert(sample.observed_at_ms);
            }
            stays_full
        })
        .await;
        assert!(full.capacity_bytes > 0);
        assert!(
            !marker.exists(),
            "the gateway must still be waiting to write"
        );
        assert_eq!(
            supervisor.status().await.processes["hermes"].pid(),
            Some(gateway_pid),
            "the supervisor still reports the waiting gateway as running"
        );

        spawn_status_writer(
            agent_home.join("agentd").join("status.json"),
            DeviceRef::new("acct", "dev"),
            Ledger::open(home.path().join("agentd.sqlite3")).unwrap(),
            supervisor.clone(),
            None,
            output.clone(),
        );
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(status) = read_status(&agent_home.join("agentd").join("status.json"))
                    && status.container_output.is_some()
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the status heartbeat must publish the observation");

        let while_full = fetch_health(&agent_home, &fake_finitechat).await;
        let reported = assert_ready_with_output(&while_full);
        assert_eq!(reported["stdout"]["kind"], "pipe");
        assert_eq!(reported["stdout"]["buffered_bytes"], full.capacity_bytes);
        assert_eq!(reported["stdout"]["capacity_bytes"], full.capacity_bytes);
        assert_eq!(reported["stderr"]["kind"], "pipe");

        // Drain the pipe the way a returning collector would.
        std::thread::spawn(move || {
            let mut buffer = [0u8; 65536];
            while matches!(stdin.read(&mut buffer), Ok(read) if read > 0) {}
        });
        tokio::time::timeout(Duration::from_secs(30), async {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the gateway must finish its write once the pipe drains");
        // From a sample at capacity to a later empty sample the drops sum to
        // at least the capacity.
        let drained = wait_for_pipe(&output, "the drained pipe is empty", |sample| {
            sample.buffered_bytes == 0 && sample.observed_decrease_bytes >= full.capacity_bytes
        })
        .await;
        assert_eq!(drained.first_observed_at_ms, full.first_observed_at_ms);
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(status) = read_status(&agent_home.join("agentd").join("status.json"))
                    && let Some(ContainerOutputStatus {
                        stdout: OutputStreamObservation::Pipe(sample),
                        ..
                    }) = status.container_output
                    && sample.observed_at_ms >= drained.observed_at_ms
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the heartbeat must publish the drained observation");
        let after_drain = fetch_health(&agent_home, &fake_finitechat).await;
        let reported_after = assert_ready_with_output(&after_drain);
        assert_eq!(reported_after["stdout"]["buffered_bytes"], 0);
        assert!(
            reported_after["stdout"]["observed_decrease_bytes"]
                .as_u64()
                .is_some_and(|decrease| decrease >= full.capacity_bytes)
        );
        assert_eq!(
            supervisor.status().await.processes["hermes"].pid(),
            Some(gateway_pid)
        );
        stop(&supervisor).await;

        let report = json!({
            "completed": true,
            "pipe_identity": {"dev": pipe.0, "ino": pipe.1},
            "gateway_pid": gateway_pid,
            "observation_while_full": serde_json::to_value(&full).unwrap(),
            "health_while_full": while_full,
            "observation_after_drain": serde_json::to_value(&drained).unwrap(),
            "health_after_drain": after_drain,
        });
        std::fs::File::create(report_path)
            .and_then(|mut file| file.write_all(report.to_string().as_bytes()))
            .unwrap();
    }
}
