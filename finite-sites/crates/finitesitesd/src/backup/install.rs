//! Publish a verified Recovery Set as one durable, no-replace directory rename.

use std::fs::{self, File};
use std::path::Path;

use super::{BackupError, limits};

pub(super) fn publish(staging: tempfile::TempDir, target: &Path) -> Result<(), BackupError> {
    sync_tree(staging.path())?;
    rename_no_replace(staging.path(), target)
}

fn sync_tree(root: &Path) -> Result<(), BackupError> {
    let mut pending = vec![(root.to_path_buf(), 0, false)];
    let mut entries = 1;
    // Count entries before queuing them: Git expansion is independent of the
    // manifest size. Postorder sync persists children before their directories.
    while let Some((path, depth, children_synced)) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            if children_synced {
                File::open(&path)?.sync_all()?;
            } else {
                pending.push((path.clone(), depth, true));
                for entry in fs::read_dir(&path)? {
                    if entries >= limits::MAX_BACKUP_RESTORE_TREE_ENTRIES
                        || depth >= limits::MAX_BACKUP_RESTORE_TREE_DEPTH
                    {
                        return Err(BackupError::Invalid(
                            "restore tree exceeds entry or depth limit",
                        ));
                    }
                    entries += 1;
                    pending.push((entry?.path(), depth + 1, false));
                }
            }
        } else if metadata.is_file() {
            File::open(&path)?.sync_all()?;
        } else {
            return Err(BackupError::Invalid(
                "unexpected file type in restore staging",
            ));
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rename_no_replace(source: &Path, target: &Path) -> Result<(), BackupError> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let parent = target
        .parent()
        .ok_or(BackupError::Invalid("destination has no parent"))?
        .canonicalize()?;
    if source
        .parent()
        .map(Path::canonicalize)
        .transpose()?
        .as_ref()
        != Some(&parent)
    {
        return Err(BackupError::Invalid(
            "restore staging must share destination parent",
        ));
    }
    let name = |path: &Path| {
        let name = path
            .file_name()
            .ok_or(BackupError::Invalid("restore path has no name"))?;
        CString::new(name.as_bytes()).map_err(|_| BackupError::Invalid("restore path contains NUL"))
    };
    let source = name(source)?;
    let target = name(target)?;
    // Pin both rename endpoints and the final sync to the same directory.
    let parent = File::open(parent)?;
    let fd = parent.as_raw_fd();
    // SAFETY: the directory descriptor and NUL-terminated names remain valid
    // throughout the syscall; the kernel atomically rejects any existing target.
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            fd,
            source.as_ptr(),
            fd,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    // SAFETY: the same descriptor/name lifetimes and no-replace contract apply.
    #[cfg(target_os = "macos")]
    let result =
        unsafe { libc::renameatx_np(fd, source.as_ptr(), fd, target.as_ptr(), libc::RENAME_EXCL) };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    parent.sync_all().map_err(|error| {
        BackupError::Io(std::io::Error::new(
            error.kind(),
            format!("restore published but destination parent sync failed: {error}"),
        ))
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn rename_no_replace(_source: &Path, _target: &Path) -> Result<(), BackupError> {
    Err(BackupError::Invalid(
        "atomic restore publication requires Linux or macOS",
    ))
}
