//! The daemon, supervisor and OS signals are real; the children are shell
//! stubs. A container stop reaches agentd as SIGTERM, and agentd must not
//! exit until Hermes has drained: exiting first drops the runtime, which
//! SIGKILLs Hermes before its adapter releases its inbox leases (the
//! interruption smoke's graceful-stop case exited 143 within ~150ms).
use std::os::unix::fs::PermissionsExt;
use std::process::Stdio;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::process::Command;

#[tokio::test]
async fn sigterm_waits_for_hermes_to_drain_while_the_sidecar_serves() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path();
    std::fs::write(
        home.join("config.json"),
        r#"{"account_id":"test-account","device_id":"test-device"}"#,
    )
    .unwrap();
    let write_script = |name, source: String| {
        let path = home.join(name);
        std::fs::write(&path, source).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    let h = home.display();
    let sleeper = write_script("sleep-service", "#!/bin/sh\nexec sleep 60\n".to_owned());
    let prepare = write_script("prepare", "#!/bin/sh\nexit 0\n".to_owned());
    // The sidecar never answers health, so agentd is still in bridge warmup;
    // that path and the steady-state path share the same awaited shutdown.
    // Other invocations (the boot admission seed) return at once.
    let sidecar = write_script(
        "sidecar",
        format!(
            "#!/bin/sh\ncase \" $* \" in *\" serve \"*) ;; *) exit 0 ;; esac\n\
             echo $$ > {h}/sidecar.pid\nexec sleep 60\n"
        ),
    );
    let hermes = write_script(
        "hermes-gateway",
        format!(
            "#!/bin/sh\n\
             trap 'sleep 1; if kill -0 \"$(cat {h}/sidecar.pid)\"; then echo alive; else echo gone; fi > {h}/drained; exit 0' TERM\n\
             echo $$ > {h}/hermes.pid\nsleep 60 & wait\n"
        ),
    );
    let reservation = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_finite-agentd"))
        .arg("serve")
        .env_remove("FINITE_CORE_URL")
        .env_remove("FINITE_CORE_CREDENTIAL")
        .env("FINITECHAT_HOME", home)
        .env("HERMES_HOME", home)
        .env("FINITECHAT_BIN", sidecar)
        .env("FINITE_AGENTD_PREPARE_COMMAND", prepare)
        .env("FINITE_AGENTD_HERMES_COMMAND", hermes)
        .env("FINITE_AGENTD_HEALTH_PYTHON", &sleeper)
        .env("FINITE_AGENTD_BRIDGE_ADDR", address.to_string())
        .env("FINITE_AGENTD_BRIDGE_READY_TIMEOUT_SECS", "60")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let read_pid = |name: &'static str| async move {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(raw) = std::fs::read_to_string(home.join(name))
                    && let Ok(pid) = raw.trim().parse::<i32>()
                {
                    break rustix::process::Pid::from_raw(pid).unwrap();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{name} was never written"))
    };
    let hermes_pid = read_pid("hermes.pid").await;
    let sidecar_pid = read_pid("sidecar.pid").await;

    rustix::process::kill_process(
        rustix::process::Pid::from_raw(daemon.id().unwrap() as i32).unwrap(),
        rustix::process::Signal::TERM,
    )
    .unwrap();
    let exit = tokio::time::timeout(Duration::from_secs(15), daemon.wait())
        .await
        .expect("agentd shutdown is bounded")
        .unwrap();

    assert!(exit.success(), "agentd exited {exit:?}");
    assert_eq!(
        std::fs::read_to_string(home.join("drained"))
            .ok()
            .as_deref(),
        Some("alive\n"),
        "Hermes must finish draining before agentd exits, with the sidecar still up"
    );
    for (name, pid) in [("Hermes", hermes_pid), ("sidecar", sidecar_pid)] {
        assert!(
            rustix::process::test_kill_process(pid).is_err(),
            "{name} outlived agentd"
        );
    }
}
