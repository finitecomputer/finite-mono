//! The daemon, supervisor and OS signals are real. Shell children qualify
//! lifecycle only; native Hermes authentication has a separate component proof.
use std::os::unix::fs::PermissionsExt;
use std::process::Stdio;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::process::Command;

#[tokio::test]
async fn stop_during_bridge_warmup_joins_hosted_child() {
    bridge_warmup_exit(true).await;
}

#[tokio::test]
async fn bridge_deadline_failure_joins_hosted_child() {
    bridge_warmup_exit(false).await;
}

async fn bridge_warmup_exit(signal: bool) {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    std::fs::write(
        home.join("config.json"),
        r#"{"account_id":"test-account","device_id":"test-device"}"#,
    )
    .unwrap();
    let write_script = |name, source| {
        let path = home.join(name);
        std::fs::write(&path, source).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    let sleeper = write_script("sleep-service", "#!/bin/sh\nexec sleep 60\n");
    let prepare = write_script("prepare", "#!/bin/sh\nexit 0\n");
    let bridge = write_script("bridge", "#!/bin/sh\nexit 1\n");
    write_script(
        "hermes",
        "#!/bin/sh\necho $$ > \"$HERMES_HOME/native.pid\"\nexec sleep 60\n",
    );
    let reservation = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    // The daemon starts the child only through Core's desired-state path.
    let core_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let core_url = format!("http://{}", core_listener.local_addr().unwrap());
    let core_server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        loop {
            let Ok((mut socket, _)) = core_listener.accept().await else {
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
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_finite-agentd"))
        .arg("serve")
        .env("FINITE_CORE_URL", core_url)
        .env("FINITE_CORE_CREDENTIAL", "a".repeat(64))
        .env(
            "PATH",
            format!("{}:{}", home.display(), std::env::var("PATH").unwrap()),
        )
        .env("FINITECHAT_HOME", home)
        .env("HERMES_HOME", home)
        .env("FINITECHAT_BIN", bridge)
        .env("FINITE_AGENTD_PREPARE_COMMAND", prepare)
        .env("FINITE_AGENTD_HERMES_COMMAND", &sleeper)
        .env("FINITE_AGENTD_HEALTH_PYTHON", &sleeper)
        .env("FINITE_AGENTD_SIMPLEX_SCRIPT", &sleeper)
        .env("FINITE_AGENTD_BRIDGE_ADDR", address.to_string())
        .env(
            "FINITE_AGENTD_BRIDGE_READY_TIMEOUT_SECS",
            if signal { "60" } else { "1" },
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
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let hosted_pid = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(pid) = std::fs::read_to_string(home.join("native.pid")) {
                break pid.trim().parse::<i32>().unwrap();
            }
            assert!(
                daemon.try_wait().unwrap().is_none(),
                "daemon exited before optional child started"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    if signal {
        rustix::process::kill_process(
            rustix::process::Pid::from_raw(daemon.id().unwrap() as i32).unwrap(),
            rustix::process::Signal::TERM,
        )
        .unwrap();
    }
    let exit = tokio::time::timeout(Duration::from_secs(5), daemon.wait())
        .await
        .unwrap()
        .unwrap();
    core_server.abort();
    assert_eq!(exit.success(), signal);
    assert!(
        rustix::process::test_kill_process(rustix::process::Pid::from_raw(hosted_pid).unwrap())
            .is_err(),
        "optional child outlived daemon"
    );
}
