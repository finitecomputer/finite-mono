use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

use crate::CliError;

const CONTROL_DIR_NAME: &str = ".finitebrain";
const ENCRYPTED_SYNC_DIR_NAME: &str = "encrypted-sync";
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
    validate_private_directory(root)?;
    let control_dir = root.join(CONTROL_DIR_NAME);
    validate_private_directory(&control_dir)?;
    validate_managed_entries(&control_dir)
}

pub(crate) fn repair_private_working_tree(
    root: &Path,
) -> Result<WorkingTreeRepairReport, CliError> {
    let root_metadata = fs::symlink_metadata(root)?;
    reject_symlink_or_wrong_kind(root, &root_metadata, true)?;
    let control_dir = root.join(CONTROL_DIR_NAME);
    let control_metadata = fs::symlink_metadata(&control_dir)?;
    reject_symlink_or_wrong_kind(&control_dir, &control_metadata, true)?;

    let mut directories = vec![control_dir.clone()];
    let mut files = Vec::new();
    collect_managed_entries(&control_dir, &mut directories, &mut files)?;

    set_private_directory_permissions(root)?;
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
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(managed_symlink_error(&path));
        }
        if metadata.is_dir() {
            directories.push(path.clone());
            collect_managed_entries(&path, directories, files)?;
        } else if metadata.is_file() {
            files.push(path);
        } else {
            return Err(CliError::InsecureWorkingTree {
                path,
                reason: "Finite-managed path is neither a regular file nor a directory".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_managed_entries(directory: &Path) -> Result<(), CliError> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(managed_symlink_error(&path));
        }
        if metadata.is_dir() {
            validate_private_directory_metadata(&path, &metadata)?;
            validate_managed_entries(&path)?;
        } else if metadata.is_file() {
            validate_private_file(&path, &metadata)?;
        } else {
            return Err(CliError::InsecureWorkingTree {
                path,
                reason: "Finite-managed path is neither a regular file nor a directory".to_owned(),
            });
        }
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
    Ok(())
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
        return Err(CliError::InsecureWorkingTree {
            path: path.to_path_buf(),
            reason: format!("directory mode is {mode:04o}; expected 0700"),
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
        return Err(CliError::InsecureWorkingTree {
            path: path.to_path_buf(),
            reason: format!("control file mode is {mode:04o}; expected 0600"),
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

fn create_private_directory_if_missing(path: &Path) -> Result<(), CliError> {
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
fn set_private_file_permissions(path: &Path) -> Result<(), CliError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> Result<(), CliError> {
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
