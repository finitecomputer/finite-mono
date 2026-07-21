//! Provider-independent Runtime Retirement recovery artifacts.
//!
//! The archive is deliberately a boring ZIP containing a versioned manifest
//! and an exact `data/` tree. Extraction never delegates path handling to the
//! ZIP library: every member is matched against the verified manifest first.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
use std::path::{Component, Path, PathBuf};
use time::OffsetDateTime;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const RECOVERY_ZIP_SCHEMA: &str = "finite.agent-runtime-recovery-zip.v1";
const MANIFEST_NAME: &str = "manifest.json";
const DATA_PREFIX: &str = "data/";

#[derive(Debug, thiserror::Error)]
pub enum RecoveryZipError {
    #[error("invalid recovery artifact: {0}")]
    Invalid(String),
    #[error("recovery artifact I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("recovery artifact JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("recovery artifact ZIP failed: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("recovery artifact tree walk failed: {0}")]
    Walk(#[from] walkdir::Error),
}

pub type RecoveryZipResult<T> = Result<T, RecoveryZipError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryEntryKind {
    Directory,
    File,
    Symlink,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryManifestEntry {
    /// UTF-8, slash-separated path relative to `/data`.
    pub path: String,
    pub kind: RecoveryEntryKind,
    /// Unix permission bits only (setuid/setgid/sticky included).
    pub mode: u32,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symlink_target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryManifest {
    pub schema: String,
    pub request_id: String,
    pub project_id: String,
    pub agent_runtime_id: String,
    pub durable_state_id: String,
    pub runtime_artifact_id: String,
    pub runtime_image_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_principal: Option<String>,
    pub created_at: String,
    pub file_count: u64,
    pub total_file_bytes: u64,
    pub entries: Vec<RecoveryManifestEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryArtifactContext {
    pub request_id: String,
    pub project_id: String,
    pub agent_runtime_id: String,
    pub durable_state_id: String,
    pub runtime_artifact_id: String,
    pub runtime_image_digest: String,
    pub agent_principal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRecoveryZip {
    pub manifest: RecoveryManifest,
    pub zip_bytes: u64,
    pub zip_sha256: String,
    pub manifest_sha256: String,
}

/// Produce the content-bearing portion of the manifest used as the quiescence
/// fence. Two consecutive results must be exactly equal before archival.
pub fn source_manifest(data_root: &Path) -> RecoveryZipResult<Vec<RecoveryManifestEntry>> {
    let metadata = fs::symlink_metadata(data_root)?;
    if !metadata.file_type().is_dir() {
        return Err(invalid("data root is not a directory"));
    }

    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    for item in WalkDir::new(data_root).follow_links(false).min_depth(1) {
        let item = item?;
        let relative = item
            .path()
            .strip_prefix(data_root)
            .map_err(|_| invalid("walked path escaped data root"))?;
        let path = normalized_relative_path(relative)?;
        if !seen.insert(path.clone()) {
            return Err(invalid(format!("duplicate source path {path}")));
        }
        reject_path_below_symlink(&path, &entries)?;

        let metadata = fs::symlink_metadata(item.path())?;
        let mode = unix_mode(&metadata);
        let entry = if metadata.file_type().is_dir() {
            RecoveryManifestEntry {
                path,
                kind: RecoveryEntryKind::Directory,
                mode,
                size: 0,
                sha256: None,
                symlink_target: None,
            }
        } else if metadata.file_type().is_file() {
            let (size, sha256) = hash_file(item.path())?;
            RecoveryManifestEntry {
                path,
                kind: RecoveryEntryKind::File,
                mode,
                size,
                sha256: Some(sha256),
                symlink_target: None,
            }
        } else if metadata.file_type().is_symlink() {
            let target = fs::read_link(item.path())?;
            let target = target
                .to_str()
                .ok_or_else(|| invalid("non-UTF-8 symlink target"))?
                .to_string();
            validate_symlink_target(relative, &target)?;
            RecoveryManifestEntry {
                path,
                kind: RecoveryEntryKind::Symlink,
                mode,
                size: target.len() as u64,
                sha256: None,
                symlink_target: Some(target),
            }
        } else {
            return Err(invalid(format!(
                "unsupported source file kind at {}",
                item.path().display()
            )));
        };
        entries.push(entry);
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

pub fn create_recovery_zip(
    data_root: &Path,
    destination: &Path,
    context: &RecoveryArtifactContext,
) -> RecoveryZipResult<VerifiedRecoveryZip> {
    validate_context(context)?;
    let entries = source_manifest(data_root)?;
    let manifest = RecoveryManifest {
        schema: RECOVERY_ZIP_SCHEMA.to_string(),
        request_id: context.request_id.clone(),
        project_id: context.project_id.clone(),
        agent_runtime_id: context.agent_runtime_id.clone(),
        durable_state_id: context.durable_state_id.clone(),
        runtime_artifact_id: context.runtime_artifact_id.clone(),
        runtime_image_digest: context.runtime_image_digest.clone(),
        agent_principal: context.agent_principal.clone(),
        created_at: OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|error| invalid(format!("could not format creation time: {error}")))?,
        file_count: entries
            .iter()
            .filter(|entry| entry.kind == RecoveryEntryKind::File)
            .count() as u64,
        total_file_bytes: entries
            .iter()
            .filter(|entry| entry.kind == RecoveryEntryKind::File)
            .map(|entry| entry.size)
            .sum(),
        entries,
    };
    validate_manifest(&manifest, Some(context))?;

    let parent = destination
        .parent()
        .ok_or_else(|| invalid("recovery ZIP has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("recovery ZIP filename must be UTF-8"))?;
    let temporary = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    if temporary.exists() {
        return Err(invalid("recovery ZIP staging path already exists"));
    }

    let result = (|| -> RecoveryZipResult<()> {
        let output = root_only_create_new(&temporary)?;
        let mut zip = ZipWriter::new(output);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        zip.start_file(MANIFEST_NAME, options.unix_permissions(0o600))?;
        zip.write_all(&manifest_bytes)?;

        for entry in &manifest.entries {
            let archive_name = format!("{DATA_PREFIX}{}", entry.path);
            match entry.kind {
                RecoveryEntryKind::Directory => {
                    zip.add_directory(
                        format!("{archive_name}/"),
                        options.unix_permissions(entry.mode),
                    )?;
                }
                RecoveryEntryKind::File => {
                    zip.start_file(archive_name, options.unix_permissions(entry.mode))?;
                    let mut input = File::open(data_root.join(path_from_manifest(&entry.path)?))?;
                    std::io::copy(&mut input, &mut zip)?;
                }
                RecoveryEntryKind::Symlink => {
                    let target = entry
                        .symlink_target
                        .as_deref()
                        .ok_or_else(|| invalid("symlink entry has no target"))?;
                    zip.start_file(archive_name, options.unix_permissions(0o120777))?;
                    zip.write_all(target.as_bytes())?;
                }
            }
        }
        let mut output = zip.finish()?;
        output.flush()?;
        output.sync_all()?;
        fs::rename(&temporary, destination)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    verify_recovery_zip(destination, Some(context))
}

pub fn verify_recovery_zip(
    path: &Path,
    expected: Option<&RecoveryArtifactContext>,
) -> RecoveryZipResult<VerifiedRecoveryZip> {
    let (zip_bytes, zip_sha256) = hash_file(path)?;
    let file = File::open(path)?;
    let mut zip = ZipArchive::new(file)?;
    let mut names = BTreeSet::new();
    for index in 0..zip.len() {
        let member = zip.by_index(index)?;
        let name = member.name().to_string();
        validate_archive_name(&name)?;
        if !names.insert(name.clone()) {
            return Err(invalid(format!("duplicate ZIP member {name}")));
        }
    }
    if !names.contains(MANIFEST_NAME) {
        return Err(invalid("ZIP has no manifest.json"));
    }

    let mut manifest_bytes = Vec::new();
    zip.by_name(MANIFEST_NAME)?
        .read_to_end(&mut manifest_bytes)?;
    let manifest_sha256 = hash_bytes(&manifest_bytes);
    let manifest: RecoveryManifest = serde_json::from_slice(&manifest_bytes)?;
    validate_manifest(&manifest, expected)?;

    let expected_names = manifest
        .entries
        .iter()
        .map(|entry| match entry.kind {
            RecoveryEntryKind::Directory => format!("{DATA_PREFIX}{}/", entry.path),
            RecoveryEntryKind::File | RecoveryEntryKind::Symlink => {
                format!("{DATA_PREFIX}{}", entry.path)
            }
        })
        .chain(std::iter::once(MANIFEST_NAME.to_string()))
        .collect::<BTreeSet<_>>();
    if names != expected_names {
        return Err(invalid("ZIP members do not exactly match manifest"));
    }

    for entry in &manifest.entries {
        let name = match entry.kind {
            RecoveryEntryKind::Directory => format!("{DATA_PREFIX}{}/", entry.path),
            RecoveryEntryKind::File | RecoveryEntryKind::Symlink => {
                format!("{DATA_PREFIX}{}", entry.path)
            }
        };
        let mut member = zip.by_name(&name)?;
        match entry.kind {
            RecoveryEntryKind::Directory => {
                if !member.is_dir() || member.size() != 0 {
                    return Err(invalid(format!("invalid directory member {name}")));
                }
            }
            RecoveryEntryKind::File => {
                let (size, sha256) = hash_reader(&mut member)?;
                if size != entry.size || Some(sha256.as_str()) != entry.sha256.as_deref() {
                    return Err(invalid(format!("file content mismatch for {name}")));
                }
            }
            RecoveryEntryKind::Symlink => {
                let mut target = String::new();
                member.read_to_string(&mut target)?;
                if target.len() as u64 != entry.size
                    || Some(target.as_str()) != entry.symlink_target.as_deref()
                {
                    return Err(invalid(format!("symlink target mismatch for {name}")));
                }
                validate_symlink_target(Path::new(&entry.path), &target)?;
            }
        }
    }

    Ok(VerifiedRecoveryZip {
        manifest,
        zip_bytes,
        zip_sha256,
        manifest_sha256,
    })
}

pub fn restore_recovery_zip(
    archive: &Path,
    target: &Path,
    expected: Option<&RecoveryArtifactContext>,
) -> RecoveryZipResult<VerifiedRecoveryZip> {
    let verified = verify_recovery_zip(archive, expected)?;
    if target.exists() {
        let metadata = fs::symlink_metadata(target)?;
        if !metadata.file_type().is_dir() || fs::read_dir(target)?.next().is_some() {
            return Err(invalid("restore target must be an empty directory"));
        }
    } else {
        fs::create_dir(target)?;
    }

    let file = File::open(archive)?;
    let mut zip = ZipArchive::new(file)?;
    for entry in verified
        .manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == RecoveryEntryKind::Directory)
    {
        let output = target.join(path_from_manifest(&entry.path)?);
        fs::create_dir(&output)?;
        set_mode(&output, entry.mode)?;
    }
    for entry in verified
        .manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == RecoveryEntryKind::File)
    {
        let output = target.join(path_from_manifest(&entry.path)?);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let name = format!("{DATA_PREFIX}{}", entry.path);
        let mut member = zip.by_name(&name)?;
        let mut file = root_only_create_new(&output)?;
        std::io::copy(&mut member, &mut file)?;
        file.flush()?;
        file.sync_all()?;
        set_mode(&output, entry.mode)?;
    }
    for entry in verified
        .manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == RecoveryEntryKind::Symlink)
    {
        let output = target.join(path_from_manifest(&entry.path)?);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let target_value = entry
            .symlink_target
            .as_deref()
            .ok_or_else(|| invalid("symlink entry has no target"))?;
        create_symlink(target_value, &output)?;
    }
    File::open(target)?.sync_all()?;
    Ok(verified)
}

fn validate_manifest(
    manifest: &RecoveryManifest,
    expected: Option<&RecoveryArtifactContext>,
) -> RecoveryZipResult<()> {
    if manifest.schema != RECOVERY_ZIP_SCHEMA {
        return Err(invalid("unsupported recovery manifest schema"));
    }
    let context = RecoveryArtifactContext {
        request_id: manifest.request_id.clone(),
        project_id: manifest.project_id.clone(),
        agent_runtime_id: manifest.agent_runtime_id.clone(),
        durable_state_id: manifest.durable_state_id.clone(),
        runtime_artifact_id: manifest.runtime_artifact_id.clone(),
        runtime_image_digest: manifest.runtime_image_digest.clone(),
        agent_principal: manifest.agent_principal.clone(),
    };
    validate_context(&context)?;
    if expected.is_some_and(|expected| expected != &context) {
        return Err(invalid("recovery manifest identity mismatch"));
    }

    let mut seen = BTreeSet::new();
    let mut by_path = BTreeMap::new();
    let mut file_count = 0_u64;
    let mut total_file_bytes = 0_u64;
    for entry in &manifest.entries {
        let normalized = normalized_relative_path(Path::new(&entry.path))?;
        if normalized != entry.path || !seen.insert(entry.path.clone()) {
            return Err(invalid(format!(
                "invalid or duplicate manifest path {}",
                entry.path
            )));
        }
        reject_path_below_symlink(&entry.path, &manifest.entries)?;
        if entry.mode > 0o7777 {
            return Err(invalid(format!("invalid mode for {}", entry.path)));
        }
        match entry.kind {
            RecoveryEntryKind::Directory => {
                if entry.size != 0 || entry.sha256.is_some() || entry.symlink_target.is_some() {
                    return Err(invalid(format!(
                        "invalid directory metadata for {}",
                        entry.path
                    )));
                }
            }
            RecoveryEntryKind::File => {
                if entry.sha256.as_deref().is_none_or(|hash| !is_sha256(hash))
                    || entry.symlink_target.is_some()
                {
                    return Err(invalid(format!("invalid file metadata for {}", entry.path)));
                }
                file_count += 1;
                total_file_bytes = total_file_bytes
                    .checked_add(entry.size)
                    .ok_or_else(|| invalid("manifest file size overflow"))?;
            }
            RecoveryEntryKind::Symlink => {
                let target = entry
                    .symlink_target
                    .as_deref()
                    .ok_or_else(|| invalid(format!("missing symlink target for {}", entry.path)))?;
                if entry.sha256.is_some() || entry.size != target.len() as u64 {
                    return Err(invalid(format!(
                        "invalid symlink metadata for {}",
                        entry.path
                    )));
                }
                validate_symlink_target(Path::new(&entry.path), target)?;
            }
        }
        by_path.insert(entry.path.as_str(), entry.kind.clone());
    }
    if file_count != manifest.file_count || total_file_bytes != manifest.total_file_bytes {
        return Err(invalid("manifest totals do not match entries"));
    }
    Ok(())
}

fn validate_context(context: &RecoveryArtifactContext) -> RecoveryZipResult<()> {
    for (name, value) in [
        ("request_id", context.request_id.as_str()),
        ("project_id", context.project_id.as_str()),
        ("agent_runtime_id", context.agent_runtime_id.as_str()),
        ("durable_state_id", context.durable_state_id.as_str()),
        ("runtime_artifact_id", context.runtime_artifact_id.as_str()),
        (
            "runtime_image_digest",
            context.runtime_image_digest.as_str(),
        ),
    ] {
        if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err(invalid(format!("invalid {name}")));
        }
    }
    if context.agent_principal.as_deref().is_some_and(|value| {
        value.is_empty() || value.len() > 256 || value.chars().any(char::is_control)
    }) {
        return Err(invalid("invalid agent_principal"));
    }
    let Some((repository, digest)) = context.runtime_image_digest.rsplit_once("@sha256:") else {
        return Err(invalid("runtime_image_digest is not immutable"));
    };
    if repository.is_empty()
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(invalid("runtime_image_digest is not immutable"));
    }
    Ok(())
}

fn normalized_relative_path(path: &Path) -> RecoveryZipResult<String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(invalid("path must be non-empty and relative"));
    }
    let mut values = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => values.push(
                value
                    .to_str()
                    .ok_or_else(|| invalid("non-UTF-8 path"))?
                    .to_string(),
            ),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(invalid("path contains an unsafe component"));
            }
        }
    }
    if values.is_empty() {
        return Err(invalid("path has no normal components"));
    }
    Ok(values.join("/"))
}

fn validate_archive_name(name: &str) -> RecoveryZipResult<()> {
    if name == MANIFEST_NAME {
        return Ok(());
    }
    let Some(relative) = name.strip_prefix(DATA_PREFIX) else {
        return Err(invalid(format!("unexpected ZIP member {name}")));
    };
    let relative = relative.strip_suffix('/').unwrap_or(relative);
    normalized_relative_path(Path::new(relative)).map(|_| ())
}

fn validate_symlink_target(link_path: &Path, target: &str) -> RecoveryZipResult<()> {
    let target_path = Path::new(target);
    if target.is_empty() || target_path.is_absolute() {
        return Err(invalid("symlink target must be non-empty and relative"));
    }
    let mut depth = link_path.components().count().saturating_sub(1) as isize;
    for component in target_path.components() {
        match component {
            Component::Normal(value) => {
                value
                    .to_str()
                    .ok_or_else(|| invalid("non-UTF-8 symlink target"))?;
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return Err(invalid("symlink target escapes data root"));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(invalid("symlink target escapes data root"));
            }
        }
    }
    Ok(())
}

fn reject_path_below_symlink(
    path: &str,
    entries: &[RecoveryManifestEntry],
) -> RecoveryZipResult<()> {
    let mut parent = Path::new(path).parent();
    while let Some(value) = parent {
        if !value.as_os_str().is_empty() {
            let normalized = normalized_relative_path(value)?;
            if entries
                .iter()
                .any(|entry| entry.path == normalized && entry.kind == RecoveryEntryKind::Symlink)
            {
                return Err(invalid(format!("path {path} is nested below a symlink")));
            }
        }
        parent = value.parent();
    }
    Ok(())
}

fn path_from_manifest(path: &str) -> RecoveryZipResult<PathBuf> {
    normalized_relative_path(Path::new(path))?;
    Ok(path.split('/').collect())
}

fn hash_file(path: &Path) -> RecoveryZipResult<(u64, String)> {
    hash_reader(File::open(path)?)
}

fn hash_reader(mut reader: impl Read) -> RecoveryZipResult<(u64, String)> {
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(read as u64)
            .ok_or_else(|| invalid("file size overflow"))?;
        hasher.update(&buffer[..read]);
    }
    Ok((size, hex::encode(hasher.finalize())))
}

fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn root_only_create_new(path: &Path) -> RecoveryZipResult<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    Ok(options.open(path)?)
}

#[cfg(unix)]
fn unix_mode(metadata: &fs::Metadata) -> u32 {
    metadata.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn unix_mode(_metadata: &fs::Metadata) -> u32 {
    0o600
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> RecoveryZipResult<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> RecoveryZipResult<()> {
    Ok(())
}

#[cfg(unix)]
fn create_symlink(target: &str, link: &Path) -> RecoveryZipResult<()> {
    symlink(target, link)?;
    Ok(())
}

#[cfg(not(unix))]
fn create_symlink(_target: &str, _link: &Path) -> RecoveryZipResult<()> {
    Err(invalid("symlink restore is unsupported on this platform"))
}

fn invalid(message: impl Into<String>) -> RecoveryZipError {
    RecoveryZipError::Invalid(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::io::{Seek, SeekFrom, Write};
    use tempfile::tempdir;

    fn context() -> RecoveryArtifactContext {
        RecoveryArtifactContext {
            request_id: "rcr_test".to_string(),
            project_id: "project_test".to_string(),
            agent_runtime_id: "runtime_test".to_string(),
            durable_state_id: "state_test".to_string(),
            runtime_artifact_id: "artifact_test".to_string(),
            runtime_image_digest: format!("registry.example/runtime@sha256:{}", "a".repeat(64)),
            agent_principal: Some("npub1test".to_string()),
        }
    }

    fn fixture(root: &Path) {
        fs::create_dir_all(root.join("hermes/db")).unwrap();
        fs::create_dir_all(root.join("workspace")).unwrap();
        fs::write(root.join("hermes/db/chat.sqlite3"), b"sqlite-state").unwrap();
        fs::write(root.join("workspace/note.txt"), b"hello").unwrap();
        #[cfg(unix)]
        {
            fs::set_permissions(
                root.join("workspace/note.txt"),
                fs::Permissions::from_mode(0o640),
            )
            .unwrap();
            symlink("workspace/note.txt", root.join("latest-note")).unwrap();
        }
    }

    #[test]
    fn round_trip_preserves_content_modes_and_symlinks() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        fixture(&source);
        let archive = temp.path().join("retirement.zip");
        let created = create_recovery_zip(&source, &archive, &context()).unwrap();
        assert_eq!(created.manifest.schema, RECOVERY_ZIP_SCHEMA);

        let restored = temp.path().join("restored");
        restore_recovery_zip(&archive, &restored, Some(&context())).unwrap();
        assert_eq!(
            fs::read(restored.join("hermes/db/chat.sqlite3")).unwrap(),
            b"sqlite-state"
        );
        #[cfg(unix)]
        {
            assert_eq!(
                fs::metadata(restored.join("workspace/note.txt"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o640
            );
            assert_eq!(
                fs::read_link(restored.join("latest-note")).unwrap(),
                PathBuf::from("workspace/note.txt")
            );
        }
    }

    #[test]
    fn round_trip_preserves_sqlite_wal_state_and_accepts_new_writes() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let database_dir = source.join("hermes/db");
        fs::create_dir_all(&database_dir).unwrap();
        let database = database_dir.join("chat.sqlite3");
        let connection = Connection::open(&database).unwrap();
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .unwrap();
        connection
            .pragma_update(None, "wal_autocheckpoint", 0)
            .unwrap();
        connection
            .execute_batch(
                "CREATE TABLE chats (id INTEGER PRIMARY KEY, body TEXT NOT NULL);
                 INSERT INTO chats (body) VALUES ('before interruption'), ('during interruption');",
            )
            .unwrap();
        assert!(database.with_extension("sqlite3-wal").is_file());
        assert_eq!(
            source_manifest(&source).unwrap(),
            source_manifest(&source).unwrap()
        );

        let archive = temp.path().join("retirement.zip");
        create_recovery_zip(&source, &archive, &context()).unwrap();
        drop(connection);
        let restored = temp.path().join("restored");
        restore_recovery_zip(&archive, &restored, Some(&context())).unwrap();

        let restored_connection =
            Connection::open(restored.join("hermes/db/chat.sqlite3")).unwrap();
        let before: i64 = restored_connection
            .query_row("SELECT COUNT(*) FROM chats", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, 2);
        restored_connection
            .execute("INSERT INTO chats (body) VALUES ('after restore')", [])
            .unwrap();
        let after: i64 = restored_connection
            .query_row("SELECT COUNT(*) FROM chats", [], |row| row.get(0))
            .unwrap();
        assert_eq!(after, 3, "restored chat state must remain writable");
    }

    #[test]
    fn rejects_corrupt_and_truncated_zip() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        fixture(&source);
        let archive = temp.path().join("retirement.zip");
        create_recovery_zip(&source, &archive, &context()).unwrap();

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&archive)
            .unwrap();
        let length = file.metadata().unwrap().len();
        file.seek(SeekFrom::Start(length / 2)).unwrap();
        file.write_all(b"CORRUPT").unwrap();
        assert!(verify_recovery_zip(&archive, Some(&context())).is_err());

        file.set_len(24).unwrap();
        assert!(verify_recovery_zip(&archive, Some(&context())).is_err());
    }

    #[test]
    fn rejects_context_mismatch_and_nonempty_restore_target() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        fixture(&source);
        let archive = temp.path().join("retirement.zip");
        create_recovery_zip(&source, &archive, &context()).unwrap();

        let mut wrong = context();
        wrong.request_id = "rcr_wrong".to_string();
        assert!(verify_recovery_zip(&archive, Some(&wrong)).is_err());

        let target = temp.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("existing"), b"do not overwrite").unwrap();
        assert!(restore_recovery_zip(&archive, &target, Some(&context())).is_err());
        assert_eq!(
            fs::read(target.join("existing")).unwrap(),
            b"do not overwrite"
        );
    }

    #[test]
    fn rejects_unsafe_paths_symlink_escapes_and_unsupported_types() {
        assert!(normalized_relative_path(Path::new("../escape")).is_err());
        assert!(normalized_relative_path(Path::new("/absolute")).is_err());
        assert!(validate_symlink_target(Path::new("link"), "../escape").is_err());
        assert!(validate_symlink_target(Path::new("dir/link"), "../../escape").is_err());

        #[cfg(unix)]
        {
            use std::os::unix::net::UnixListener;
            let temp = tempdir().unwrap();
            let source = temp.path().join("source");
            fs::create_dir(&source).unwrap();
            let _listener = UnixListener::bind(source.join("socket")).unwrap();
            assert!(source_manifest(&source).is_err());
        }
    }
}
