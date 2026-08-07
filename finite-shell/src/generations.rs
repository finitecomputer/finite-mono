//! Generation layout: unpacked payload roots under `/data/generations/`,
//! selected by the relative `current`/`previous` symlinks (swapped by
//! renaming a temporary symlink — atomic on the virtiofs `/data` bind, as the
//! M1 spike proved). Staging verifies a signed payload bundle end to end,
//! enforces the shell-version and bad-list gates, runs the venv fixup, and
//! moves the tree into place.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use serde::Deserialize;

use crate::state::{SharedState, StagedGeneration, now_rfc3339};
use crate::{
    DataLayout, SHELL_VERSION, STAGE_FETCH_TIMEOUT, STAGE_MANIFEST_CAP_BYTES,
    STAGE_TARBALL_CAP_BYTES, ShellError, ShellSettings, fixup,
};

/// The one thing the shell requires inside an otherwise opaque payload tree.
pub const PAYLOAD_AGENTD_RELATIVE: &str = "bin/finite-agentd";

/// Read which generation a `current`/`previous` symlink names, if any.
pub fn read_link_version(link: &Path) -> Option<String> {
    let target = fs::read_link(link).ok()?;
    target.file_name()?.to_str().map(str::to_owned)
}

/// Atomically retarget `link` to the (relative) generation name `version`:
/// create a temporary symlink next to it, then rename over the link.
pub fn set_link_version(link: &Path, version: &str) -> Result<(), ShellError> {
    let parent = link
        .parent()
        .ok_or_else(|| ShellError::Io(std::io::Error::other("symlink has no parent")))?;
    let temporary = parent.join(format!(
        ".{}.tmp.{}",
        link.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("link"),
        std::process::id()
    ));
    let _ = fs::remove_file(&temporary);
    std::os::unix::fs::symlink(version, &temporary)?;
    fs::rename(&temporary, link)?;
    Ok(())
}

pub fn remove_link(link: &Path) -> Result<(), ShellError> {
    match fs::remove_file(link) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Where a `stage` request's bundle bytes come from.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct StageRequest {
    #[serde(default)]
    pub tarball_url: Option<String>,
    #[serde(default)]
    pub manifest_url: Option<String>,
    #[serde(default)]
    pub tarball_sha256: Option<String>,
    #[serde(default)]
    pub tarball_path: Option<PathBuf>,
    #[serde(default)]
    pub manifest_path: Option<PathBuf>,
    #[serde(default)]
    pub force: bool,
}

pub enum StageSource {
    Remote {
        tarball_url: String,
        manifest_url: String,
        tarball_sha256: String,
    },
    Local {
        tarball_path: PathBuf,
        manifest_path: PathBuf,
    },
}

impl StageRequest {
    pub fn source(&self) -> Result<StageSource, ShellError> {
        let remote = [&self.tarball_url, &self.manifest_url, &self.tarball_sha256];
        let local_given = self.tarball_path.is_some() || self.manifest_path.is_some();
        if remote.iter().any(|field| field.is_some()) && local_given {
            return Err(ShellError::InvalidRequest(
                "URL and path stage forms are mutually exclusive".to_owned(),
            ));
        }
        if local_given {
            return match (&self.tarball_path, &self.manifest_path) {
                (Some(tarball_path), Some(manifest_path)) => Ok(StageSource::Local {
                    tarball_path: tarball_path.clone(),
                    manifest_path: manifest_path.clone(),
                }),
                _ => Err(ShellError::InvalidRequest(
                    "tarballPath and manifestPath are both required".to_owned(),
                )),
            };
        }
        match (&self.tarball_url, &self.manifest_url, &self.tarball_sha256) {
            (Some(tarball_url), Some(manifest_url), Some(tarball_sha256)) => {
                Ok(StageSource::Remote {
                    tarball_url: tarball_url.clone(),
                    manifest_url: manifest_url.clone(),
                    tarball_sha256: tarball_sha256.clone(),
                })
            }
            _ => Err(ShellError::InvalidRequest(
                "either tarballUrl + manifestUrl + tarballSha256, or tarballPath + manifestPath, is required"
                    .to_owned(),
            )),
        }
    }
}

/// Stage a payload bundle: fetch (or copy), verify sha256 + signature + tree
/// digest, enforce `min_shell_version` and the bad list, require
/// `bin/finite-agentd`, run the venv fixup, move the tree to
/// `generations/<version>`, and record it as staged. Idempotent for the same
/// verified version.
pub async fn stage_payload(
    settings: &ShellSettings,
    layout: &DataLayout,
    state: &SharedState,
    request: &StageRequest,
) -> Result<StagedGeneration, ShellError> {
    let source = request.source()?;
    let public_key_hex = settings
        .release_public_key
        .as_deref()
        .ok_or(ShellError::MissingReleaseKey)?;
    let public_key = finite_release::parse_verifying_key_hex(public_key_hex)
        .map_err(|error| ShellError::Config(format!("release public key is invalid: {error}")))?;

    let staging_dir = layout.staging_dir();
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)?;
    }
    fs::create_dir_all(&staging_dir)?;

    let (tarball_path, manifest_path, expected_sha256) = match source {
        StageSource::Local {
            tarball_path,
            manifest_path,
        } => (tarball_path, manifest_path, None),
        StageSource::Remote {
            tarball_url,
            manifest_url,
            tarball_sha256,
        } => {
            let expected = tarball_sha256.trim().to_ascii_lowercase();
            if !finite_release::is_lower_hex_256(&expected) {
                return Err(ShellError::InvalidRequest(
                    "tarballSha256 must be 64 hex characters".to_owned(),
                ));
            }
            finite_release::check_bundle_url_policy(
                &tarball_url,
                settings.allow_insecure_bundle_url,
            )?;
            finite_release::check_bundle_url_policy(
                &manifest_url,
                settings.allow_insecure_bundle_url,
            )?;
            let client = reqwest::Client::builder()
                .timeout(STAGE_FETCH_TIMEOUT)
                .connect_timeout(Duration::from_secs(10))
                .build()
                .map_err(|error| ShellError::Fetch(error.to_string()))?;
            let manifest_bytes =
                fetch_bounded(&client, &manifest_url, STAGE_MANIFEST_CAP_BYTES, "manifest").await?;
            let tarball_bytes =
                fetch_bounded(&client, &tarball_url, STAGE_TARBALL_CAP_BYTES, "tarball").await?;
            let tarball_path = staging_dir.join("payload.tar.gz");
            let manifest_path = staging_dir.join("payload.tar.gz.manifest.json");
            fs::write(&tarball_path, &tarball_bytes)?;
            fs::write(&manifest_path, &manifest_bytes)?;
            (tarball_path, manifest_path, Some(expected))
        }
    };

    // Structural manifest read first: the version gates run before the
    // (potentially large) unpack-and-digest work.
    let manifest =
        finite_release::PayloadBundleManifestV1::from_json_bytes(&fs::read(&manifest_path)?)?;
    let required = manifest.min_shell_version()?;
    let shell = finite_release::ShellSemver::parse(SHELL_VERSION)
        .map_err(|error| ShellError::Config(format!("SHELL_VERSION is invalid: {error}")))?;
    if required > shell {
        return Err(ShellError::MinShellVersion {
            required: manifest.min_shell_version.clone(),
            shell: SHELL_VERSION.to_owned(),
        });
    }
    let version = manifest.version_label.clone();
    let snapshot = state.snapshot();
    if snapshot.is_bad(&version) && !request.force {
        return Err(ShellError::VersionBad(version));
    }
    if read_link_version(&layout.current_link()).as_deref() == Some(version.as_str()) {
        return Err(ShellError::VersionIsCurrent(version));
    }

    let unpack_dir = layout
        .generations_dir()
        .join(format!(".stage-tmp-{version}"));
    if unpack_dir.exists() {
        fs::remove_dir_all(&unpack_dir)?;
    }
    fs::create_dir_all(layout.generations_dir())?;
    let verified = tokio::task::spawn_blocking({
        let tarball_path = tarball_path.clone();
        let manifest_path = manifest_path.clone();
        let unpack_dir = unpack_dir.clone();
        move || {
            finite_release::verify_payload_bundle(
                &tarball_path,
                &manifest_path,
                &public_key,
                expected_sha256.as_deref(),
                &unpack_dir,
            )
        }
    })
    .await
    .map_err(|error| ShellError::Fetch(format!("verification task failed: {error}")))??;
    let manifest = verified.manifest;

    if !unpack_dir.join(PAYLOAD_AGENTD_RELATIVE).is_file() {
        fs::remove_dir_all(&unpack_dir)?;
        return Err(ShellError::Contract(format!(
            "payload tree does not contain {PAYLOAD_AGENTD_RELATIVE}"
        )));
    }

    let final_dir = layout.generation_dir(&version);
    fixup::apply_venv_fixup(&unpack_dir, &final_dir, &settings.shell_python)?;
    if final_dir.exists() {
        fs::remove_dir_all(&final_dir)?;
    }
    fs::rename(&unpack_dir, &final_dir)?;
    let _ = fs::remove_dir_all(&staging_dir);

    let record = StagedGeneration {
        version_label: version.clone(),
        artifact_id: manifest.artifact_id.clone(),
        tree_digest: manifest.tree_digest.clone(),
        tarball_sha256: manifest.tarball_sha256.clone(),
        min_shell_version: manifest.min_shell_version.clone(),
        staged_at: now_rfc3339(),
    };
    state.update(|state| {
        if request.force {
            // An explicit forced re-stage is the operator override for a
            // bad-listed version; the retry starts with a clean slate.
            state.bad.retain(|bad| bad.version_label != version);
        }
        state.staged = Some(record.clone());
    })?;
    Ok(record)
}

async fn fetch_bounded(
    client: &reqwest::Client,
    url: &str,
    cap: usize,
    what: &str,
) -> Result<Vec<u8>, ShellError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| ShellError::Fetch(format!("{what} fetch failed: {error}")))?;
    if !response.status().is_success() {
        return Err(ShellError::Fetch(format!(
            "{what} fetch failed with {}",
            response.status()
        )));
    }
    if let Some(length) = response.content_length()
        && length > cap as u64
    {
        return Err(ShellError::Fetch(format!(
            "{what} exceeds the {cap}-byte cap"
        )));
    }
    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|error| ShellError::Fetch(format!("{what} fetch failed: {error}")))?;
        if bytes.len() + chunk.len() > cap {
            return Err(ShellError::Fetch(format!(
                "{what} exceeds the {cap}-byte cap"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
