//! Process-level lifecycle proof. Shell children stand in for the services;
//! the daemon, supervisor, loopback HTTP client, signals and ledger are real.
use std::{os::unix::fs::PermissionsExt, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    process::Command,
};

#[tokio::test]
async fn daemon_survives_absent_bridge_and_reconnects_without_restarting_hermes() {
    service_lifecycle(false).await;
}
#[tokio::test]
async fn invalid_optional_networking_config_does_not_terminate_chat() {
    service_lifecycle(true).await;
}
async fn service_lifecycle(invalid_networking: bool) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.json"),
        r#"{"account_id":"test-account","device_id":"test-device"}"#,
    )
    .unwrap();
    let sleeper = dir.path().join("service.sh");
    std::fs::write(&sleeper, "#!/bin/sh\nexec sleep 60\n").unwrap();
    std::fs::set_permissions(&sleeper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let prepare = dir.path().join("prepare.sh");
    let failed_bridge = dir.path().join("failed-bridge.sh");
    for (path, exit) in [(&prepare, 0), (&failed_bridge, 1)] {
        std::fs::write(path, format!("#!/bin/sh\nexit {exit}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let reservation = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let mut command = Command::new(env!("CARGO_BIN_EXE_finite-agentd"));
    if invalid_networking {
        command
            .env("FINITE_IROH_RELAY_URL", "not a URL")
            .env_remove("FINITE_CORE_URL")
            .env_remove("FINITE_CORE_CREDENTIAL");
    } else {
        command.env_remove("FINITE_IROH_RELAY_URL");
    }
    let mut daemon = command
        .arg("serve")
        .env("FINITECHAT_HOME", dir.path())
        .env("HERMES_HOME", dir.path().join("hermes"))
        .env("FINITECHAT_BIN", &failed_bridge)
        .env("FINITE_AGENTD_PREPARE_COMMAND", &prepare)
        .env("FINITE_AGENTD_HERMES_COMMAND", &sleeper)
        .env("FINITE_AGENTD_HEALTH_PYTHON", &sleeper)
        .env("FINITE_AGENTD_BRIDGE_ADDR", address.to_string())
        // Previous binaries exit after this deadline. It is now obsolete.
        .env("FINITE_AGENTD_BRIDGE_READY_TIMEOUT_SECS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let path = dir.path().join("agentd/status.json");
    let original = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(status) = finite_agentd::read_status(&path)
                && let Some(pid) = status
                    .processes
                    .processes
                    .get("hermes")
                    .and_then(|p| p.pid())
            {
                break pid;
            }
            assert!(daemon.try_wait().unwrap().is_none());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        daemon.try_wait().unwrap().is_none(),
        "bridge outage terminated daemon"
    );
    let listener = TcpListener::bind(address).await.unwrap();
    let (mut connection, _) = tokio::time::timeout(Duration::from_secs(8), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut request = [0; 4096];
    let count = tokio::time::timeout(Duration::from_secs(2), connection.read(&mut request))
        .await
        .unwrap()
        .unwrap();
    assert!(
        std::str::from_utf8(&request[..count])
            .unwrap()
            .starts_with("GET /v1/agentd/inbound?")
    );
    connection
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    drop(connection);
    // EOF must reconnect too, rather than silently stopping delivery.
    tokio::time::timeout(Duration::from_secs(8), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let status = finite_agentd::read_status(&path).unwrap();
    assert_eq!(status.processes.processes["hermes"].pid(), Some(original));
    assert!(status.processes.processes["finitechat"].restart_count > 0);
    let pid = rustix::process::Pid::from_raw(daemon.id().unwrap() as i32).unwrap();
    rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), daemon.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    assert!(
        rustix::process::test_kill_process(
            rustix::process::Pid::from_raw(original as i32).unwrap()
        )
        .is_err(),
        "daemon exited before draining child"
    );
}
