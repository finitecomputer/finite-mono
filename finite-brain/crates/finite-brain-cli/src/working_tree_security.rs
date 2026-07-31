use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

use crate::CliError;

const CONTROL_DIR_NAME: &str = ".finitebrain";
const ENCRYPTED_SYNC_DIR_NAME: &str = "encrypted-sync";
const MAX_MANAGED_ENTRY_COUNT: usize = 10_000;
const MAX_MANAGED_DEPTH: usize = 32;
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkingTreeRepairReport {
    pub(crate) state: &'static str,
    pub(crate) working_tree_path: String,
    pub(crate) repaired_directories: usize,
    pub(crate) repaired_files: usize,
}

pub(crate) fn initialize_private_working_tree(root: &Path) -> Result<(), CliError> {
    let control_dir = root.join(CONTROL_DIR_NAME);
    match fs::symlink_metadata(root) {
        Ok(metadata) => {
            reject_symlink_or_wrong_kind(root, &metadata, true)?;
            if control_dir_exists(&control_dir)? {
                validate_private_working_tree(root)?;
            } else {
                set_private_directory_permissions(root)?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_private_directory_all(root)?;
        }
        Err(error) => return Err(error.into()),
    }

    create_private_directory_if_missing(&control_dir)?;
    create_private_directory_if_missing(&control_dir.join(ENCRYPTED_SYNC_DIR_NAME))?;
    validate_private_working_tree(root)
}

pub(crate) fn validate_private_working_tree(root: &Path) -> Result<(), CliError> {
    validate_working_tree_managed_structure(root)?;
    validate_private_directory(root)?;
    let control_dir = root.join(CONTROL_DIR_NAME);
    validate_private_directory(&control_dir)?;
    validate_managed_entries(&control_dir)
}

pub(crate) fn validate_working_tree_managed_structure(root: &Path) -> Result<(), CliError> {
    let root_metadata = fs::symlink_metadata(root)?;
    reject_symlink_or_wrong_kind(root, &root_metadata, true)?;
    validate_owner_traversal_permission(root, &root_metadata)?;
    let control_dir = root.join(CONTROL_DIR_NAME);
    let control_metadata = fs::symlink_metadata(&control_dir)?;
    reject_symlink_or_wrong_kind(&control_dir, &control_metadata, true)?;
    validate_owner_traversal_permission(&control_dir, &control_metadata)?;
    validate_managed_entry_structure(&control_dir)
}

pub(crate) fn repair_private_working_tree(
    root: &Path,
) -> Result<WorkingTreeRepairReport, CliError> {
    let root_metadata = fs::symlink_metadata(root)?;
    reject_symlink_or_wrong_kind(root, &root_metadata, true)?;
    let control_dir = root.join(CONTROL_DIR_NAME);
    let control_metadata = fs::symlink_metadata(&control_dir)?;
    reject_symlink_or_wrong_kind(&control_dir, &control_metadata, true)?;

    set_private_directory_permissions(root)?;
    set_private_directory_permissions(&control_dir)?;

    let mut directories = vec![control_dir.clone()];
    let mut files = Vec::new();
    collect_managed_entries(&control_dir, &mut directories, &mut files)?;

    for directory in &directories {
        set_private_directory_permissions(directory)?;
    }
    for file in &files {
        set_private_file_permissions(file)?;
    }
    let encrypted_sync = control_dir.join(ENCRYPTED_SYNC_DIR_NAME);
    if !control_dir_exists(&encrypted_sync)? {
        create_private_directory_if_missing(&encrypted_sync)?;
        directories.push(encrypted_sync);
    }
    validate_private_working_tree(root)?;

    Ok(WorkingTreeRepairReport {
        state: "repaired",
        working_tree_path: root.display().to_string(),
        repaired_directories: directories.len() + 1,
        repaired_files: files.len(),
    })
}

pub(crate) fn write_private_file_atomic(path: &Path, body: &[u8]) -> Result<(), CliError> {
    let parent = path.parent().ok_or_else(|| CliError::InsecureWorkingTree {
        path: path.to_path_buf(),
        reason: "Finite-managed control file has no parent directory".to_owned(),
    })?;
    create_private_directory_if_missing(parent)?;
    validate_private_directory(parent)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            reject_symlink_or_wrong_kind(path, &metadata, false)?;
            validate_private_file(path, &metadata)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    write_atomic_platform(path, parent, body)
}

pub(crate) fn write_private_file_atomic_for_migration(
    path: &Path,
    body: &[u8],
) -> Result<(), CliError> {
    let parent = path.parent().ok_or_else(|| CliError::InsecureWorkingTree {
        path: path.to_path_buf(),
        reason: "Finite-managed control file has no parent directory".to_owned(),
    })?;
    let parent_metadata = fs::symlink_metadata(parent)?;
    reject_symlink_or_wrong_kind(parent, &parent_metadata, true)?;
    let target_metadata = fs::symlink_metadata(path)?;
    reject_symlink_or_wrong_kind(path, &target_metadata, false)?;
    write_atomic_platform(path, parent, body)
}

fn control_dir_exists(path: &Path) -> Result<bool, CliError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn collect_managed_entries(
    directory: &Path,
    directories: &mut Vec<PathBuf>,
    files: &mut Vec<PathBuf>,
) -> Result<(), CliError> {
    let mut pending = vec![(directory.to_path_buf(), 0usize)];
    let mut entry_count = 0usize;
    while let Some((current, depth)) = pending.pop() {
        let Some(entries) = read_managed_directory_if_present(&current)? else {
            continue;
        };
        for entry in entries {
            let path = entry?.path();
            entry_count = entry_count.saturating_add(1);
            enforce_managed_traversal_bounds(&path, depth, entry_count)?;
            let Some(metadata) = managed_metadata_if_present(&path)? else {
                continue;
            };
            if metadata.is_dir() {
                reject_symlink_or_wrong_kind(&path, &metadata, true)?;
                set_private_directory_permissions(&path)?;
                directories.push(path.clone());
                pending.push((path, depth + 1));
            } else if metadata.is_file() {
                reject_symlink_or_wrong_kind(&path, &metadata, false)?;
                files.push(path);
            } else if metadata.file_type().is_symlink() {
                return Err(managed_symlink_error(&path));
            } else {
                return Err(CliError::InsecureWorkingTree {
                    path,
                    reason: "Finite-managed path is neither a regular file nor a directory"
                        .to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn validate_managed_entries(directory: &Path) -> Result<(), CliError> {
    let mut pending = vec![(directory.to_path_buf(), 0usize)];
    let mut entry_count = 0usize;
    while let Some((current, depth)) = pending.pop() {
        let Some(entries) = read_managed_directory_if_present(&current)? else {
            continue;
        };
        for entry in entries {
            let path = entry?.path();
            entry_count = entry_count.saturating_add(1);
            enforce_managed_traversal_bounds(&path, depth, entry_count)?;
            let Some(metadata) = managed_metadata_if_present(&path)? else {
                continue;
            };
            if metadata.is_dir() {
                reject_symlink_or_wrong_kind(&path, &metadata, true)?;
                validate_private_directory_metadata(&path, &metadata)?;
                pending.push((path, depth + 1));
            } else if metadata.is_file() {
                validate_private_file(&path, &metadata)?;
            } else if metadata.file_type().is_symlink() {
                return Err(managed_symlink_error(&path));
            } else {
                return Err(CliError::InsecureWorkingTree {
                    path,
                    reason: "Finite-managed path is neither a regular file nor a directory"
                        .to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn validate_managed_entry_structure(directory: &Path) -> Result<(), CliError> {
    let mut pending = vec![(directory.to_path_buf(), 0usize)];
    let mut entry_count = 0usize;
    while let Some((current, depth)) = pending.pop() {
        let Some(entries) = read_managed_directory_if_present(&current)? else {
            continue;
        };
        for entry in entries {
            let path = entry?.path();
            entry_count = entry_count.saturating_add(1);
            enforce_managed_traversal_bounds(&path, depth, entry_count)?;
            let Some(metadata) = managed_metadata_if_present(&path)? else {
                continue;
            };
            if metadata.is_dir() {
                reject_symlink_or_wrong_kind(&path, &metadata, true)?;
                validate_owner_traversal_permission(&path, &metadata)?;
                pending.push((path, depth + 1));
            } else if metadata.is_file() {
                reject_symlink_or_wrong_kind(&path, &metadata, false)?;
                validate_owner_read_permission(&path, &metadata)?;
            } else if metadata.file_type().is_symlink() {
                return Err(managed_symlink_error(&path));
            } else {
                return Err(CliError::InsecureWorkingTree {
                    path,
                    reason: "Finite-managed path is neither a regular file nor a directory"
                        .to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn read_managed_directory_if_present(path: &Path) -> Result<Option<fs::ReadDir>, CliError> {
    match fs::read_dir(path) {
        Ok(entries) => Ok(Some(entries)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn managed_metadata_if_present(path: &Path) -> Result<Option<fs::Metadata>, CliError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn enforce_managed_traversal_bounds(
    path: &Path,
    parent_depth: usize,
    entry_count: usize,
) -> Result<(), CliError> {
    if entry_count > MAX_MANAGED_ENTRY_COUNT {
        return Err(CliError::InsecureWorkingTree {
            path: path.to_path_buf(),
            reason: format!(
                "Finite-managed control state exceeds {MAX_MANAGED_ENTRY_COUNT} entries"
            ),
        });
    }
    if parent_depth >= MAX_MANAGED_DEPTH {
        return Err(CliError::InsecureWorkingTree {
            path: path.to_path_buf(),
            reason: format!("Finite-managed control state exceeds depth {MAX_MANAGED_DEPTH}"),
        });
    }
    Ok(())
}

fn validate_private_directory(path: &Path) -> Result<(), CliError> {
    let metadata = fs::symlink_metadata(path)?;
    reject_symlink_or_wrong_kind(path, &metadata, true)?;
    validate_private_directory_metadata(path, &metadata)
}

fn reject_symlink_or_wrong_kind(
    path: &Path,
    metadata: &fs::Metadata,
    expect_directory: bool,
) -> Result<(), CliError> {
    if metadata.file_type().is_symlink() {
        return Err(managed_symlink_error(path));
    }
    let expected_kind = if expect_directory {
        "directory"
    } else {
        "regular file"
    };
    let matches = if expect_directory {
        metadata.is_dir()
    } else {
        metadata.is_file()
    };
    if !matches {
        return Err(CliError::InsecureWorkingTree {
            path: path.to_path_buf(),
            reason: format!("Finite-managed path must be a {expected_kind}"),
        });
    }
    validate_current_owner(path, metadata)
}

fn managed_symlink_error(path: &Path) -> CliError {
    CliError::InsecureWorkingTree {
        path: path.to_path_buf(),
        reason: "managed symlink is not allowed; remove the link without following it".to_owned(),
    }
}

#[cfg(unix)]
fn validate_private_directory_metadata(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<(), CliError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = metadata.permissions().mode() & 0o7777;
    if mode != 0o700 {
        return Err(CliError::InsecureWorkingTreePermissions {
            path: path.to_path_buf(),
            actual_mode: mode,
            expected_mode: 0o700,
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_directory_metadata(
    _path: &Path,
    _metadata: &fs::Metadata,
) -> Result<(), CliError> {
    Ok(())
}

fn validate_private_file(path: &Path, metadata: &fs::Metadata) -> Result<(), CliError> {
    reject_symlink_or_wrong_kind(path, metadata, false)?;
    validate_private_file_permissions(path, metadata)
}

#[cfg(unix)]
fn validate_private_file_permissions(path: &Path, metadata: &fs::Metadata) -> Result<(), CliError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = metadata.permissions().mode() & 0o7777;
    if mode != 0o600 {
        return Err(CliError::InsecureWorkingTreePermissions {
            path: path.to_path_buf(),
            actual_mode: mode,
            expected_mode: 0o600,
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_file_permissions(
    _path: &Path,
    _metadata: &fs::Metadata,
) -> Result<(), CliError> {
    Ok(())
}

#[cfg(unix)]
fn validate_current_owner(path: &Path, metadata: &fs::Metadata) -> Result<(), CliError> {
    use std::os::unix::fs::MetadataExt;

    validate_owner_ids(path, metadata.uid(), rustix::process::geteuid().as_raw())
}

#[cfg(unix)]
fn validate_owner_ids(path: &Path, owner_uid: u32, effective_uid: u32) -> Result<(), CliError> {
    if owner_uid != effective_uid {
        return Err(CliError::InsecureWorkingTreeOwnership {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_current_owner(_path: &Path, _metadata: &fs::Metadata) -> Result<(), CliError> {
    Ok(())
}

#[cfg(unix)]
fn validate_owner_traversal_permission(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<(), CliError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = metadata.permissions().mode() & 0o7777;
    if mode & 0o500 != 0o500 {
        return Err(CliError::InsecureWorkingTreePermissions {
            path: path.to_path_buf(),
            actual_mode: mode,
            expected_mode: 0o700,
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_owner_traversal_permission(
    _path: &Path,
    _metadata: &fs::Metadata,
) -> Result<(), CliError> {
    Ok(())
}

#[cfg(unix)]
fn validate_owner_read_permission(path: &Path, metadata: &fs::Metadata) -> Result<(), CliError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = metadata.permissions().mode() & 0o7777;
    if mode & 0o400 == 0 {
        return Err(CliError::InsecureWorkingTreePermissions {
            path: path.to_path_buf(),
            actual_mode: mode,
            expected_mode: 0o600,
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_owner_read_permission(_path: &Path, _metadata: &fs::Metadata) -> Result<(), CliError> {
    Ok(())
}

#[cfg(unix)]
fn create_private_directory_all(path: &Path) -> Result<(), CliError> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700).create(path)?;
    set_private_directory_permissions(path)
}

#[cfg(not(unix))]
fn create_private_directory_all(path: &Path) -> Result<(), CliError> {
    fs::create_dir_all(path)?;
    Ok(())
}

pub(crate) fn create_private_directory_if_missing(path: &Path) -> Result<(), CliError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            reject_symlink_or_wrong_kind(path, &metadata, true)?;
            validate_private_directory_metadata(path, &metadata)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| CliError::InsecureWorkingTree {
                path: path.to_path_buf(),
                reason: "Finite-managed directory has no parent".to_owned(),
            })?;
            if parent != path {
                create_private_directory_if_missing(parent)?;
            }
            create_private_directory(path)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> Result<(), CliError> {
    use std::os::unix::fs::DirBuilderExt;

    fs::DirBuilder::new().mode(0o700).create(path)?;
    set_private_directory_permissions(path)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> Result<(), CliError> {
    fs::create_dir(path)?;
    Ok(())
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<(), CliError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> Result<(), CliError> {
    Ok(())
}

#[cfg(unix)]
pub(crate) fn set_private_file_permissions(path: &Path) -> Result<(), CliError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn set_private_file_permissions(_path: &Path) -> Result<(), CliError> {
    Ok(())
}

#[cfg(unix)]
fn write_atomic_platform(path: &Path, parent: &Path, body: &[u8]) -> Result<(), CliError> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CliError::InsecureWorkingTree {
            path: path.to_path_buf(),
            reason: "Finite-managed control filename is not valid UTF-8".to_owned(),
        })?;
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".{file_name}.tmp-{}-{sequence}",
        std::process::id()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(body)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok::<(), CliError>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn managed_control_traversal_rejects_excessive_depth() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("brain");
        initialize_private_working_tree(&root).unwrap();
        let mut directory = root.join(CONTROL_DIR_NAME);
        for index in 0..=MAX_MANAGED_DEPTH {
            directory = directory.join(format!("level-{index}"));
            create_private_directory(&directory).unwrap();
        }

        let error = validate_private_working_tree(&root).unwrap_err();

        assert!(error.to_string().contains("exceeds depth"));
    }

    #[cfg(unix)]
    #[test]
    fn ownership_validation_rejects_a_different_effective_account() {
        let error = validate_owner_ids(Path::new("/brain/.finitebrain"), 1000, 1001).unwrap_err();

        assert!(matches!(
            error,
            CliError::InsecureWorkingTreeOwnership { .. }
        ));
    }
}

#[cfg(not(unix))]
fn write_atomic_platform(path: &Path, parent: &Path, body: &[u8]) -> Result<(), CliError> {
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".finitebrain.tmp-{}-{sequence}",
        std::process::id()
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(body)?;
        file.sync_all()?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(&temporary, path)?;
        Ok::<(), CliError>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
