//! `agent.payload.*`: typed control of the finite-shell payload-generations
//! mechanism, forwarded over the shell's local control socket
//! (`/data/shell/shell.sock`, line-delimited JSON).
//!
//! The daemon never stages, verifies, or flips anything itself — the shell
//! owns all of that. This module translates typed platform commands into the
//! shell's socket verbs, resolves the channel form through a fresh verified
//! service directory (exactly like `agent.skills.sync` does), and surfaces
//! the shell's responses as command results. `flip`/`rollback` results are
//! the shell's immediate `flip_started`/`rollback_started` acknowledgements:
//! agentd dies mid-flip, so the outcome is only observable through
//! `agent.payload.status` after the new generation's agentd is up.

use std::path::{Path, PathBuf};

use finitechat_proto::{
    DeviceRef, RuntimeCommandJsonPayloadV1, RuntimeCommandPayloadKindV1, RuntimeCommandRequestV1,
    RuntimeCommandTargetV1, RuntimeCommandTerminalStatusV1,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::AgentdError;
use crate::daemon::{DaemonConfig, failure_result, load_agent_identity, success_result};
use crate::directory::{SERVICE_DIRECTORY_URL_ENV, fetch_verified_directory};
use crate::ledger::{CommandDecision, Ledger};

pub(crate) const SHELL_SOCKET_ENV: &str = "FINITE_SHELL_SOCKET";
pub(crate) const DEFAULT_SHELL_SOCKET: &str = "/data/shell/shell.sock";

pub(crate) const PAYLOAD_STAGE_SCHEMA: &str = "finite.agent.payload.stage.v1";
pub(crate) const PAYLOAD_SET_CHANNEL_SCHEMA: &str = "finite.agent.payload.set-channel.v1";

/// `agent.payload.stage` payload. Two mutually exclusive forms, mirroring
/// `agent.skills.sync`: the explicit form pins the exact bundle, the channel
/// form resolves the `payload_bundle` head through a fresh fetch of the
/// signed service directory and then hands the shell explicit URLs.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PayloadStageRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tarball_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tarball_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// Operator override: re-stage a bad-listed version.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub force: bool,
}

/// `agent.payload.set-channel` payload.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PayloadSetChannelRequest {
    pub channel: String,
}

#[derive(Debug, Clone)]
struct ExplicitPayloadRef {
    tarball_url: String,
    manifest_url: String,
    tarball_sha256: String,
}

enum PayloadStageSource {
    Explicit(ExplicitPayloadRef),
    Channel(String),
}

impl PayloadStageRequest {
    fn source(&self) -> Result<PayloadStageSource, AgentdError> {
        let explicit = [&self.tarball_url, &self.manifest_url, &self.tarball_sha256];
        match (&self.channel, explicit.iter().any(|field| field.is_some())) {
            (Some(_), true) => Err(AgentdError::InvalidPayload(
                "channel and explicit bundle fields are mutually exclusive".to_owned(),
            )),
            (Some(channel), false) => {
                let channel = channel.trim();
                if channel.is_empty() {
                    return Err(AgentdError::InvalidPayload(
                        "channel must not be empty".to_owned(),
                    ));
                }
                Ok(PayloadStageSource::Channel(channel.to_owned()))
            }
            (None, _) => match (&self.tarball_url, &self.manifest_url, &self.tarball_sha256) {
                (Some(tarball_url), Some(manifest_url), Some(tarball_sha256)) => {
                    Ok(PayloadStageSource::Explicit(ExplicitPayloadRef {
                        tarball_url: tarball_url.clone(),
                        manifest_url: manifest_url.clone(),
                        tarball_sha256: tarball_sha256.clone(),
                    }))
                }
                _ => Err(AgentdError::InvalidPayload(
                    "either channel, or tarballUrl + manifestUrl + tarballSha256, is required"
                        .to_owned(),
                )),
            },
        }
    }
}

/// Everything the payload command family needs, split from [`DaemonConfig`]
/// so tests can point it at a fake shell socket and directory server.
#[derive(Debug, Clone)]
pub(crate) struct PayloadSettings {
    pub shell_socket: PathBuf,
    /// Hex-encoded release public key; required for the channel form (the
    /// directory document must verify before its channel heads are trusted).
    pub release_public_key: Option<String>,
    pub allow_insecure_url: bool,
    pub service_directory_url: Option<String>,
}

impl PayloadSettings {
    pub(crate) fn from_daemon_config(config: &DaemonConfig) -> Self {
        Self {
            shell_socket: config.shell_socket.clone(),
            release_public_key: config.release_public_key.clone(),
            allow_insecure_url: config.allow_insecure_bundle_url,
            service_directory_url: config.service_directory_url.clone(),
        }
    }
}

/// One request/response roundtrip over the shell's control socket. Shell
/// errors (`{"ok": false, "error": ...}`) become typed command failures.
async fn shell_roundtrip(socket: &Path, request: &Value) -> Result<Value, AgentdError> {
    let stream = UnixStream::connect(socket).await.map_err(|error| {
        AgentdError::ShellUnavailable(format!(
            "shell control socket {} is unavailable: {error}",
            socket.display()
        ))
    })?;
    let (read_half, mut write_half) = stream.into_split();
    let mut line = request.to_string();
    line.push('\n');
    write_half
        .write_all(line.as_bytes())
        .await
        .map_err(|error| AgentdError::ShellUnavailable(format!("shell socket write: {error}")))?;
    let mut reader = BufReader::new(read_half).lines();
    let response = reader
        .next_line()
        .await
        .map_err(|error| AgentdError::ShellUnavailable(format!("shell socket read: {error}")))?
        .ok_or_else(|| {
            AgentdError::ShellUnavailable("the shell closed the control socket".to_owned())
        })?;
    let response: Value = serde_json::from_str(response.trim()).map_err(|error| {
        AgentdError::ShellUnavailable(format!("shell response is not JSON: {error}"))
    })?;
    if response.get("ok") == Some(&Value::Bool(true)) {
        return Ok(response);
    }
    let code = response
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let message = response
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("the shell rejected the request")
        .to_owned();
    Err(AgentdError::ShellRejected { code, message })
}

/// The shell's response without the socket-protocol `ok` marker — the typed
/// command result body.
fn response_body(mut response: Value) -> Value {
    if let Some(object) = response.as_object_mut() {
        object.remove("ok");
    }
    response
}

/// Resolve the channel's `payload_bundle` head through a fresh
/// fetch-and-verify of the service directory (never the cache: staging acts
/// on channel heads).
async fn resolve_channel_payload(
    settings: &PayloadSettings,
    channel: &str,
) -> Result<ExplicitPayloadRef, AgentdError> {
    let public_key_hex = settings
        .release_public_key
        .as_deref()
        .ok_or(AgentdError::MissingReleaseKey)?;
    let directory_url = settings.service_directory_url.as_deref().ok_or_else(|| {
        AgentdError::Config(format!(
            "{SERVICE_DIRECTORY_URL_ENV} is not configured; channel-based payload staging is unavailable"
        ))
    })?;
    let directory =
        fetch_verified_directory(directory_url, public_key_hex, settings.allow_insecure_url)
            .await?;
    let bundle = directory
        .channel_bundle(channel, finite_service_directory::PAYLOAD_BUNDLE_KIND)
        .ok_or_else(|| AgentdError::PayloadChannelHeadMissing(channel.to_owned()))?;
    Ok(ExplicitPayloadRef {
        tarball_url: bundle.tarball_url.clone(),
        manifest_url: bundle.manifest_url.clone(),
        tarball_sha256: bundle.tarball_sha256.clone(),
    })
}

pub(crate) async fn payload_status(settings: &PayloadSettings) -> Result<Value, AgentdError> {
    let response = shell_roundtrip(&settings.shell_socket, &json!({ "verb": "status" })).await?;
    Ok(response_body(response))
}

pub(crate) async fn payload_stage(
    settings: &PayloadSettings,
    request: &PayloadStageRequest,
) -> Result<Value, AgentdError> {
    let (bundle, channel) = match request.source()? {
        PayloadStageSource::Explicit(explicit) => (explicit, None),
        PayloadStageSource::Channel(channel) => (
            resolve_channel_payload(settings, &channel).await?,
            Some(channel),
        ),
    };
    let mut stage = json!({
        "verb": "stage",
        "tarballUrl": bundle.tarball_url,
        "manifestUrl": bundle.manifest_url,
        "tarballSha256": bundle.tarball_sha256,
    });
    if request.force {
        stage["force"] = json!(true);
    }
    let response = shell_roundtrip(&settings.shell_socket, &stage).await?;
    let mut body = response_body(response);
    if let Some(channel) = channel {
        // Proof the bundle was resolved through the service directory, not
        // named by the caller.
        body["channel"] = json!(channel);
    }
    Ok(body)
}

pub(crate) async fn payload_flip(settings: &PayloadSettings) -> Result<Value, AgentdError> {
    let response = shell_roundtrip(&settings.shell_socket, &json!({ "verb": "flip" })).await?;
    Ok(response_body(response))
}

pub(crate) async fn payload_rollback(settings: &PayloadSettings) -> Result<Value, AgentdError> {
    let response = shell_roundtrip(&settings.shell_socket, &json!({ "verb": "rollback" })).await?;
    Ok(response_body(response))
}

pub(crate) async fn payload_set_channel(
    settings: &PayloadSettings,
    request: &PayloadSetChannelRequest,
) -> Result<Value, AgentdError> {
    let response = shell_roundtrip(
        &settings.shell_socket,
        &json!({ "verb": "set-channel", "channel": request.channel }),
    )
    .await?;
    Ok(response_body(response))
}

/// One-shot operator/test entry for `payload-status`: a plain read-only
/// forward to the shell, no ledger involvement.
pub async fn run_payload_status_cli(config: &DaemonConfig) -> Result<Value, AgentdError> {
    payload_status(&PayloadSettings::from_daemon_config(config)).await
}

/// One-shot operator/test entry for `payload-stage`: run exactly the daemon's
/// `agent.payload.stage` command path — the same settings-from-env
/// resolution, channel lookup, shell forwarding, and durable ledger recording
/// — without a chat-channel sender (mirrors `run_skills_sync_cli`).
pub async fn run_payload_stage_cli(
    config: &DaemonConfig,
    request_id: &str,
    request: PayloadStageRequest,
) -> Result<Value, AgentdError> {
    let settings = PayloadSettings::from_daemon_config(config);
    std::fs::create_dir_all(config.state_dir())?;
    let ledger = Ledger::open(config.state_dir().join("agentd.sqlite3"))?;
    let identity = load_agent_identity(&config.agent_home)
        .unwrap_or_else(|_| DeviceRef::new("local-operator", "agentd-cli"));
    let command_request = RuntimeCommandRequestV1 {
        payload_kind: RuntimeCommandPayloadKindV1::Request,
        request_id: request_id.to_owned(),
        command: "agent.payload.stage".to_owned(),
        target: RuntimeCommandTargetV1 {
            account_id: identity.account_id.clone(),
            device_id: Some(identity.device_id.clone()),
        },
        resource_key: None,
        body: RuntimeCommandJsonPayloadV1 {
            schema: PAYLOAD_STAGE_SCHEMA.to_owned(),
            json_payload: serde_json::to_vec(&request)?,
        },
    };

    match ledger.begin_command(&command_request)? {
        CommandDecision::Replay(result) => replayed_result_value(result),
        CommandDecision::Execute | CommandDecision::Resume => {
            match payload_stage(&settings, &request).await {
                Ok(body) => {
                    let result = success_result(&command_request, body.clone())?;
                    ledger.finish_command(request_id, &result)?;
                    Ok(body)
                }
                Err(error) => {
                    let result = failure_result(&command_request, &error);
                    ledger.finish_command(request_id, &result)?;
                    Err(error)
                }
            }
        }
    }
}

fn replayed_result_value(
    result: finitechat_proto::RuntimeCommandResultV1,
) -> Result<Value, AgentdError> {
    if result.status == RuntimeCommandTerminalStatusV1::Succeeded {
        let body = result
            .body
            .map(|body| serde_json::from_slice::<Value>(&body.json_payload))
            .transpose()?
            .unwrap_or(Value::Null);
        return Ok(body);
    }
    let message = result
        .error
        .map(|error| error.message)
        .unwrap_or_else(|| "the recorded command failed".to_owned());
    Err(AgentdError::ShellRejected {
        code: "ledger_replay".to_owned(),
        message: format!("replayed from the ledger: {message}"),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::io::AsyncReadExt;
    use tokio::net::UnixListener;

    use super::*;

    /// A fake shell control socket: records each received request line and
    /// answers with the canned response for that request index.
    fn fake_shell_socket(dir: &Path, responses: Vec<Value>) -> (PathBuf, Arc<Mutex<Vec<Value>>>) {
        let socket_path = dir.join("shell.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&received);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let responses = responses.clone();
                let seen = Arc::clone(&seen);
                tokio::spawn(async move {
                    let (read_half, mut write_half) = stream.into_split();
                    let mut lines = BufReader::new(read_half).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let request: Value = serde_json::from_str(line.trim()).unwrap();
                        let index = {
                            let mut seen = seen.lock().unwrap();
                            seen.push(request);
                            seen.len() - 1
                        };
                        let response = responses
                            .get(index)
                            .cloned()
                            .unwrap_or_else(|| json!({ "ok": true }));
                        let mut bytes = response.to_string().into_bytes();
                        bytes.push(b'\n');
                        if write_half.write_all(&bytes).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        (socket_path, received)
    }

    fn settings(socket: &Path) -> PayloadSettings {
        PayloadSettings {
            shell_socket: socket.to_path_buf(),
            release_public_key: Some(hex::encode(test_signing_key().verifying_key().to_bytes())),
            allow_insecure_url: true,
            service_directory_url: None,
        }
    }

    fn test_signing_key() -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[7_u8; 32])
    }

    #[tokio::test]
    async fn status_flip_rollback_and_set_channel_forward_their_shell_verbs() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, received) = fake_shell_socket(
            dir.path(),
            vec![
                json!({ "ok": true, "shellVersion": "0.1.0", "current": "seed-1" }),
                json!({ "ok": true, "result": "flip_started", "from": "seed-1", "to": "v2" }),
                json!({ "ok": true, "result": "rollback_started", "from": "v2", "to": "seed-1" }),
                json!({ "ok": true, "channel": "canary" }),
            ],
        );
        let settings = settings(&socket);

        let status = payload_status(&settings).await.unwrap();
        assert_eq!(status["shellVersion"], json!("0.1.0"));
        assert!(status.get("ok").is_none(), "socket framing must not leak");

        let flip = payload_flip(&settings).await.unwrap();
        assert_eq!(flip["result"], json!("flip_started"));
        let rollback = payload_rollback(&settings).await.unwrap();
        assert_eq!(rollback["result"], json!("rollback_started"));
        let set = payload_set_channel(
            &settings,
            &PayloadSetChannelRequest {
                channel: "canary".to_owned(),
            },
        )
        .await
        .unwrap();
        assert_eq!(set["channel"], json!("canary"));

        assert_eq!(
            *received.lock().unwrap(),
            vec![
                json!({ "verb": "status" }),
                json!({ "verb": "flip" }),
                json!({ "verb": "rollback" }),
                json!({ "verb": "set-channel", "channel": "canary" }),
            ]
        );
    }

    #[tokio::test]
    async fn explicit_stage_forwards_the_exact_camel_case_fields_and_force() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, received) = fake_shell_socket(
            dir.path(),
            vec![json!({
                "ok": true,
                "result": "staged",
                "versionLabel": "v2",
                "artifactId": "finite-agent-payload-v2",
            })],
        );

        let result = payload_stage(
            &settings(&socket),
            &PayloadStageRequest {
                tarball_url: Some("https://release.invalid/p.tar.gz".to_owned()),
                manifest_url: Some("https://release.invalid/p.tar.gz.manifest.json".to_owned()),
                tarball_sha256: Some("a".repeat(64)),
                channel: None,
                force: true,
            },
        )
        .await
        .unwrap();

        assert_eq!(result["result"], json!("staged"));
        assert_eq!(result["versionLabel"], json!("v2"));
        assert!(result.get("channel").is_none());
        assert_eq!(
            *received.lock().unwrap(),
            vec![json!({
                "verb": "stage",
                "tarballUrl": "https://release.invalid/p.tar.gz",
                "manifestUrl": "https://release.invalid/p.tar.gz.manifest.json",
                "tarballSha256": "a".repeat(64),
                "force": true,
            })]
        );
    }

    #[tokio::test]
    async fn a_shell_error_response_surfaces_its_code_and_message() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, _received) = fake_shell_socket(
            dir.path(),
            vec![json!({
                "ok": false,
                "error": { "code": "nothing_staged", "message": "no staged generation" },
            })],
        );

        let error = payload_flip(&settings(&socket)).await.unwrap_err();
        assert_eq!(error.public_code(), "payload_shell_rejected");
        assert!(error.public_message().contains("nothing_staged"));
        assert!(error.public_message().contains("no staged generation"));
    }

    #[tokio::test]
    async fn a_missing_shell_socket_is_a_distinct_unavailable_error() {
        let dir = tempfile::tempdir().unwrap();
        let settings = settings(&dir.path().join("absent.sock"));
        let error = payload_status(&settings).await.unwrap_err();
        assert_eq!(error.public_code(), "shell_unavailable");
    }

    #[tokio::test]
    async fn channel_and_explicit_fields_are_mutually_exclusive_and_one_is_required() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, received) = fake_shell_socket(dir.path(), Vec::new());
        let settings = settings(&socket);

        let mut both = PayloadStageRequest {
            tarball_url: Some("https://release.invalid/p.tar.gz".to_owned()),
            manifest_url: Some("https://release.invalid/p.tar.gz.manifest.json".to_owned()),
            tarball_sha256: Some("a".repeat(64)),
            channel: None,
            force: false,
        };
        both.channel = Some("canary".to_owned());
        let error = payload_stage(&settings, &both).await.unwrap_err();
        assert!(error.public_message().contains("mutually exclusive"));

        let neither = PayloadStageRequest {
            tarball_url: None,
            manifest_url: None,
            tarball_sha256: None,
            channel: None,
            force: false,
        };
        let error = payload_stage(&settings, &neither).await.unwrap_err();
        assert!(matches!(error, AgentdError::InvalidPayload(_)));
        assert!(
            received.lock().unwrap().is_empty(),
            "invalid requests must never reach the shell"
        );
    }

    // ------------------------------------------------------------------
    // Channel resolution through the signed service directory (the same
    // fixture shape as the M2 skills-channel tests).
    // ------------------------------------------------------------------

    async fn serve_bytes(routes: Vec<(String, Vec<u8>)>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let routes = routes.clone();
                tokio::spawn(async move {
                    let mut buffer = vec![0_u8; 8192];
                    let read = stream.read(&mut buffer).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                    let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    match routes.iter().find(|(route, _)| *route == path) {
                        Some((_, body)) => {
                            let header = format!(
                                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                                body.len()
                            );
                            let _ = stream.write_all(header.as_bytes()).await;
                            let _ = stream.write_all(body).await;
                        }
                        None => {
                            let _ = stream
                                .write_all(
                                    b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                                )
                                .await;
                        }
                    }
                    let _ = stream.shutdown().await;
                });
            }
        });
        format!("http://{addr}")
    }

    /// A signed `finite_service_directory.v1` document advertising a payload
    /// bundle as the canary `payload_bundle` head (or no head at all).
    fn signed_directory_bytes(payload_head: Option<(&str, &str)>) -> Vec<u8> {
        use std::collections::BTreeMap;

        use finite_service_directory::{
            ChannelBundleV1, SERVICE_DIRECTORY_SCHEMA, ServiceDirectoryV1, ServiceEntryV1,
        };

        let mut channels = BTreeMap::new();
        if let Some((tarball_url, tarball_sha256)) = payload_head {
            channels.insert(
                "canary".to_owned(),
                BTreeMap::from([(
                    finite_service_directory::PAYLOAD_BUNDLE_KIND.to_owned(),
                    ChannelBundleV1 {
                        artifact_id: "finite-agent-payload-v2".to_owned(),
                        version_label: "v2".to_owned(),
                        manifest_url: format!("{tarball_url}.manifest.json"),
                        tarball_url: tarball_url.to_owned(),
                        tarball_sha256: tarball_sha256.to_owned(),
                    },
                )]),
            );
        }
        let mut directory = ServiceDirectoryV1 {
            schema: SERVICE_DIRECTORY_SCHEMA.to_owned(),
            generated_at: "2026-08-06T00:00:00Z".to_owned(),
            services: BTreeMap::from([(
                "core".to_owned(),
                ServiceEntryV1 {
                    base_url: "http://192.168.64.1:18789".to_owned(),
                    client_version: None,
                },
            )]),
            channels,
            signature: String::new(),
        };
        directory.sign(&test_signing_key()).unwrap();
        serde_json::to_vec(&directory).unwrap()
    }

    fn channel_request(channel: &str) -> PayloadStageRequest {
        PayloadStageRequest {
            tarball_url: None,
            manifest_url: None,
            tarball_sha256: None,
            channel: Some(channel.to_owned()),
            force: false,
        }
    }

    #[tokio::test]
    async fn the_channel_form_resolves_the_payload_head_and_sends_explicit_urls_to_the_shell() {
        let dir = tempfile::tempdir().unwrap();
        let tarball_url = "http://release.local/finite-agent-payload-v2.tar.gz";
        let tarball_sha256 = "b".repeat(64);
        let directory_base = serve_bytes(vec![(
            "/api/core/v1/service-directory".to_owned(),
            signed_directory_bytes(Some((tarball_url, &tarball_sha256))),
        )])
        .await;
        let (socket, received) = fake_shell_socket(
            dir.path(),
            vec![json!({ "ok": true, "result": "staged", "versionLabel": "v2" })],
        );
        let mut settings = settings(&socket);
        settings.service_directory_url =
            Some(format!("{directory_base}/api/core/v1/service-directory"));

        let result = payload_stage(&settings, &channel_request("canary"))
            .await
            .unwrap();

        assert_eq!(result["result"], json!("staged"));
        assert_eq!(
            result["channel"],
            json!("canary"),
            "the result must prove the bundle was resolved through the directory"
        );
        assert_eq!(
            *received.lock().unwrap(),
            vec![json!({
                "verb": "stage",
                "tarballUrl": tarball_url,
                "manifestUrl": format!("{tarball_url}.manifest.json"),
                "tarballSha256": tarball_sha256,
            })],
            "the shell must receive explicit URLs, never the channel name"
        );
    }

    #[tokio::test]
    async fn a_channel_with_no_payload_bundle_head_is_a_distinct_error() {
        let dir = tempfile::tempdir().unwrap();
        let directory_base = serve_bytes(vec![(
            "/api/core/v1/service-directory".to_owned(),
            signed_directory_bytes(None),
        )])
        .await;
        let (socket, received) = fake_shell_socket(dir.path(), Vec::new());
        let mut settings = settings(&socket);
        settings.service_directory_url =
            Some(format!("{directory_base}/api/core/v1/service-directory"));

        let error = payload_stage(&settings, &channel_request("canary"))
            .await
            .unwrap_err();

        assert!(matches!(error, AgentdError::PayloadChannelHeadMissing(_)));
        assert_eq!(error.public_code(), "payload_channel_head_missing");
        assert!(error.public_message().contains("canary"));
        assert!(received.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_channel_form_requires_a_directory_url_and_a_release_key() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, _received) = fake_shell_socket(dir.path(), Vec::new());
        let no_directory = settings(&socket);
        let error = payload_stage(&no_directory, &channel_request("canary"))
            .await
            .unwrap_err();
        assert!(error.public_message().contains(SERVICE_DIRECTORY_URL_ENV));

        let mut unkeyed = settings(&socket);
        unkeyed.release_public_key = None;
        unkeyed.service_directory_url = Some("http://127.0.0.1:1/directory".to_owned());
        let error = payload_stage(&unkeyed, &channel_request("canary"))
            .await
            .unwrap_err();
        assert!(matches!(error, AgentdError::MissingReleaseKey));
    }

    fn daemon_config(agent_home: &Path, shell_socket: &Path) -> DaemonConfig {
        DaemonConfig {
            agent_home: agent_home.to_path_buf(),
            hermes_home: agent_home.join("hermes-home"),
            bridge_url: "http://127.0.0.1:1".to_owned(),
            bridge_addr: "127.0.0.1:1".to_owned(),
            finitechat_bin: PathBuf::from("/nonexistent"),
            prepare_command: PathBuf::from("/nonexistent"),
            hermes_command: PathBuf::from("/nonexistent"),
            hermes_probe_python: PathBuf::from("python"),
            hermes_probe_script: PathBuf::from("/nonexistent"),
            health_server: None,
            authorized_accounts: std::collections::BTreeSet::new(),
            specialization_bundle: None,
            release_public_key: Some(hex::encode(test_signing_key().verifying_key().to_bytes())),
            allow_insecure_bundle_url: true,
            finite_cli: PathBuf::from("/nonexistent"),
            service_directory_url: None,
            shell_socket: shell_socket.to_path_buf(),
        }
    }

    #[tokio::test]
    async fn the_stage_cli_entry_records_through_the_ledger_and_replays_terminal_results() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, received) = fake_shell_socket(
            dir.path(),
            vec![json!({ "ok": true, "result": "staged", "versionLabel": "v2" })],
        );
        let agent_home = dir.path().join("agent-home");
        let config = daemon_config(&agent_home, &socket);
        let request = || PayloadStageRequest {
            tarball_url: Some("https://release.invalid/p.tar.gz".to_owned()),
            manifest_url: Some("https://release.invalid/p.tar.gz.manifest.json".to_owned()),
            tarball_sha256: Some("a".repeat(64)),
            channel: None,
            force: false,
        };

        let first = run_payload_stage_cli(&config, "cli-payload-1", request())
            .await
            .unwrap();
        assert_eq!(first["result"], json!("staged"));
        assert!(
            agent_home.join("agentd/agentd.sqlite3").is_file(),
            "the CLI must record through the daemon's own ledger file"
        );

        let replayed = run_payload_stage_cli(&config, "cli-payload-1", request())
            .await
            .unwrap();
        assert_eq!(replayed, first);
        assert_eq!(
            received.lock().unwrap().len(),
            1,
            "a replayed request id must not reach the shell again"
        );
    }
}
