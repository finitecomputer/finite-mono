//! Core assignment-authenticated desired state for the optional native child.
//! Runner alone injects FINITE_CORE_URL/CREDENTIAL after authenticated creation
//! lease issuance; both names are reserved against customer RuntimeSpec input.
//! See runner::with_runtime_core_bootstrap and Core runtime_credentials. Native
//! passwords originate in Core and reach only the supervised native process.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use tokio::sync::{Mutex, mpsc, watch};

use crate::AgentdError;
use crate::hosted_hermes::{HostedHermesHandle, configured_spec};
use crate::supervisor::{ProcessState, ProcessStatus, now_ms};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const NATIVE_BASE: &str = "http://127.0.0.1:8642";

// No Debug/Serialize: this response contains native runtime credentials.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Desired {
    runtime_id: String,
    generation: u64,
    enabled: bool,
    public_url: Option<String>,
    username: Option<String>,
    password: Option<String>,
    signing_secret: Option<String>,
    access_ttl_seconds: u64,
}

impl Desired {
    fn spec(&self, home: &Path) -> Result<crate::supervisor::ProcessSpec, AgentdError> {
        if self.access_ttl_seconds != 60 {
            return Err(invalid("native session lifetime"));
        }
        let required =
            |value: &Option<String>| value.clone().ok_or_else(|| invalid("native credentials"));
        configured_spec(BTreeMap::from([
            ("FINITE_AGENTD_HOSTED_HERMES_ENABLED".into(), "1".into()),
            (
                "FINITE_AGENTD_HOSTED_HERMES_BIND_ADDR".into(),
                "0.0.0.0:8642".into(),
            ),
            ("HERMES_HOME".into(), home.display().to_string()),
            (
                "HERMES_DASHBOARD_PUBLIC_URL".into(),
                required(&self.public_url)?,
            ),
            (
                "HERMES_DASHBOARD_BASIC_AUTH_USERNAME".into(),
                required(&self.username)?,
            ),
            (
                "HERMES_DASHBOARD_BASIC_AUTH_PASSWORD".into(),
                required(&self.password)?,
            ),
            (
                "HERMES_DASHBOARD_BASIC_AUTH_SECRET".into(),
                required(&self.signing_secret)?,
            ),
            (
                "HERMES_DASHBOARD_BASIC_AUTH_TTL_SECONDS".into(),
                "60".into(),
            ),
        ]))
    }
}

struct CoreConnection {
    client: Client,
    url: String,
    credential: String,
}
enum Pull {
    Desired(Desired),
    Revoked,
}

impl CoreConnection {
    fn from_env() -> Result<Self, AgentdError> {
        Self::new(
            std::env::var("FINITE_CORE_URL").unwrap_or_default(),
            std::env::var("FINITE_CORE_CREDENTIAL").unwrap_or_default(),
        )
    }

    fn new(origin: String, credential: String) -> Result<Self, AgentdError> {
        let origin = Url::parse(&origin).map_err(|_| invalid("Core origin"))?;
        let local = matches!(origin.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if !(origin.scheme() == "https" || origin.scheme() == "http" && local)
            || origin.host_str().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
            || credential.len() != 64
            || !credential
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(invalid("Core assignment configuration"));
        }
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            url: format!("{}api/core/v1/runtime/hosted-hermes", origin.as_str()),
            credential,
        })
    }

    async fn pull(&self) -> Result<Pull, AgentdError> {
        let response = self
            .client
            .get(&self.url)
            .bearer_auth(&self.credential)
            .send()
            .await?;
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            return Ok(Pull::Revoked);
        }
        let desired: Desired = bounded_json(response.error_for_status()?).await?;
        if desired.generation == 0
            || desired.runtime_id.is_empty()
            || desired.runtime_id.len() > 128
        {
            return Err(invalid("Core desired-state binding"));
        }
        Ok(Pull::Desired(desired))
    }

    async fn report(&self, generation: u64, applied: bool) -> Result<bool, AgentdError> {
        let response = self.client.post(format!("{}/report", self.url)).bearer_auth(&self.credential)
            .json(&serde_json::json!({"generation": generation, "status": if applied { "applied" } else { "error" }})).send().await?;
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            return Ok(false);
        }
        response.error_for_status()?;
        Ok(true)
    }
}

async fn bounded_json<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
) -> Result<T, AgentdError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 32 * 1024 {
            return Err(invalid("bounded HTTP response"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid("HTTP response schema"))
}

async fn native_ready(client: &Client, base: &str, desired: &Desired) -> Result<(), AgentdError> {
    // Production callers always pass NATIVE_BASE, not a Core-provided target.
    // Public /api/status is not an authentication or readiness check.
    let anonymous = client.get(format!("{base}/api/auth/me")).send().await?;
    if anonymous.status() != StatusCode::UNAUTHORIZED {
        return Err(invalid("native auth rejection"));
    }
    let login = client.post(format!("{base}/auth/password-login"))
        .json(&serde_json::json!({"provider":"basic", "username":desired.username, "password":desired.password}))
        .send().await?.error_for_status()?;
    let token = login
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .filter_map(|value| value.split_once('='))
        .find(|(name, _)| name.ends_with("hermes_session_at"))
        .map(|(_, value)| value.to_owned())
        .ok_or_else(|| invalid("native session cookie"))?;
    let response = client
        .get(format!("{base}/api/auth/me"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?;
    let identity: serde_json::Value = bounded_json(response).await?;
    if !valid_native_identity(&identity, desired.username.as_deref(), now_ms() / 1000) {
        return Err(invalid("native session identity"));
    }
    Ok(())
}

fn valid_native_identity(identity: &serde_json::Value, username: Option<&str>, now: u64) -> bool {
    username.is_some()
        && identity.get("provider").and_then(|v| v.as_str()) == Some("basic")
        && identity.get("user_id").and_then(|v| v.as_str()) == username
        && identity
            .get("expires_at")
            .and_then(|v| v.as_u64())
            .is_some_and(|expiry| expiry > now && expiry <= now + 60)
}

#[derive(Default)]
struct Applied {
    runtime_id: Option<String>,
    generation: Option<u64>,
    child: Option<HostedHermesHandle>,
    verified_pid: Option<u32>,
    reported: Option<bool>,
}

impl Applied {
    async fn stop(&mut self) {
        if let Some(child) = &self.child {
            child.shutdown().await;
        }
        self.child = None;
        self.verified_pid = None;
        self.reported = None;
    }

    async fn reconcile(
        &mut self,
        core: &CoreConnection,
        home: &Path,
        status: &watch::Sender<ProcessStatus>,
    ) {
        let desired = match core.pull().await {
            Ok(Pull::Desired(desired)) => desired,
            Ok(Pull::Revoked) => {
                self.stop().await;
                self.generation = None;
                status.send_replace(unavailable("Core assignment authorization was revoked"));
                return;
            }
            Err(_) => return, // retain last applied configuration on transport/schema failure
        };
        if self
            .runtime_id
            .as_ref()
            .is_some_and(|id| id != &desired.runtime_id)
        {
            self.stop().await;
            status.send_replace(unavailable("Core assignment Runtime changed"));
            return;
        }
        self.runtime_id = Some(desired.runtime_id.clone());
        if self
            .generation
            .is_some_and(|generation| desired.generation < generation)
        {
            return; // a stale response cannot roll back an applied generation
        }
        if self.generation != Some(desired.generation) {
            self.stop().await;
            self.generation = Some(desired.generation);
            if desired.enabled {
                match desired.spec(home) {
                    Ok(spec) => self.child = Some(HostedHermesHandle::start_spec(spec)),
                    Err(_) => {
                        status.send_replace(unavailable("Core native configuration was invalid"));
                    }
                }
            } else {
                status.send_replace(ProcessStatus {
                    state: ProcessState::Stopped,
                    restart_count: 0,
                    updated_at_ms: now_ms(),
                });
            }
        }
        let applied = if !desired.enabled {
            true
        } else if let Some(child) = &self.child {
            let child_status = child.status();
            let pid = child_status.pid();
            status.send_replace(child_status);
            if pid.is_some() && pid != self.verified_pid {
                if native_ready(&core.client, NATIVE_BASE, &desired)
                    .await
                    .is_ok()
                    && child.status().pid() == pid
                {
                    self.verified_pid = pid;
                } else {
                    self.verified_pid = None;
                }
            }
            pid.is_some() && pid == self.verified_pid
        } else {
            false
        };
        if self.reported != Some(applied) {
            match core.report(desired.generation, applied).await {
                Ok(true) => self.reported = Some(applied),
                Ok(false) => {
                    self.stop().await;
                    self.generation = None;
                    status.send_replace(unavailable("Core assignment authorization was revoked"));
                }
                Err(_) => {}
            }
        }
    }
}

fn invalid(what: &str) -> AgentdError {
    AgentdError::Config(format!("Hosted Hermes requires valid {what}"))
}
fn unavailable(error: &str) -> ProcessStatus {
    ProcessStatus {
        state: ProcessState::Unavailable {
            error: error.into(),
        },
        restart_count: 0,
        updated_at_ms: now_ms(),
    }
}

pub(super) fn start(home: PathBuf) -> HostedHermesHandle {
    let (stop, mut stop_rx) = mpsc::channel(1);
    let (status_tx, status) = watch::channel(ProcessStatus::default());
    let task = tokio::spawn(async move {
        let core = match CoreConnection::from_env() {
            Ok(core) => core,
            Err(_) => {
                status_tx.send_replace(unavailable("Core assignment configuration was invalid"));
                stop_rx.recv().await;
                return;
            }
        };
        let mut applied = Applied::default();
        loop {
            tokio::select! {
                _ = applied.reconcile(&core, &home, &status_tx) => {},
                _ = stop_rx.recv() => break,
            }
            if let Some(child) = &applied.child {
                status_tx.send_replace(child.status());
            }
            tokio::select! {
                _ = tokio::time::sleep(POLL_INTERVAL) => {},
                _ = stop_rx.recv() => break,
            }
        }
        applied.stop().await;
        status_tx.send_replace(ProcessStatus {
            state: ProcessState::Stopped,
            restart_count: 0,
            updated_at_ms: now_ms(),
        });
    });
    HostedHermesHandle {
        stop,
        status,
        task: Arc::new(Mutex::new(Some(task))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    struct Request {
        head: String,
        body: String,
    }

    // Real loopback HTTP tests qualify our client/schema/lifecycle contract.
    // These fixture responses do not claim native Hermes or real Core proof.
    async fn server(
        count: usize,
        mut respond: impl FnMut(usize, &Request) -> (u16, String, String) + Send + 'static,
    ) -> (String, tokio::task::JoinHandle<Vec<Request>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for index in 0..count {
                let (mut socket, _) =
                    tokio::time::timeout(Duration::from_secs(5), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut bytes = Vec::new();
                let end = loop {
                    let mut chunk = [0; 2048];
                    let size = socket.read(&mut chunk).await.unwrap();
                    assert!(size > 0);
                    bytes.extend_from_slice(&chunk[..size]);
                    assert!(bytes.len() < 64 * 1024);
                    if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < end + length {
                    let mut chunk = [0; 2048];
                    let size = socket.read(&mut chunk).await.unwrap();
                    assert!(size > 0);
                    bytes.extend_from_slice(&chunk[..size]);
                }
                let request = Request {
                    head,
                    body: String::from_utf8(bytes[end..end + length].to_vec()).unwrap(),
                };
                let (status, body, headers) = respond(index, &request);
                socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}", body.len()).as_bytes()).await.unwrap();
                requests.push(request);
            }
            requests
        });
        (origin, task)
    }

    fn disabled() -> String {
        serde_json::json!({"runtimeId":"runtime_test","generation":2,"enabled":false,"publicUrl":null,"username":null,"password":null,"signingSecret":null,"accessTtlSeconds":60}).to_string()
    }

    fn desired() -> Desired {
        serde_json::from_value(serde_json::json!({"runtimeId":"runtime_test","generation":2,"enabled":true,"publicUrl":"https://agents.example.test/runtimes/runtime_test/","username":"test-user","password":"test-password","signingSecret":"test-signing-secret","accessTtlSeconds":60})).unwrap()
    }

    #[tokio::test]
    async fn pull_and_report_bind_bearer_and_reject_oversized_or_revoked_responses() {
        let credential = "a".repeat(64);
        let expected = credential.clone();
        let (origin, requests) = server(5, move |index, request| {
            assert!(
                request
                    .head
                    .contains(&format!("authorization: Bearer {expected}\r\n"))
            );
            match index {
                0 => (200, disabled(), String::new()),
                1 => {
                    assert!(
                        request
                            .head
                            .starts_with("POST /api/core/v1/runtime/hosted-hermes/report ")
                    );
                    assert_eq!(
                        serde_json::from_str::<serde_json::Value>(&request.body).unwrap(),
                        serde_json::json!({"generation":2,"status":"applied"})
                    );
                    (204, String::new(), String::new())
                }
                2 => (401, "{}".into(), String::new()),
                3 => (403, "{}".into(), String::new()),
                _ => (200, " ".repeat(33 * 1024), String::new()),
            }
        })
        .await;
        let core = CoreConnection::new(origin, credential).unwrap();
        assert!(matches!(
            core.pull().await.unwrap(),
            Pull::Desired(Desired {
                enabled: false,
                generation: 2,
                ..
            })
        ));
        assert!(core.report(2, true).await.unwrap());
        assert!(matches!(core.pull().await.unwrap(), Pull::Revoked));
        assert!(!core.report(2, false).await.unwrap());
        assert!(core.pull().await.is_err());
        assert_eq!(requests.await.unwrap().len(), 5);
    }

    #[test]
    fn readiness_rejects_wrong_principal_and_lifetime() {
        let identity = |user: &str, expiry: u64| serde_json::json!({"provider":"basic","user_id":user,"expires_at":expiry});
        assert!(valid_native_identity(
            &identity("owner", 1060),
            Some("owner"),
            1000
        ));
        assert!(!valid_native_identity(
            &identity("other", 1060),
            Some("owner"),
            1000
        ));
        for expiry in [999, 1000, 1061, 4600] {
            assert!(!valid_native_identity(
                &identity("owner", expiry),
                Some("owner"),
                1000
            ));
        }
    }

    #[tokio::test]
    async fn readiness_requires_anonymous_rejection_then_native_login_and_protected_identity() {
        let (origin, requests) = server(3, |index, request| match index {
            0 => {
                assert!(request.head.starts_with("GET /api/auth/me "));
                (401, "{}".into(), String::new())
            }
            1 => {
                assert!(request.head.starts_with("POST /auth/password-login "));
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&request.body).unwrap()["password"],
                    "test-password"
                );
                (
                    200,
                    r#"{"ok":true}"#.into(),
                    "Set-Cookie: hermes_session_at=test-native-session; HttpOnly; Path=/\r\n"
                        .into(),
                )
            }
            _ => {
                assert!(request.head.starts_with("GET /api/auth/me "));
                assert!(
                    request
                        .head
                        .contains("authorization: Bearer test-native-session\r\n")
                );
                assert!(!request.head.contains("test-password"));
                (200, serde_json::json!({"provider":"basic","user_id":"test-user","expires_at":now_ms()/1000+60}).to_string(), String::new())
            }
        })
        .await;
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        native_ready(&client, &origin, &desired()).await.unwrap();
        assert_eq!(requests.await.unwrap().len(), 3);
        let (ungated, requests) = server(1, |_, _| {
            (200, r#"{"provider":"basic"}"#.into(), String::new())
        })
        .await;
        assert!(native_ready(&client, &ungated, &desired()).await.is_err());
        assert_eq!(requests.await.unwrap().len(), 1);
    }

    async fn child() -> (HostedHermesHandle, u32) {
        let child = HostedHermesHandle::start_spec(crate::supervisor::ProcessSpec {
            name: "hermes-serve",
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "exec sleep 60".into()],
            environment: BTreeMap::new(),
        });
        let pid = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(pid) = child.status().pid() {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        (child, pid)
    }

    #[tokio::test]
    async fn transient_core_failure_retains_child_but_revocation_observes_its_exit() {
        let (child, pid) = child().await;
        let mut applied = Applied {
            runtime_id: Some("runtime_test".into()),
            generation: Some(1),
            child: Some(child),
            ..Applied::default()
        };
        let (origin, requests) = server(2, |index, _| {
            (
                if index == 0 { 503 } else { 401 },
                "{}".into(),
                String::new(),
            )
        })
        .await;
        let core = CoreConnection::new(origin, "a".repeat(64)).unwrap();
        let (status, _) = watch::channel(ProcessStatus::default());
        applied
            .reconcile(&core, Path::new("/synthetic"), &status)
            .await;
        assert_eq!(applied.child.as_ref().unwrap().status().pid(), Some(pid));
        applied
            .reconcile(&core, Path::new("/synthetic"), &status)
            .await;
        assert!(applied.child.is_none());
        assert!(
            rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid as i32).unwrap())
                .is_err()
        );
        requests.await.unwrap();
    }

    #[tokio::test]
    async fn disabled_applied_report_is_sent_only_after_child_exit() {
        let (child, pid) = child().await;
        let mut applied = Applied {
            runtime_id: Some("runtime_test".into()),
            generation: Some(1),
            child: Some(child),
            ..Applied::default()
        };
        let (origin, requests) = server(2, move |index, request| {
            if index == 0 {
                return (200, disabled(), String::new());
            }
            assert!(request.head.starts_with("POST "));
            assert!(
                rustix::process::test_kill_process(
                    rustix::process::Pid::from_raw(pid as i32).unwrap()
                )
                .is_err(),
                "reported applied before child exit"
            );
            (204, String::new(), String::new())
        })
        .await;
        let core = CoreConnection::new(origin, "a".repeat(64)).unwrap();
        let (status, _) = watch::channel(ProcessStatus::default());
        applied
            .reconcile(&core, Path::new("/synthetic"), &status)
            .await;
        assert_eq!(applied.reported, Some(true));
        assert!(matches!(status.borrow().state, ProcessState::Stopped));
        requests.await.unwrap();
    }

    #[test]
    fn native_spec_keeps_secrets_out_of_argv_and_debug() {
        let config = desired();
        let spec = config.spec(Path::new("/data/agent/hermes-home")).unwrap();
        let debug = format!("{spec:?}");
        for value in [config.password.unwrap(), config.signing_secret.unwrap()] {
            assert!(!debug.contains(&value));
            assert!(spec.args.iter().all(|arg| !arg.contains(&value)));
        }
    }
}
