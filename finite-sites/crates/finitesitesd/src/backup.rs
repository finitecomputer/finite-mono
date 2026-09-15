//! Incremental local Recovery Points. Transport is separate from capture;
//! completion is committed only after every referenced object is durable.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path};

use finitesites_proto::{hex, limits};
use finitesites_store::recovery;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

mod git;
mod install;

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("backup IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("backup registry: {0}")]
    Registry(#[from] finitesites_store::StoreError),
    #[error("backup manifest: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid recovery point: {0}")]
    Invalid(&'static str),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub id: String,
    pub captured_at: u64,
    pub new_objects: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    sha256: String,
    size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: u32,
    captured_at: u64,
    entries: Vec<Entry>,
    projects: Vec<git::ProjectArchive>,
}

pub fn capture(data: &Path, repository: &Path, now: u64) -> Result<Receipt, BackupError> {
    let data = data.canonicalize()?;
    let repository = if repository.try_exists()? {
        repository.canonicalize()?
    } else {
        repository
            .parent()
            .ok_or(BackupError::Invalid("repository has no parent"))?
            .canonicalize()?
            .join(
                repository
                    .file_name()
                    .ok_or(BackupError::Invalid("repository has no name"))?,
            )
    };
    if repository.starts_with(&data) || data.starts_with(&repository) {
        return Err(BackupError::Invalid(
            "source and backup repository must be separate",
        ));
    }
    private_dir(&repository)?;
    private_dir(&repository.join("objects"))?;
    private_dir(&repository.join("points"))?;
    let staging = tempfile::tempdir_in(&repository)?;
    let seed = staging.path().join("seed.db");
    File::create_new(&seed)?;
    let seed_inventory = recovery::capture(&data.join("registry.db"), &seed)?;
    let mut observed = std::collections::BTreeMap::new();
    let mut observed_items = seed_inventory.projects.len();
    let mut observed_bytes = 0;
    for project_id in &seed_inventory.projects {
        let state = git::observe(&data, project_id)?;
        if let Some(state) = &state {
            let (items, bytes) = state.size();
            observed_items += items;
            observed_bytes += bytes;
        }
        if observed_items > limits::MAX_BACKUP_OBJECTS as usize
            || observed_bytes > limits::MAX_BACKUP_MANIFEST_BYTES
        {
            return Err(BackupError::Invalid("oversized manifest inventory"));
        }
        observed.insert(project_id.clone(), state);
    }
    let registry = staging.path().join("registry.db");
    File::create_new(&registry)?;
    let inventory = recovery::capture(&data.join("registry.db"), &registry)?;
    if inventory.projects != seed_inventory.projects {
        return Err(BackupError::Invalid(
            "project inventory changed; retry capture",
        ));
    }
    let mut entries = Vec::new();
    let mut new_objects = 0;
    let mut projects = Vec::new();
    for project_id in &inventory.projects {
        projects.push(git::capture(
            &data,
            project_id,
            &inventory.git_events,
            staging.path(),
            &repository,
            &mut entries,
            &mut new_objects,
        )?);
        if !projects
            .last()
            .expect("project was captured")
            .matches(&observed[project_id])
        {
            return Err(BackupError::Invalid(
                "captured Git refs differ from checkpoint observation",
            ));
        }
        if git::observe(&data, project_id)? != observed[project_id] {
            return Err(BackupError::Invalid(
                "Git refs changed since registry capture; retry",
            ));
        }
    }
    add_entry(
        &repository,
        &registry,
        "registry.db",
        &mut entries,
        &mut new_objects,
    )?;
    let cookie = read_bounded(&data.join("cookie-secret"), 65)?;
    if !hex::is_hex32(std::str::from_utf8(&cookie).unwrap_or("").trim()) {
        return Err(BackupError::Invalid("invalid cookie secret"));
    }
    add_entry(
        &repository,
        &data.join("cookie-secret"),
        "cookie-secret",
        &mut entries,
        &mut new_objects,
    )?;
    if entries.last().expect("cookie entry was added").sha256 != digest(&cookie) {
        return Err(BackupError::Invalid(
            "cookie secret changed during capture; retry",
        ));
    }
    for (hash, size) in inventory.blobs {
        let path = format!("blobs/{}/{}/{}", &hash[..2], &hash[2..4], hash);
        add_entry(
            &repository,
            &data.join(&path),
            &path,
            &mut entries,
            &mut new_objects,
        )?;
        let entry = entries.last().expect("entry was added");
        if entry.sha256 != hash || entry.size != size {
            return Err(BackupError::Invalid("source blob hash or size mismatch"));
        }
    }
    // Optimistic validation avoids stopping writers. A busy generation can
    // retry; its already-written immutable objects remain reusable.
    let after = staging.path().join("after.db");
    File::create_new(&after)?;
    let after = recovery::capture(&data.join("registry.db"), &after)?;
    if after.git_events != inventory.git_events || after.projects != inventory.projects {
        return Err(BackupError::Invalid(
            "Git registry changed during capture; retry",
        ));
    }
    for project_id in &inventory.projects {
        if git::observe(&data, project_id)? != observed[project_id] {
            return Err(BackupError::Invalid(
                "Git refs changed during capture; retry",
            ));
        }
    }
    if read_bounded(&data.join("cookie-secret"), 65)? != cookie {
        return Err(BackupError::Invalid(
            "cookie secret changed during capture; retry",
        ));
    }
    let manifest = Manifest {
        format: 1,
        captured_at: now,
        entries,
        projects,
    };
    validate_manifest(&manifest)?;
    let mut encoded = ManifestBuffer(Vec::new());
    serde_json::to_writer(&mut encoded, &manifest)?;
    let bytes = encoded.0;
    let id = digest(&bytes);
    // Object and point directories must survive a crash too, including a
    // repository created by this capture. Persist their parent entries before
    // publishing the first selectable point.
    File::open(&repository)?.sync_all()?;
    File::open(repository.parent().expect("repository parent was resolved"))?.sync_all()?;
    persist(&repository.join("points").join(&id), &bytes)?;
    Ok(Receipt {
        id,
        captured_at: now,
        new_objects,
    })
}

pub fn restore(repository: &Path, id: &str, target: &Path) -> Result<(), BackupError> {
    if !hex::is_hex32(id) || target.try_exists()? {
        return Err(BackupError::Invalid(
            "invalid point or destination already exists",
        ));
    }
    let bytes = read_bounded(
        &repository.join("points").join(id),
        limits::MAX_BACKUP_MANIFEST_BYTES,
    )?;
    if digest(&bytes) != id {
        return Err(BackupError::Invalid("manifest checksum mismatch"));
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    validate_manifest(&manifest)?;
    let staging = tempfile::tempdir_in(
        target
            .parent()
            .ok_or(BackupError::Invalid("destination has no parent"))?,
    )?;
    let mut paths = std::collections::BTreeSet::new();
    for entry in &manifest.entries {
        validate_entry(entry)?;
        if !paths.insert(&entry.path) {
            return Err(BackupError::Invalid("duplicate manifest path"));
        }
        let bytes = read_bounded(
            &repository.join("objects").join(&entry.sha256),
            limits::MAX_BACKUP_OBJECT_BYTES,
        )?;
        if digest(&bytes) != entry.sha256 || bytes.len() as u64 != entry.size {
            return Err(BackupError::Invalid("object checksum or size mismatch"));
        }
        let path = staging.path().join(&entry.path);
        private_dir(path.parent().expect("staged file has parent"))?;
        persist(&path, &bytes)?;
    }
    let inventory = recovery::inspect(&staging.path().join("registry.db"))?;
    for (hash, size) in &inventory.blobs {
        let path = format!("blobs/{}/{}/{}", &hash[..2], &hash[2..4], hash);
        if !manifest
            .entries
            .iter()
            .any(|entry| entry.path == path && entry.sha256 == *hash && entry.size == *size)
        {
            return Err(BackupError::Invalid(
                "manifest omits a required registry blob",
            ));
        }
    }
    let mut project_ids = manifest
        .projects
        .iter()
        .map(|project| project.project_id.clone())
        .collect::<Vec<_>>();
    project_ids.sort();
    if project_ids != inventory.projects {
        return Err(BackupError::Invalid(
            "manifest project inventory differs from registry",
        ));
    }
    for project in &manifest.projects {
        git::restore(staging.path(), project, &inventory.git_events)?;
    }
    let cookie = read_bounded(&staging.path().join("cookie-secret"), 65)?;
    if !hex::is_hex32(std::str::from_utf8(&cookie).unwrap_or("").trim()) {
        return Err(BackupError::Invalid("cookie secret is missing or invalid"));
    }
    if staging.path().join("git-bundles").try_exists()? {
        fs::remove_dir_all(staging.path().join("git-bundles"))?;
    }
    install::publish(staging, target)
}

fn validate_entry(entry: &Entry) -> Result<(), BackupError> {
    if !hex::is_hex32(&entry.sha256)
        || entry.size > limits::MAX_BACKUP_OBJECT_BYTES
        || entry.path.is_empty()
        || Path::new(&entry.path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(BackupError::Invalid("invalid manifest entry"));
    }
    let blob_path = format!(
        "blobs/{}/{}/{}",
        &entry.sha256[..2],
        &entry.sha256[2..4],
        entry.sha256
    );
    let bundle = entry
        .path
        .strip_prefix("git-bundles/")
        .and_then(|name| name.strip_suffix(".bundle"));
    if entry.path != "registry.db"
        && entry.path != "cookie-secret"
        && entry.path != blob_path
        && !bundle.is_some_and(|id| {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
    {
        return Err(BackupError::Invalid("unexpected recovery path"));
    }
    Ok(())
}

fn validate_manifest(manifest: &Manifest) -> Result<(), BackupError> {
    let items = manifest
        .projects
        .iter()
        .fold(manifest.entries.len(), |sum, project| {
            sum.saturating_add(project.items())
        });
    if manifest.format != 1 || items > limits::MAX_BACKUP_OBJECTS as usize {
        return Err(BackupError::Invalid(
            "unsupported format or oversized manifest",
        ));
    }
    // Count a conservative upper bound, including every path's parents even
    // when shared. Apply the install budget before publishing a recovery point.
    let mut tree_entries = 1;
    for entry in &manifest.entries {
        tree_entries += Path::new(&entry.path).components().count() as u64;
    }
    for project in &manifest.projects {
        tree_entries += project.tree_entries()?;
    }
    if tree_entries > u64::from(limits::MAX_BACKUP_RESTORE_TREE_ENTRIES) {
        return Err(BackupError::Invalid("manifest exceeds restore tree budget"));
    }
    Ok(())
}

struct ManifestBuffer(Vec<u8>);

impl Write for ManifestBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) as u64 > limits::MAX_BACKUP_MANIFEST_BYTES {
            return Err(std::io::Error::other("oversized manifest"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn add_entry(
    repository: &Path,
    source: &Path,
    path: &str,
    entries: &mut Vec<Entry>,
    count: &mut u32,
) -> Result<(), BackupError> {
    if entries.len() >= limits::MAX_BACKUP_OBJECTS as usize {
        return Err(BackupError::Invalid("oversized manifest inventory"));
    }
    let bytes = read_bounded(source, limits::MAX_BACKUP_OBJECT_BYTES)?;
    let sha256 = digest(&bytes);
    if persist(&repository.join("objects").join(&sha256), &bytes)? {
        *count += 1;
    }
    entries.push(Entry {
        path: path.to_string(),
        sha256,
        size: bytes.len() as u64,
    });
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(&Sha256::digest(bytes))
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, BackupError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(BackupError::Invalid("object is not a bounded regular file"));
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(BackupError::Invalid("object exceeds size limit"));
    }
    Ok(bytes)
}

fn persist(path: &Path, bytes: &[u8]) -> Result<bool, BackupError> {
    if path.try_exists()? {
        if read_bounded(path, limits::MAX_BACKUP_OBJECT_BYTES)? != bytes {
            return Err(BackupError::Invalid("existing immutable object differs"));
        }
        return Ok(false);
    }
    let parent = path
        .parent()
        .ok_or(BackupError::Invalid("object has no parent"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(path).map_err(|error| error.error)?;
    File::open(parent)?.sync_all()?;
    Ok(true)
}

fn private_dir(path: &Path) -> Result<(), BackupError> {
    fs::create_dir_all(path)?;
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(BackupError::Invalid("symlink directory"));
    }
    set_private_dir(path)
}

fn set_private_dir(path: &Path) -> Result<(), BackupError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
