//! Generation layout: unpacked payload roots under `/data/generations/`,
//! selected by the relative `current`/`previous` symlinks (swapped by
//! renaming a temporary symlink — atomic on the virtiofs `/data` bind, as the
//! M1 spike proved). Staging verifies a signed payload bundle end to end,
//! enforces the shell-version and bad-list gates, runs the venv fixup, and
//! moves the tree into place.

use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::state::{SharedState, StagedGeneration, now_rfc3339};
use crate::{
    DataLayout, SHELL_VERSION, STAGE_FETCH_TIMEOUT, STAGE_HEADROOM_BYTES, STAGE_MANIFEST_CAP_BYTES,
    STAGE_TARBALL_CAP_BYTES, ShellError, ShellSettings, fixup,
};

/// The one thing the shell requires inside an otherwise opaque payload tree.
pub const PAYLOAD_AGENTD_RELATIVE: &str = "bin/finite-agentd";

/// Read which generation a `current`/`previous` symlink names, if any.
pub fn read_link_version(link: &Path) -> Option<String> {
    let target = fs::read_link(link).ok()?;
    target.file_name()?.to_str().map(str::to_owned)
}

/// Refuse any version label that is not exactly one safe path component:
/// non-empty, no separators, not `.`/`..`, not the reserved link names, no
/// leading dot (dot-entries are shell staging temps). This is the shell's own
/// containment layer, enforced before ANY filesystem operation derived from a
/// label — independent of finite-release's manifest validation, so a signed
/// label like `../agent` can never escape `/data/generations/` even if one
/// layer regresses.
pub fn validate_generation_name(version: &str) -> Result<(), ShellError> {
    let unsafe_label = || ShellError::UnsafeVersionLabel(version.to_owned());
    if version.is_empty() || version.len() > 81 || version.starts_with('.') {
        return Err(unsafe_label());
    }
    if matches!(version, "current" | "previous") {
        return Err(unsafe_label());
    }
    if !version
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(unsafe_label());
    }
    // Belt and braces: the string must parse as exactly one normal component.
    let mut components = Path::new(version).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err(unsafe_label()),
    }
}

/// The one way to derive a generation directory from a version label for any
/// filesystem operation: validates the label as a single safe component and
/// proves canonical containment under `generations/` before returning the
/// path.
pub fn contained_generation_dir(layout: &DataLayout, version: &str) -> Result<PathBuf, ShellError> {
    validate_generation_name(version)?;
    let generations_dir = layout.generations_dir();
    fs::create_dir_all(&generations_dir)?;
    let canonical_parent = fs::canonicalize(&generations_dir)?;
    let dir = canonical_parent.join(version);
    // A single validated component cannot escape its parent; assert it
    // anyway so a future refactor cannot silently weaken this.
    if dir.parent() != Some(canonical_parent.as_path()) {
        return Err(ShellError::UnsafeVersionLabel(version.to_owned()));
    }
    Ok(layout.generation_dir(version))
}

/// Remove stale staging debris under `generations/`: dot-prefixed entries
/// (`.stage-tmp-*`, `.seed-tmp`, orphaned `.current.tmp.*` symlinks) are all
/// shell temp artifacts; a crash mid-stage must not leak them forever. Runs
/// at boot and before each stage.
pub fn sweep_stale_temp(layout: &DataLayout) -> Result<(), ShellError> {
    let staging_dir = layout.staging_dir();
    if staging_dir.exists() {
        let _ = fs::remove_dir_all(&staging_dir);
    }
    let generations_dir = layout.generations_dir();
    if !generations_dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&generations_dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            let _ = fs::remove_dir_all(&path);
        } else {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

/// Delete generation directories that are not current, previous, or staged.
/// Bad-list entries keep their `state.json` records (the record is the
/// memory) but lose their directories (the bytes are reclaimable). Runs at
/// boot and after each successful flip's health gate.
pub fn prune_generations(
    layout: &DataLayout,
    state: &SharedState,
) -> Result<Vec<String>, ShellError> {
    let generations_dir = layout.generations_dir();
    if !generations_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut keep: Vec<String> = Vec::new();
    keep.extend(read_link_version(&layout.current_link()));
    keep.extend(read_link_version(&layout.previous_link()));
    if let Some(staged) = state.snapshot().staged {
        keep.push(staged.version_label);
    }
    let mut pruned = Vec::new();
    for entry in fs::read_dir(&generations_dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        // Dot-entries belong to the temp sweep; the link names are never
        // generation directories.
        if name.starts_with('.') || matches!(name.as_str(), "current" | "previous") {
            continue;
        }
        // Only prune real directories (the links are symlinks).
        if entry.file_type()?.is_symlink() || !entry.path().is_dir() {
            continue;
        }
        if keep.contains(&name) {
            continue;
        }
        fs::remove_dir_all(entry.path())?;
        pruned.push(name);
    }
    Ok(pruned)
}

/// Refuse to stage unless `/data` has `2 × tarball size + 512 MiB` free —
/// download + unpack both land on `/data`, and a full `/data` disables the
/// staging mechanism a fix would ship through.
fn ensure_disk_headroom(data_dir: &Path, tarball_bytes: u64) -> Result<(), ShellError> {
    let needed_bytes = tarball_bytes
        .saturating_mul(2)
        .saturating_add(STAGE_HEADROOM_BYTES);
    let stats = rustix::fs::statvfs(data_dir).map_err(std::io::Error::from)?;
    let available_bytes = stats.f_bavail.saturating_mul(stats.f_frsize);
    if available_bytes < needed_bytes {
        return Err(ShellError::InsufficientDisk {
            needed_bytes,
            available_bytes,
        });
    }
    Ok(())
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

    // Hygiene first: a crash mid-stage must never accumulate debris.
    sweep_stale_temp(layout)?;
    let staging_dir = layout.staging_dir();
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
            let tarball_path = staging_dir.join("payload.tar.gz");
            let manifest_path = staging_dir.join("payload.tar.gz.manifest.json");
            fs::write(&manifest_path, &manifest_bytes)?;
            // The tarball streams to a temp file on /data — never buffered in
            // RAM — with its sha256 computed as the bytes arrive.
            let streamed_sha256 = fetch_to_file(
                &client,
                &tarball_url,
                STAGE_TARBALL_CAP_BYTES as u64,
                &layout.data_dir,
                &tarball_path,
            )
            .await?;
            if streamed_sha256 != expected {
                let _ = fs::remove_dir_all(&staging_dir);
                return Err(ShellError::Release(
                    finite_release::ReleaseError::VerificationFailed(format!(
                        "tarball sha256 {streamed_sha256} does not match the expected {expected}"
                    )),
                ));
            }
            (tarball_path, manifest_path, Some(expected))
        }
    };

    // Disk preflight against the actual tarball size before any unpack work
    // (the streaming fetch already preflighted against Content-Length).
    ensure_disk_headroom(&layout.data_dir, fs::metadata(&tarball_path)?.len())?;

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
    // Path containment: the shell's own layer, independent of the manifest
    // validation that already refused unsafe labels above.
    let final_dir = contained_generation_dir(layout, &version)?;
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
    .map_err(|error| ShellError::Fetch(format!("verification task failed: {error}")))
    .and_then(|result| result.map_err(ShellError::from));
    let verified = match verified {
        Ok(verified) => verified,
        Err(error) => {
            // A refused bundle leaves no debris behind.
            let _ = fs::remove_dir_all(&unpack_dir);
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(error);
        }
    };
    let manifest = verified.manifest;

    if !unpack_dir.join(PAYLOAD_AGENTD_RELATIVE).is_file() {
        fs::remove_dir_all(&unpack_dir)?;
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(ShellError::Contract(format!(
            "payload tree does not contain {PAYLOAD_AGENTD_RELATIVE}"
        )));
    }

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

/// Stream a download to `destination` on `/data` (never buffered in RAM),
/// hashing sha256 as bytes arrive. Preflights disk headroom against
/// Content-Length when the server provides one, and enforces `cap` on the
/// actual bytes. Returns the streamed content's sha256 hex.
async fn fetch_to_file(
    client: &reqwest::Client,
    url: &str,
    cap: u64,
    data_dir: &Path,
    destination: &Path,
) -> Result<String, ShellError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| ShellError::Fetch(format!("tarball fetch failed: {error}")))?;
    if !response.status().is_success() {
        return Err(ShellError::Fetch(format!(
            "tarball fetch failed with {}",
            response.status()
        )));
    }
    if let Some(length) = response.content_length() {
        if length > cap {
            return Err(ShellError::Fetch(format!(
                "tarball exceeds the {cap}-byte cap"
            )));
        }
        ensure_disk_headroom(data_dir, length)?;
    }
    let mut file = fs::File::create(destination)?;
    let mut digest = Sha256::new();
    let mut written = 0_u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|error| ShellError::Fetch(format!("tarball fetch failed: {error}")))?;
        written += chunk.len() as u64;
        if written > cap {
            let _ = fs::remove_file(destination);
            return Err(ShellError::Fetch(format!(
                "tarball exceeds the {cap}-byte cap"
            )));
        }
        digest.update(&chunk);
        file.write_all(&chunk)?;
    }
    file.sync_all()?;
    Ok(hex::encode(digest.finalize()))
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
