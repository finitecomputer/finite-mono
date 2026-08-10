//! `agent.skills.sync`: fetch, verify, and adopt a signed managed-skills
//! bundle.
//!
//! The daemon downloads the bundle referenced by the typed command payload,
//! proves it against the release public key with `finite-release` (tarball
//! sha256, manifest signature, recomputed tree digest), and only then hands
//! the unpacked tree to the in-image `finite skills sync --source` machinery,
//! which keeps its validate/lock/atomic-exchange/rollback guarantees.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use finitechat_proto::{
    DeviceRef, RuntimeCommandJsonPayloadV1, RuntimeCommandPayloadKindV1, RuntimeCommandRequestV1,
    RuntimeCommandTargetV1, RuntimeCommandTerminalStatusV1,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::process::Command as TokioCommand;

use crate::AgentdError;
use crate::daemon::{DaemonConfig, failure_result, load_agent_identity, success_result};
use crate::directory::{
    SERVICE_DIRECTORY_CACHE_FILE, SERVICE_DIRECTORY_URL_ENV, fetch_verified_directory_with_floor,
};
use crate::ledger::{CommandDecision, Ledger};

pub(crate) const SKILLS_SYNC_SCHEMA: &str = "finite.agent.skills.sync.v1";

const MANIFEST_FETCH_CAP_BYTES: usize = 64 * 1024;
const TARBALL_FETCH_CAP_BYTES: usize = 64 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
const CLI_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) const RELEASE_PUBLIC_KEY_ENV: &str = "FINITE_RELEASE_PUBLIC_KEY";
/// One shared URL policy for all fetched release material; the env name and
/// enforcement live in `finite-release`.
pub(crate) const ALLOW_INSECURE_BUNDLE_URL_ENV: &str =
    finite_release::ALLOW_INSECURE_BUNDLE_URL_ENV;
pub(crate) const FINITE_CLI_PATH_ENV: &str = "FINITE_CLI_PATH";
pub(crate) const DEFAULT_FINITE_CLI_PATH: &str = "/runtime/bin/finite";

/// `agent.skills.sync` payload. Two mutually exclusive forms:
/// the explicit form pins the exact bundle (`tarballUrl` + `manifestUrl` +
/// `tarballSha256`); the channel form (`channel`) resolves the bundle through
/// a fresh fetch of the signed service directory.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SkillsSyncRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tarball_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tarball_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ExplicitBundleRef {
    pub tarball_url: String,
    pub manifest_url: String,
    pub tarball_sha256: String,
}

#[derive(Debug)]
enum SkillsSyncSource {
    Explicit(ExplicitBundleRef),
    Channel(String),
}

impl SkillsSyncRequest {
    fn source(&self) -> Result<SkillsSyncSource, AgentdError> {
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
                Ok(SkillsSyncSource::Channel(channel.to_owned()))
            }
            (None, _) => match (&self.tarball_url, &self.manifest_url, &self.tarball_sha256) {
                (Some(tarball_url), Some(manifest_url), Some(tarball_sha256)) => {
                    Ok(SkillsSyncSource::Explicit(ExplicitBundleRef {
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

#[derive(Debug, Clone)]
pub(crate) struct SkillsSyncSettings {
    /// Parent for per-artifact staging directories
    /// (`<agent home>/agentd/staging/skills/<artifact_id>`).
    pub staging_root: PathBuf,
    /// Hex-encoded 32-byte ed25519 release public key; sync refuses without
    /// it.
    pub release_public_key: Option<String>,
    /// Dev-harness-only escape hatch admitting plain-http bundle URLs.
    pub allow_insecure_url: bool,
    /// The in-image `finite` CLI that owns the atomic adoption step.
    pub finite_cli: PathBuf,
    /// Core's signed service directory endpoint; required for the channel
    /// form.
    pub service_directory_url: Option<String>,
    /// The daemon's verified directory cache — the durable anti-replay floor
    /// for channel resolution. `None` disables the floor (tests).
    pub directory_cache: Option<PathBuf>,
}

impl SkillsSyncSettings {
    pub(crate) fn from_daemon_config(config: &DaemonConfig) -> Self {
        Self {
            staging_root: config.state_dir().join("staging").join("skills"),
            release_public_key: config.release_public_key.clone(),
            allow_insecure_url: config.allow_insecure_bundle_url,
            finite_cli: config.finite_cli.clone(),
            service_directory_url: config.service_directory_url.clone(),
            directory_cache: Some(config.agent_home.join(SERVICE_DIRECTORY_CACHE_FILE)),
        }
    }
}

/// Resolve the channel's skills bundle through a fresh fetch-and-verify of
/// the service directory (never a cache read: sync acts on channel heads; the
/// cache only supplies the durable anti-replay floor).
async fn resolve_channel_bundle(
    settings: &SkillsSyncSettings,
    public_key_hex: &str,
    channel: &str,
) -> Result<ExplicitBundleRef, AgentdError> {
    let directory_url = settings.service_directory_url.as_deref().ok_or_else(|| {
        AgentdError::Config(format!(
            "{SERVICE_DIRECTORY_URL_ENV} is not configured; channel-based skills sync is unavailable"
        ))
    })?;
    let directory = fetch_verified_directory_with_floor(
        directory_url,
        public_key_hex,
        settings.allow_insecure_url,
        settings.directory_cache.as_deref(),
    )
    .await?;
    let bundle = directory
        .channel_bundle(channel, finite_service_directory::SKILLS_BUNDLE_KIND)
        .ok_or_else(|| AgentdError::SkillsChannelHeadMissing(channel.to_owned()))?;
    Ok(ExplicitBundleRef {
        tarball_url: bundle.tarball_url.clone(),
        manifest_url: bundle.manifest_url.clone(),
        tarball_sha256: bundle.tarball_sha256.clone(),
    })
}

pub(crate) async fn sync_skills(
    settings: &SkillsSyncSettings,
    request: &SkillsSyncRequest,
) -> Result<Value, AgentdError> {
    let source = request.source()?;
    let public_key_hex = settings
        .release_public_key
        .as_deref()
        .ok_or(AgentdError::MissingReleaseKey)?;
    let public_key = finite_release::parse_verifying_key_hex(public_key_hex).map_err(|error| {
        AgentdError::Config(format!("{RELEASE_PUBLIC_KEY_ENV} is invalid: {error}"))
    })?;
    let (bundle_ref, channel) = match source {
        SkillsSyncSource::Explicit(explicit) => (explicit, None),
        SkillsSyncSource::Channel(channel) => (
            resolve_channel_bundle(settings, public_key_hex, &channel).await?,
            Some(channel),
        ),
    };
    let expected_tarball_sha256 = bundle_ref.tarball_sha256.trim().to_ascii_lowercase();
    if !finite_release::is_lower_hex_256(&expected_tarball_sha256) {
        return Err(AgentdError::InvalidPayload(
            "tarballSha256 must be 64 hex characters".to_owned(),
        ));
    }
    check_url_policy(&bundle_ref.tarball_url, settings.allow_insecure_url)?;
    check_url_policy(&bundle_ref.manifest_url, settings.allow_insecure_url)?;

    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
        .build()?;
    let manifest_bytes = fetch_bounded(
        &client,
        &bundle_ref.manifest_url,
        MANIFEST_FETCH_CAP_BYTES,
        "manifest",
    )
    .await?;
    let manifest = finite_release::SkillsBundleManifestV1::from_json_bytes(&manifest_bytes)
        .map_err(|error| AgentdError::SkillsBundle(error.to_string()))?;
    if manifest.tarball_sha256 != expected_tarball_sha256 {
        return Err(AgentdError::SkillsBundle(format!(
            "the command's tarballSha256 does not match the manifest's {}",
            manifest.tarball_sha256
        )));
    }

    let tarball_bytes = fetch_bounded(
        &client,
        &bundle_ref.tarball_url,
        TARBALL_FETCH_CAP_BYTES,
        "tarball",
    )
    .await?;

    let staging_dir = settings.staging_root.join(&manifest.artifact_id);
    let artifact_id = manifest.artifact_id.clone();
    let manifest_for_verify = manifest.clone();
    let staging_for_verify = staging_dir.clone();
    let staged_tree = tokio::task::spawn_blocking(move || {
        stage_and_verify(
            &staging_for_verify,
            &manifest_for_verify,
            &manifest_bytes,
            &tarball_bytes,
            &public_key,
        )
    })
    .await
    .map_err(|error| AgentdError::SkillsBundle(format!("bundle staging failed: {error}")))??;

    let cli_result = run_finite_cli(&settings.finite_cli, &staged_tree).await;
    match cli_result {
        Ok(cli_output) => {
            if let Err(error) = std::fs::remove_dir_all(&staging_dir) {
                eprintln!(
                    "finite-agentd: synced skills but could not remove staging {}: {error}",
                    staging_dir.display()
                );
            }
            let mut result = json!({
                "applied": true,
                "artifactId": artifact_id,
                "treeDigest": manifest.tree_digest,
            });
            if let Some(channel) = channel {
                // Proof the bundle was resolved through the service
                // directory, not named by the caller.
                result["channel"] = json!(channel);
            }
            match cli_output.get("skillCount").and_then(Value::as_u64) {
                Some(skill_count) => {
                    result["skillCount"] = json!(skill_count);
                }
                None => {
                    result["error"] =
                        json!("the finite CLI applied the bundle but reported no skill count");
                }
            }
            Ok(result)
        }
        // Staging is deliberately kept on failure for diagnosis.
        Err(error) => Err(error),
    }
}

/// One-shot operator/test entry: run exactly the daemon's `agent.skills.sync`
/// command path — the same settings-from-env resolution, verification, apply
/// step, and durable ledger recording — without a chat-channel sender.
///
/// The ledger is opened the same way `run_daemon` opens it (the agentd state
/// dir's `agentd.sqlite3`), so a later daemon delivery reusing the request id
/// replays the recorded result instead of re-executing.
pub async fn run_skills_sync_cli(
    config: &DaemonConfig,
    request_id: &str,
    request: SkillsSyncRequest,
) -> Result<Value, AgentdError> {
    let settings = SkillsSyncSettings::from_daemon_config(config);
    std::fs::create_dir_all(config.state_dir())?;
    let ledger = Ledger::open(config.state_dir().join("agentd.sqlite3"))?;
    // In-guest the agent identity file always exists; the fallback keeps the
    // ledger fingerprint stable for out-of-guest tests and dry runs.
    let identity = load_agent_identity(&config.agent_home)
        .unwrap_or_else(|_| DeviceRef::new("local-operator", "agentd-cli"));
    let command_request = RuntimeCommandRequestV1 {
        payload_kind: RuntimeCommandPayloadKindV1::Request,
        request_id: request_id.to_owned(),
        command: "agent.skills.sync".to_owned(),
        target: RuntimeCommandTargetV1 {
            account_id: identity.account_id.clone(),
            device_id: Some(identity.device_id.clone()),
        },
        resource_key: None,
        body: RuntimeCommandJsonPayloadV1 {
            schema: SKILLS_SYNC_SCHEMA.to_owned(),
            // The request serializes exactly as the daemon's typed payload
            // (camelCase, absent fields omitted), so ledger fingerprints stay
            // stable between the CLI and chat-channel entries.
            json_payload: serde_json::to_vec(&request)?,
        },
    };

    match ledger.begin_command(&command_request)? {
        CommandDecision::Replay(result) => replayed_result_value(result),
        CommandDecision::Execute | CommandDecision::Resume => {
            match sync_skills(&settings, &request).await {
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
    Err(AgentdError::SkillsBundle(format!(
        "replayed from the ledger: {message}"
    )))
}

fn check_url_policy(url: &str, allow_insecure: bool) -> Result<(), AgentdError> {
    finite_release::check_bundle_url_policy(url, allow_insecure)
        .map_err(|error| AgentdError::InvalidPayload(error.to_string()))
}

async fn fetch_bounded(
    client: &reqwest::Client,
    url: &str,
    cap: usize,
    what: &str,
) -> Result<Vec<u8>, AgentdError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| AgentdError::Transport(format!("{what} fetch failed: {error}")))?;
    if !response.status().is_success() {
        return Err(AgentdError::Transport(format!(
            "{what} fetch failed with {}",
            response.status()
        )));
    }
    if let Some(length) = response.content_length()
        && length > cap as u64
    {
        return Err(AgentdError::SkillsBundle(format!(
            "{what} exceeds the {cap}-byte cap"
        )));
    }
    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|error| AgentdError::Transport(format!("{what} fetch failed: {error}")))?;
        if bytes.len() + chunk.len() > cap {
            return Err(AgentdError::SkillsBundle(format!(
                "{what} exceeds the {cap}-byte cap"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Write the fetched bytes into a fresh staging directory and verify them end
/// to end. Returns the unpacked tree ready for `finite skills sync --source`.
fn stage_and_verify(
    staging_dir: &Path,
    manifest: &finite_release::SkillsBundleManifestV1,
    manifest_bytes: &[u8],
    tarball_bytes: &[u8],
    public_key: &ed25519_dalek::VerifyingKey,
) -> Result<PathBuf, AgentdError> {
    if staging_dir.exists() {
        std::fs::remove_dir_all(staging_dir)?;
    }
    std::fs::create_dir_all(staging_dir)?;
    let tarball_path = staging_dir.join(format!("{}.tar.gz", manifest.artifact_id));
    let manifest_path = staging_dir.join(format!("{}.tar.gz.manifest.json", manifest.artifact_id));
    std::fs::write(&tarball_path, tarball_bytes)?;
    std::fs::write(&manifest_path, manifest_bytes)?;
    let tree = staging_dir.join("tree");
    finite_release::verify_skills_bundle(
        &tarball_path,
        &manifest_path,
        public_key,
        Some(&manifest.tarball_sha256),
        &tree,
    )
    .map_err(|error| AgentdError::SkillsBundle(error.to_string()))?;
    Ok(tree)
}

async fn run_finite_cli(finite_cli: &Path, source: &Path) -> Result<Value, AgentdError> {
    let output = tokio::time::timeout(
        CLI_TIMEOUT,
        TokioCommand::new(finite_cli)
            .arg("skills")
            .arg("sync")
            .arg("--source")
            .arg(source)
            .arg("--json")
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| AgentdError::SkillsBundle("finite skills sync timed out".to_owned()))??;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim().lines().last().unwrap_or_default().to_owned();
        return Err(AgentdError::SkillsBundle(format!(
            "finite skills sync failed ({}): {detail}",
            output.status
        )));
    }
    let parsed = stdout
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line.trim()).ok());
    Ok(parsed.unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn settings(staging_root: &Path, finite_cli: &Path) -> SkillsSyncSettings {
        SkillsSyncSettings {
            staging_root: staging_root.to_path_buf(),
            release_public_key: Some(hex::encode(test_signing_key().verifying_key().to_bytes())),
            allow_insecure_url: true,
            finite_cli: finite_cli.to_path_buf(),
            service_directory_url: None,
            directory_cache: None,
        }
    }

    fn explicit_request(
        tarball_url: impl Into<String>,
        manifest_url: impl Into<String>,
        tarball_sha256: impl Into<String>,
    ) -> SkillsSyncRequest {
        SkillsSyncRequest {
            tarball_url: Some(tarball_url.into()),
            manifest_url: Some(manifest_url.into()),
            tarball_sha256: Some(tarball_sha256.into()),
            channel: None,
        }
    }

    fn channel_request(channel: impl Into<String>) -> SkillsSyncRequest {
        SkillsSyncRequest {
            tarball_url: None,
            manifest_url: None,
            tarball_sha256: None,
            channel: Some(channel.into()),
        }
    }

    fn test_signing_key() -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[9_u8; 32])
    }

    fn write_fake_cli(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("fake-finite");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn packed_bundle(dir: &Path) -> finite_release::PackOutput {
        let source = dir.join("source-tree");
        std::fs::create_dir_all(source.join("demo-skill")).unwrap();
        std::fs::write(
            source.join("demo-skill/SKILL.md"),
            "---\nname: demo\ndescription: Demo skill.\n---\n\nDemo.\n",
        )
        .unwrap();
        finite_release::pack_skills_bundle(
            &finite_release::PackRequest {
                source: &source,
                artifact_id: "skills-demo",
                version_label: "test-1",
                source_git_sha: None,
                created_at: Some("2026-08-06T00:00:00Z"),
                out_dir: &dir.join("packed"),
            },
            &test_signing_key(),
        )
        .unwrap()
    }

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

    #[tokio::test]
    async fn plain_http_urls_are_refused_without_the_dev_escape_hatch() {
        let dir = tempfile::tempdir().unwrap();
        let mut locked = settings(dir.path(), Path::new("/nonexistent"));
        locked.allow_insecure_url = false;
        let request = explicit_request(
            "http://127.0.0.1:1/bundle.tar.gz",
            "http://127.0.0.1:1/bundle.manifest.json",
            "a".repeat(64),
        );

        let error = sync_skills(&locked, &request).await.unwrap_err();
        assert!(matches!(error, AgentdError::InvalidPayload(_)));
        assert!(error.public_message().contains("https"));
    }

    #[tokio::test]
    async fn a_missing_release_public_key_fails_closed_before_any_fetch() {
        let dir = tempfile::tempdir().unwrap();
        let mut unkeyed = settings(dir.path(), Path::new("/nonexistent"));
        unkeyed.release_public_key = None;
        let request = explicit_request(
            "https://release.invalid/bundle.tar.gz",
            "https://release.invalid/bundle.manifest.json",
            "a".repeat(64),
        );

        let error = sync_skills(&unkeyed, &request).await.unwrap_err();
        assert!(matches!(error, AgentdError::MissingReleaseKey));
        assert_eq!(error.public_code(), "release_key_missing");
    }

    #[tokio::test]
    async fn a_verified_bundle_is_handed_to_the_finite_cli_and_staging_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let packed = packed_bundle(dir.path());
        let base_url = serve_bytes(vec![
            (
                "/skills-demo.tar.gz".to_owned(),
                std::fs::read(&packed.tarball_path).unwrap(),
            ),
            (
                "/skills-demo.manifest.json".to_owned(),
                std::fs::read(&packed.manifest_path).unwrap(),
            ),
        ])
        .await;
        let seen_args = dir.path().join("seen-args");
        let cli = write_fake_cli(
            dir.path(),
            &format!(
                "printf '%s\\n' \"$@\" > {}\ntest -f \"$4/demo-skill/SKILL.md\" || exit 3\nprintf '%s\\n' '{{\"applied\": true, \"skillCount\": 3, \"treeDigest\": \"ignored\"}}'",
                seen_args.display()
            ),
        );
        let staging_root = dir.path().join("staging");
        let request = explicit_request(
            format!("{base_url}/skills-demo.tar.gz"),
            format!("{base_url}/skills-demo.manifest.json"),
            packed.manifest.tarball_sha256.clone(),
        );

        let result = sync_skills(&settings(&staging_root, &cli), &request)
            .await
            .unwrap();

        assert_eq!(result["applied"], json!(true));
        assert_eq!(result["artifactId"], json!("skills-demo"));
        assert_eq!(result["treeDigest"], json!(packed.manifest.tree_digest));
        assert_eq!(result["skillCount"], json!(3));
        assert!(result.get("error").is_none());
        let args = std::fs::read_to_string(&seen_args).unwrap();
        assert!(args.contains("skills\nsync\n--source\n"));
        assert!(args.trim_end().ends_with("--json"));
        assert!(
            !staging_root.join("skills-demo").exists(),
            "staging must be cleaned up after a successful sync"
        );
    }

    #[tokio::test]
    async fn a_tampered_tarball_is_refused_and_staging_is_kept_for_diagnosis() {
        let dir = tempfile::tempdir().unwrap();
        let packed = packed_bundle(dir.path());
        let mut tarball = std::fs::read(&packed.tarball_path).unwrap();
        let middle = tarball.len() / 2;
        tarball[middle] ^= 0x01;
        let base_url = serve_bytes(vec![
            ("/skills-demo.tar.gz".to_owned(), tarball),
            (
                "/skills-demo.manifest.json".to_owned(),
                std::fs::read(&packed.manifest_path).unwrap(),
            ),
        ])
        .await;
        let cli = write_fake_cli(dir.path(), "exit 7");
        let staging_root = dir.path().join("staging");
        let request = explicit_request(
            format!("{base_url}/skills-demo.tar.gz"),
            format!("{base_url}/skills-demo.manifest.json"),
            packed.manifest.tarball_sha256.clone(),
        );

        let error = sync_skills(&settings(&staging_root, &cli), &request)
            .await
            .unwrap_err();

        assert!(matches!(error, AgentdError::SkillsBundle(_)));
        assert!(
            staging_root.join("skills-demo").exists(),
            "failed staging must be kept for diagnosis"
        );
    }

    #[tokio::test]
    async fn a_command_digest_disagreeing_with_the_manifest_is_refused_before_the_tarball_fetch() {
        let dir = tempfile::tempdir().unwrap();
        let packed = packed_bundle(dir.path());
        let base_url = serve_bytes(vec![(
            "/skills-demo.manifest.json".to_owned(),
            std::fs::read(&packed.manifest_path).unwrap(),
        )])
        .await;
        let cli = write_fake_cli(dir.path(), "exit 7");
        let request = explicit_request(
            format!("{base_url}/absent.tar.gz"),
            format!("{base_url}/skills-demo.manifest.json"),
            "b".repeat(64),
        );

        let error = sync_skills(&settings(dir.path(), &cli), &request)
            .await
            .unwrap_err();

        assert!(matches!(error, AgentdError::SkillsBundle(_)));
        assert!(
            error
                .public_message()
                .contains("does not match the manifest")
        );
    }

    #[tokio::test]
    async fn a_failing_finite_cli_surfaces_its_error_and_keeps_staging() {
        let dir = tempfile::tempdir().unwrap();
        let packed = packed_bundle(dir.path());
        let base_url = serve_bytes(vec![
            (
                "/skills-demo.tar.gz".to_owned(),
                std::fs::read(&packed.tarball_path).unwrap(),
            ),
            (
                "/skills-demo.manifest.json".to_owned(),
                std::fs::read(&packed.manifest_path).unwrap(),
            ),
        ])
        .await;
        let cli = write_fake_cli(
            dir.path(),
            "echo 'finite skills sync failed: injected CLI failure' >&2\nexit 1",
        );
        let staging_root = dir.path().join("staging");
        let request = explicit_request(
            format!("{base_url}/skills-demo.tar.gz"),
            format!("{base_url}/skills-demo.manifest.json"),
            packed.manifest.tarball_sha256.clone(),
        );

        let error = sync_skills(&settings(&staging_root, &cli), &request)
            .await
            .unwrap_err();

        assert!(matches!(error, AgentdError::SkillsBundle(_)));
        assert!(error.public_message().contains("injected CLI failure"));
        assert!(
            staging_root.join("skills-demo/tree").exists(),
            "the verified tree must remain for diagnosis"
        );
    }

    /// A signed `finite_service_directory.v1` document advertising the packed
    /// test bundle as the canary skills head (or no head at all).
    fn signed_directory_bytes(
        bundle_base_url: Option<&str>,
        packed: &finite_release::PackOutput,
    ) -> Vec<u8> {
        use std::collections::BTreeMap;

        use finite_service_directory::{
            ChannelBundleV1, SERVICE_DIRECTORY_SCHEMA, ServiceDirectoryV1, ServiceEntryV1,
        };

        let mut channels = BTreeMap::new();
        if let Some(base_url) = bundle_base_url {
            let tarball_url = format!("{base_url}/skills-demo.tar.gz");
            channels.insert(
                "canary".to_owned(),
                BTreeMap::from([(
                    finite_service_directory::SKILLS_BUNDLE_KIND.to_owned(),
                    ChannelBundleV1 {
                        artifact_id: packed.manifest.artifact_id.clone(),
                        version_label: packed.manifest.version_label.clone(),
                        manifest_url: format!("{tarball_url}.manifest.json"),
                        tarball_url,
                        tarball_sha256: packed.manifest.tarball_sha256.clone(),
                    },
                )]),
            );
        }
        let mut directory = ServiceDirectoryV1 {
            schema: SERVICE_DIRECTORY_SCHEMA.to_owned(),
            generated_at: time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap(),
            services: BTreeMap::from([(
                "sites".to_owned(),
                ServiceEntryV1 {
                    base_url: "http://192.168.64.1:18789".to_owned(),
                    client_version: None,
                },
            )]),
            channels,
            key_id: String::new(),
            signature: String::new(),
        };
        directory.sign(&test_signing_key()).unwrap();
        serde_json::to_vec(&directory).unwrap()
    }

    #[tokio::test]
    async fn the_channel_form_resolves_the_bundle_through_the_signed_directory() {
        let dir = tempfile::tempdir().unwrap();
        let packed = packed_bundle(dir.path());
        let bundle_base = serve_bytes(vec![
            (
                "/skills-demo.tar.gz".to_owned(),
                std::fs::read(&packed.tarball_path).unwrap(),
            ),
            (
                "/skills-demo.tar.gz.manifest.json".to_owned(),
                std::fs::read(&packed.manifest_path).unwrap(),
            ),
        ])
        .await;
        let directory_base = serve_bytes(vec![(
            "/api/core/v1/service-directory".to_owned(),
            signed_directory_bytes(Some(&bundle_base), &packed),
        )])
        .await;
        let cli = write_fake_cli(
            dir.path(),
            "printf '%s\\n' '{\"applied\": true, \"skillCount\": 1, \"treeDigest\": \"ignored\"}'",
        );
        let staging_root = dir.path().join("staging");
        let mut settings = settings(&staging_root, &cli);
        settings.service_directory_url =
            Some(format!("{directory_base}/api/core/v1/service-directory"));

        let result = sync_skills(&settings, &channel_request("canary"))
            .await
            .unwrap();

        assert_eq!(result["applied"], json!(true));
        assert_eq!(result["artifactId"], json!("skills-demo"));
        assert_eq!(
            result["channel"],
            json!("canary"),
            "the result must prove the bundle was resolved through the directory"
        );
        assert_eq!(result["treeDigest"], json!(packed.manifest.tree_digest));
    }

    #[tokio::test]
    async fn a_channel_with_no_skills_bundle_head_is_a_distinct_error() {
        let dir = tempfile::tempdir().unwrap();
        let packed = packed_bundle(dir.path());
        let directory_base = serve_bytes(vec![(
            "/api/core/v1/service-directory".to_owned(),
            signed_directory_bytes(None, &packed),
        )])
        .await;
        let cli = write_fake_cli(dir.path(), "exit 7");
        let mut settings = settings(dir.path(), &cli);
        settings.service_directory_url =
            Some(format!("{directory_base}/api/core/v1/service-directory"));

        let error = sync_skills(&settings, &channel_request("canary"))
            .await
            .unwrap_err();

        assert!(matches!(error, AgentdError::SkillsChannelHeadMissing(_)));
        assert_eq!(error.public_code(), "skills_channel_head_missing");
        assert!(error.public_message().contains("canary"));
    }

    #[tokio::test]
    async fn the_channel_form_requires_a_configured_directory_url() {
        let dir = tempfile::tempdir().unwrap();
        let cli = write_fake_cli(dir.path(), "exit 7");
        let error = sync_skills(&settings(dir.path(), &cli), &channel_request("canary"))
            .await
            .unwrap_err();
        assert!(matches!(error, AgentdError::Config(_)));
        assert!(error.public_message().contains(SERVICE_DIRECTORY_URL_ENV));
    }

    #[tokio::test]
    async fn channel_and_explicit_bundle_fields_are_mutually_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let cli = write_fake_cli(dir.path(), "exit 7");
        let mut request = explicit_request(
            "https://release.invalid/bundle.tar.gz",
            "https://release.invalid/bundle.tar.gz.manifest.json",
            "a".repeat(64),
        );
        request.channel = Some("canary".to_owned());
        let error = sync_skills(&settings(dir.path(), &cli), &request)
            .await
            .unwrap_err();
        assert!(matches!(error, AgentdError::InvalidPayload(_)));
        assert!(error.public_message().contains("mutually exclusive"));

        let empty = SkillsSyncRequest {
            tarball_url: None,
            manifest_url: None,
            tarball_sha256: None,
            channel: None,
        };
        let error = sync_skills(&settings(dir.path(), &cli), &empty)
            .await
            .unwrap_err();
        assert!(matches!(error, AgentdError::InvalidPayload(_)));
    }

    fn daemon_config(agent_home: &Path, finite_cli: &Path) -> DaemonConfig {
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
            finite_cli: finite_cli.to_path_buf(),
            service_directory_url: None,
            shell_socket: PathBuf::from("/nonexistent"),
            shell_control_token: None,
            brain_sync_bin: None,
        }
    }

    #[tokio::test]
    async fn the_cli_entry_runs_the_daemon_path_and_records_the_result_in_the_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let packed = packed_bundle(dir.path());
        let base_url = serve_bytes(vec![
            (
                "/skills-demo.tar.gz".to_owned(),
                std::fs::read(&packed.tarball_path).unwrap(),
            ),
            (
                "/skills-demo.manifest.json".to_owned(),
                std::fs::read(&packed.manifest_path).unwrap(),
            ),
        ])
        .await;
        let invocations = dir.path().join("cli-invocations");
        let cli = write_fake_cli(
            dir.path(),
            &format!(
                "printf 'run\\n' >> {}\nprintf '%s\\n' '{{\"applied\": true, \"skillCount\": 2, \"treeDigest\": \"ignored\"}}'",
                invocations.display()
            ),
        );
        let agent_home = dir.path().join("agent-home");
        let config = daemon_config(&agent_home, &cli);

        let first = run_skills_sync_cli(
            &config,
            "cli-request-1",
            explicit_request(
                format!("{base_url}/skills-demo.tar.gz"),
                format!("{base_url}/skills-demo.manifest.json"),
                packed.manifest.tarball_sha256.clone(),
            ),
        )
        .await
        .unwrap();

        assert_eq!(first["applied"], json!(true));
        assert_eq!(first["treeDigest"], json!(packed.manifest.tree_digest));
        assert!(
            agent_home.join("agentd/agentd.sqlite3").is_file(),
            "the CLI must record through the daemon's own ledger file"
        );

        // The same request id replays the durable terminal result instead of
        // re-fetching and re-applying — proof the exact daemon ledger path ran.
        let replayed = run_skills_sync_cli(
            &config,
            "cli-request-1",
            explicit_request(
                format!("{base_url}/skills-demo.tar.gz"),
                format!("{base_url}/skills-demo.manifest.json"),
                packed.manifest.tarball_sha256.clone(),
            ),
        )
        .await
        .unwrap();
        assert_eq!(replayed, first);
        assert_eq!(
            std::fs::read_to_string(&invocations)
                .unwrap()
                .lines()
                .count(),
            1,
            "a replayed request id must not run the finite CLI again"
        );
    }

    #[tokio::test]
    async fn auto_converge_applies_the_channel_skills_head_once_and_dedups_unchanged() {
        use finite_service_directory::ServiceDirectoryV1;

        let dir = tempfile::tempdir().unwrap();
        let packed = packed_bundle(dir.path());
        let bundle_base = serve_bytes(vec![
            (
                "/skills-demo.tar.gz".to_owned(),
                std::fs::read(&packed.tarball_path).unwrap(),
            ),
            (
                "/skills-demo.tar.gz.manifest.json".to_owned(),
                std::fs::read(&packed.manifest_path).unwrap(),
            ),
        ])
        .await;
        let directory_base = serve_bytes(vec![(
            "/api/core/v1/service-directory".to_owned(),
            signed_directory_bytes(Some(&bundle_base), &packed),
        )])
        .await;
        let invocations = dir.path().join("cli-invocations");
        let cli = write_fake_cli(
            dir.path(),
            &format!(
                "printf 'run\\n' >> {}\nprintf '%s\\n' '{{\"applied\": true, \"skillCount\": 1, \"treeDigest\": \"ignored\"}}'",
                invocations.display()
            ),
        );
        let agent_home = dir.path().join("agent-home");
        let mut config = daemon_config(&agent_home, &cli);
        config.service_directory_url =
            Some(format!("{directory_base}/api/core/v1/service-directory"));
        // The channel the shell recorded beside its socket.
        let shell_dir = dir.path().join("shell");
        std::fs::create_dir_all(&shell_dir).unwrap();
        std::fs::write(shell_dir.join("channel"), "canary\n").unwrap();
        config.shell_socket = shell_dir.join("shell.sock");

        let directory = ServiceDirectoryV1::from_verified_json_bytes(
            &signed_directory_bytes(Some(&bundle_base), &packed),
            &test_signing_key().verifying_key(),
        )
        .unwrap();

        // First tick applies the head through the verified sync path.
        crate::directory::converge_channel_skills(&config, &directory).await;
        assert_eq!(
            std::fs::read_to_string(&invocations)
                .unwrap()
                .lines()
                .count(),
            1,
            "the first converge must apply the head"
        );
        let marker =
            std::fs::read_to_string(agent_home.join("agentd/applied-skills-head")).unwrap();
        assert!(marker.contains(&packed.manifest.artifact_id));
        assert!(marker.contains(&packed.manifest.tarball_sha256));

        // Second tick with an unchanged head does no work (the marker dedups
        // before any fetch or CLI run).
        crate::directory::converge_channel_skills(&config, &directory).await;
        assert_eq!(
            std::fs::read_to_string(&invocations)
                .unwrap()
                .lines()
                .count(),
            1,
            "an unchanged head must not re-apply"
        );
    }

    #[tokio::test]
    async fn the_cli_entry_records_failures_and_fails_closed_without_a_release_key() {
        let dir = tempfile::tempdir().unwrap();
        let cli = write_fake_cli(dir.path(), "exit 7");
        let agent_home = dir.path().join("agent-home");
        let mut config = daemon_config(&agent_home, &cli);
        config.release_public_key = None;

        let error = run_skills_sync_cli(
            &config,
            "cli-request-unkeyed",
            explicit_request(
                "https://release.invalid/bundle.tar.gz",
                "https://release.invalid/bundle.manifest.json",
                "a".repeat(64),
            ),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AgentdError::MissingReleaseKey));

        // The failure is terminal in the ledger: the same request id replays
        // the recorded failure without executing again.
        let replayed = run_skills_sync_cli(
            &config,
            "cli-request-unkeyed",
            explicit_request(
                "https://release.invalid/bundle.tar.gz",
                "https://release.invalid/bundle.manifest.json",
                "a".repeat(64),
            ),
        )
        .await
        .unwrap_err();
        assert!(
            replayed
                .public_message()
                .contains("replayed from the ledger")
        );
    }
}
