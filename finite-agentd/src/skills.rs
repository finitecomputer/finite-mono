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

use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::process::Command as TokioCommand;

use crate::AgentdError;
use crate::daemon::DaemonConfig;

pub(crate) const SKILLS_SYNC_SCHEMA: &str = "finite.agent.skills.sync.v1";

const MANIFEST_FETCH_CAP_BYTES: usize = 64 * 1024;
const TARBALL_FETCH_CAP_BYTES: usize = 64 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
const CLI_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) const RELEASE_PUBLIC_KEY_ENV: &str = "FINITE_RELEASE_PUBLIC_KEY";
pub(crate) const ALLOW_INSECURE_BUNDLE_URL_ENV: &str = "FINITE_ALLOW_INSECURE_BUNDLE_URL";
pub(crate) const FINITE_CLI_PATH_ENV: &str = "FINITE_CLI_PATH";
pub(crate) const DEFAULT_FINITE_CLI_PATH: &str = "/runtime/bin/finite";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SkillsSyncRequest {
    pub tarball_url: String,
    pub manifest_url: String,
    pub tarball_sha256: String,
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
}

impl SkillsSyncSettings {
    pub(crate) fn from_daemon_config(config: &DaemonConfig) -> Self {
        Self {
            staging_root: config.state_dir().join("staging").join("skills"),
            release_public_key: config.release_public_key.clone(),
            allow_insecure_url: config.allow_insecure_bundle_url,
            finite_cli: config.finite_cli.clone(),
        }
    }
}

pub(crate) async fn sync_skills(
    settings: &SkillsSyncSettings,
    request: &SkillsSyncRequest,
) -> Result<Value, AgentdError> {
    let expected_tarball_sha256 = request.tarball_sha256.trim().to_ascii_lowercase();
    if !finite_release::is_lower_hex_256(&expected_tarball_sha256) {
        return Err(AgentdError::InvalidPayload(
            "tarballSha256 must be 64 hex characters".to_owned(),
        ));
    }
    check_url_policy(&request.tarball_url, settings.allow_insecure_url)?;
    check_url_policy(&request.manifest_url, settings.allow_insecure_url)?;
    let public_key_hex = settings
        .release_public_key
        .as_deref()
        .ok_or(AgentdError::MissingReleaseKey)?;
    let public_key = finite_release::parse_verifying_key_hex(public_key_hex).map_err(|error| {
        AgentdError::Config(format!("{RELEASE_PUBLIC_KEY_ENV} is invalid: {error}"))
    })?;

    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
        .build()?;
    let manifest_bytes = fetch_bounded(
        &client,
        &request.manifest_url,
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
        &request.tarball_url,
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

fn check_url_policy(url: &str, allow_insecure: bool) -> Result<(), AgentdError> {
    if url.starts_with("https://") || (allow_insecure && url.starts_with("http://")) {
        return Ok(());
    }
    Err(AgentdError::InvalidPayload(format!(
        "bundle URLs must use https ({ALLOW_INSECURE_BUNDLE_URL_ENV}=1 admits http in the dev harness only): {url:?}"
    )))
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
    let manifest_path = staging_dir.join(format!("{}.manifest.json", manifest.artifact_id));
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
        let request = SkillsSyncRequest {
            tarball_url: "http://127.0.0.1:1/bundle.tar.gz".to_owned(),
            manifest_url: "http://127.0.0.1:1/bundle.manifest.json".to_owned(),
            tarball_sha256: "a".repeat(64),
        };

        let error = sync_skills(&locked, &request).await.unwrap_err();
        assert!(matches!(error, AgentdError::InvalidPayload(_)));
        assert!(error.public_message().contains("https"));
    }

    #[tokio::test]
    async fn a_missing_release_public_key_fails_closed_before_any_fetch() {
        let dir = tempfile::tempdir().unwrap();
        let mut unkeyed = settings(dir.path(), Path::new("/nonexistent"));
        unkeyed.release_public_key = None;
        let request = SkillsSyncRequest {
            tarball_url: "https://release.invalid/bundle.tar.gz".to_owned(),
            manifest_url: "https://release.invalid/bundle.manifest.json".to_owned(),
            tarball_sha256: "a".repeat(64),
        };

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
        let request = SkillsSyncRequest {
            tarball_url: format!("{base_url}/skills-demo.tar.gz"),
            manifest_url: format!("{base_url}/skills-demo.manifest.json"),
            tarball_sha256: packed.manifest.tarball_sha256.clone(),
        };

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
        let request = SkillsSyncRequest {
            tarball_url: format!("{base_url}/skills-demo.tar.gz"),
            manifest_url: format!("{base_url}/skills-demo.manifest.json"),
            tarball_sha256: packed.manifest.tarball_sha256.clone(),
        };

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
        let request = SkillsSyncRequest {
            tarball_url: format!("{base_url}/absent.tar.gz"),
            manifest_url: format!("{base_url}/skills-demo.manifest.json"),
            tarball_sha256: "b".repeat(64),
        };

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
        let request = SkillsSyncRequest {
            tarball_url: format!("{base_url}/skills-demo.tar.gz"),
            manifest_url: format!("{base_url}/skills-demo.manifest.json"),
            tarball_sha256: packed.manifest.tarball_sha256.clone(),
        };

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
}
