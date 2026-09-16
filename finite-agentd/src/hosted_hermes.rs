//! Optional native HTTPS-backend process. Trusted launch configuration owns
//! credentials; this module neither generates them nor edits Hermes state.
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, mpsc, watch};
use tokio::task::JoinHandle;

use crate::AgentdError;
use crate::supervisor::{
    ProcessSpec, ProcessState, ProcessStatus, now_ms, signal_group, spawn_process, terminate_child,
};

const ENABLED: &str = "FINITE_AGENTD_HOSTED_HERMES_ENABLED";
const BIND_ADDR: &str = "FINITE_AGENTD_HOSTED_HERMES_BIND_ADDR";
const PUBLIC_URL: &str = "HERMES_DASHBOARD_PUBLIC_URL";
const USERNAME: &str = "HERMES_DASHBOARD_BASIC_AUTH_USERNAME";
const PASSWORD: &str = "HERMES_DASHBOARD_BASIC_AUTH_PASSWORD";
const SECRET: &str = "HERMES_DASHBOARD_BASIC_AUTH_SECRET";

/// This handle owns only the optional native backend. Existing gateway,
/// health, Finite Chat and SimpleX lifecycle behavior is unchanged.
#[derive(Clone)]
pub(crate) struct HostedHermesHandle {
    pub(super) stop: mpsc::Sender<()>,
    pub(super) status: watch::Receiver<ProcessStatus>,
    pub(super) task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl HostedHermesHandle {
    pub(crate) fn start(home: &Path) -> Result<Option<Self>, AgentdError> {
        if std::env::var_os("FINITE_CORE_URL").is_some()
            || std::env::var_os("FINITE_CORE_CREDENTIAL").is_some()
        {
            return Ok(Some(crate::hosted_hermes_pull::start(home.to_path_buf())));
        }
        Ok(None)
    }

    pub(super) fn start_spec(spec: ProcessSpec) -> Self {
        let (stop, mut stop_rx) = mpsc::channel(1);
        let (status_tx, status) = watch::channel(ProcessStatus::default());
        let task = tokio::spawn(async move {
            let mut restart_count = 0;
            let mut retry = Duration::from_millis(250);
            let update = |state, restart_count| {
                status_tx.send_replace(ProcessStatus {
                    state,
                    restart_count,
                    updated_at_ms: now_ms(),
                });
            };
            loop {
                update(
                    if restart_count == 0 {
                        ProcessState::Starting
                    } else {
                        ProcessState::Restarting
                    },
                    restart_count,
                );
                match spawn_process(&spec) {
                    Ok((mut child, pid)) => {
                        update(ProcessState::Running { pid }, restart_count);
                        tokio::select! {
                            result = child.wait() => {
                                // Only this optional child's group is targeted.
                                signal_group(pid, rustix::process::Signal::KILL);
                                update(ProcessState::Exited {
                                    exit: result.map(|exit| exit.to_string()).unwrap_or_else(|error| error.to_string()),
                                }, restart_count);
                            }
                            _ = stop_rx.recv() => {
                                terminate_child(&mut child).await;
                                break;
                            }
                        }
                    }
                    Err(error) => update(
                        ProcessState::Unavailable {
                            error: error.public_message(),
                        },
                        restart_count,
                    ),
                }
                tokio::select! {
                    _ = tokio::time::sleep(retry) => {},
                    _ = stop_rx.recv() => break,
                }
                restart_count += 1;
                retry = (retry * 2).min(Duration::from_secs(5));
            }
            update(ProcessState::Stopped, restart_count);
        });
        Self {
            stop,
            status,
            task: Arc::new(Mutex::new(Some(task))),
        }
    }

    pub(crate) fn status(&self) -> ProcessStatus {
        self.status.borrow().clone()
    }

    pub(crate) async fn shutdown(&self) {
        let _ = self.stop.send(()).await;
        // Retain the join handle if the caller is cancelled while waiting.
        let mut task = self.task.lock().await;
        if let Some(task) = task.as_mut() {
            let _ = task.await;
        }
        task.take();
    }
}

pub(super) fn configured_spec(
    environment: BTreeMap<String, String>,
) -> Result<ProcessSpec, AgentdError> {
    HostedHermesConfig::read(|key| environment.get(key).cloned())?;
    Ok(ProcessSpec {
        name: "hermes-serve",
        program: std::env::current_exe()?,
        args: vec!["hosted-hermes".into()],
        environment,
    })
}

// No Debug/Serialize: future settings must not accidentally expose secrets.
struct HostedHermesConfig {
    bind_addr: SocketAddr,
    public_url: reqwest::Url,
    home: PathBuf,
}

impl HostedHermesConfig {
    fn read(mut value: impl FnMut(&str) -> Option<String>) -> Result<Self, AgentdError> {
        if !matches!(value(ENABLED).as_deref(), Some("1" | "true")) {
            return Err(invalid(ENABLED));
        }
        let home = PathBuf::from(value("HERMES_HOME").ok_or_else(|| invalid("HERMES_HOME"))?);
        if !home.is_absolute() {
            return Err(invalid("HERMES_HOME"));
        }
        let bind_addr: SocketAddr = value(BIND_ADDR)
            .ok_or_else(|| invalid(BIND_ADDR))?
            .parse()
            .map_err(|_| invalid(BIND_ADDR))?;
        if bind_addr.port() == 0 || bind_addr.ip().is_multicast() {
            return Err(invalid(BIND_ADDR));
        }
        let public_url =
            reqwest::Url::parse(&value(PUBLIC_URL).ok_or_else(|| invalid(PUBLIC_URL))?)
                .map_err(|_| invalid(PUBLIC_URL))?;
        let host = public_url
            .host_str()
            .unwrap_or_default()
            .trim_end_matches('.');
        let ip = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>();
        if public_url.scheme() != "https"
            || host.is_empty()
            || host == "localhost"
            || host.ends_with(".localhost")
            || ip.is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified() || ip.is_multicast())
            || host.contains(['*', '{', '}'])
            || !public_url.username().is_empty()
            || public_url.password().is_some()
            || public_url.port() == Some(0)
            || public_url.query().is_some()
            || public_url.fragment().is_some()
        {
            return Err(invalid(PUBLIC_URL));
        }
        for key in [USERNAME, PASSWORD, SECRET] {
            let credential = value(key).ok_or_else(|| invalid(key))?;
            // Native basic auth trims environment values. Reject ambiguous
            // whitespace rather than silently changing a supplied credential.
            if credential.is_empty()
                || credential.len() > 4096
                || credential.trim() != credential
                || credential.chars().any(char::is_control)
            {
                return Err(invalid(key));
            }
        }
        Ok(Self {
            bind_addr,
            public_url,
            home,
        })
    }
}

fn invalid(key: &str) -> AgentdError {
    AgentdError::Config(format!("Hosted Hermes requires valid {key}"))
}

/// Replace this supervised child with native Hermes. Configuration errors stop
/// only this optional child; existing gateway and SimpleX supervision continues.
pub fn run_hosted_hermes() -> Result<(), AgentdError> {
    let config = HostedHermesConfig::read(|key| std::env::var(key).ok())?;
    let error = std::process::Command::new("hermes")
        .args([
            "serve",
            "--isolated",
            "--host",
            &config.bind_addr.ip().to_string(),
            "--port",
            &config.bind_addr.port().to_string(),
            "--no-open",
        ])
        .env(PUBLIC_URL, config.public_url.as_str())
        .env("HERMES_HOME", &config.home)
        // Accepted work survives a client disconnect. Process termination is
        // a separate interruption boundary, not a promise of work survival.
        .env("HERMES_TUI_WS_ORPHAN_REAP_GRACE_S", "0")
        .exec();
    Err(error.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> BTreeMap<&'static str, String> {
        BTreeMap::from([
            (ENABLED, "1".into()),
            ("HERMES_HOME", "/data/agent/hermes-home".into()),
            (BIND_ADDR, "0.0.0.0:8642".into()),
            (
                PUBLIC_URL,
                "https://agents.example.test/runtimes/runtime_test/".into(),
            ),
            (USERNAME, "synthetic-user".into()),
            (PASSWORD, "synthetic-test-password".into()),
            (SECRET, "synthetic-test-signing-secret".into()),
        ])
    }

    #[test]
    fn invalid_internal_child_configuration_is_rejected() {
        let mut values = settings();
        values.insert(ENABLED, "invalid".into());
        assert!(HostedHermesConfig::read(|key| values.get(key).cloned()).is_err());
    }

    #[test]
    fn missing_or_malformed_credentials_fail_without_echoing_values() {
        for key in [USERNAME, PASSWORD, SECRET] {
            let mut values = settings();
            values.remove(key);
            assert!(HostedHermesConfig::read(|key| values.get(key).cloned()).is_err());
            for bad in ["", " secret ", "secret\nvalue"] {
                values.insert(key, bad.into());
                let error = HostedHermesConfig::read(|key| values.get(key).cloned())
                    .err()
                    .expect("must reject invalid credentials");
                assert_eq!(
                    error.public_message(),
                    format!("Hosted Hermes requires valid {key}")
                );
            }
        }
    }

    #[test]
    fn public_url_must_engage_native_auth_even_with_loopback_backend() {
        let mut values = settings();
        values.insert(BIND_ADDR, "127.0.0.1:8642".into());
        assert!(HostedHermesConfig::read(|key| values.get(key).cloned()).is_ok());
        for url in [
            "http://agents.example.test",
            "https://localhost",
            "https://localhost.",
            "https://api.localhost",
            "https://127.0.0.2",
            "https://[::1]",
            "https://0.0.0.0",
            "https://[::]",
            "https://user:password@agents.example.test",
            "https://agents.example.test:0",
            "https://agents.example.test/?ticket=secret",
            "https://agents.example.test/#secret",
        ] {
            values.insert(PUBLIC_URL, url.into());
            assert!(
                HostedHermesConfig::read(|key| values.get(key).cloned()).is_err(),
                "{url}"
            );
        }
    }

    #[test]
    fn listener_requires_an_explicit_fixed_address_and_port() {
        let mut values = settings();
        for address in ["", "8642", "localhost:8642", "0.0.0.0:0", "224.0.0.1:8642"] {
            values.insert(BIND_ADDR, address.into());
            assert!(
                HostedHermesConfig::read(|key| values.get(key).cloned()).is_err(),
                "{address}"
            );
        }
    }

    fn sleeper(name: &'static str) -> ProcessSpec {
        ProcessSpec {
            name,
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "exec sleep 60".into()],
            environment: BTreeMap::new(),
        }
    }

    async fn running(handle: &HostedHermesHandle, after: Option<u32>) -> u32 {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(pid) = handle.status().pid()
                    && Some(pid) != after
                {
                    return pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("hosted child must run")
    }

    #[tokio::test]
    async fn hosted_restart_and_shutdown_leave_gateway_and_simplex_untouched() {
        // Real OS processes qualify only our lifecycle boundary. They do not
        // substitute for the separate pinned native-Hermes protocol proof.
        let existing = crate::supervisor::start_supervisor(
            sleeper("finitechat"),
            sleeper("health"),
            sleeper("hermes"),
            Some(sleeper("simplex")),
        );
        let original = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let status = existing.status().await;
                if status.processes.len() == 4
                    && status.processes.values().all(|entry| entry.pid().is_some())
                {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let hosted = HostedHermesHandle::start_spec(sleeper("hermes-serve"));
        let first = running(&hosted, None).await;
        rustix::process::kill_process(
            rustix::process::Pid::from_raw(first as i32).unwrap(),
            rustix::process::Signal::KILL,
        )
        .unwrap();
        let replacement = running(&hosted, Some(first)).await;
        assert!(hosted.status().restart_count > 0);
        hosted.shutdown().await;
        assert!(matches!(hosted.status().state, ProcessState::Stopped));
        assert!(
            rustix::process::test_kill_process(
                rustix::process::Pid::from_raw(replacement as i32).unwrap()
            )
            .is_err()
        );
        let after = existing.status().await;
        for (name, before) in original.processes {
            assert_eq!(
                after.processes[&name].pid(),
                before.pid(),
                "{name} restarted"
            );
        }
        existing.shutdown().await;
    }

    #[tokio::test]
    async fn hosted_shutdown_interrupts_spawn_failure_backoff() {
        let mut spec = sleeper("hermes-serve");
        spec.program = "/nonexistent/finite-hosted-proof".into();
        let handle = HostedHermesHandle::start_spec(spec);
        tokio::time::timeout(Duration::from_secs(5), async {
            while !matches!(handle.status().state, ProcessState::Unavailable { .. }) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(1), handle.shutdown())
            .await
            .unwrap();
        assert!(matches!(handle.status().state, ProcessState::Stopped));
    }
}
