//! `/usr/local/bin` shims: one-line exec scripts for every file in
//! `<current>/bin/`, so `container exec finite-agentd ...` keeps working
//! across generations. Shims exec through the `current` symlink (never a
//! versioned path), are rewritten on every boot and flip, and stale shims —
//! recognizable by their marker line — are removed.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::ShellError;

pub const SHIM_MARKER: &str = "# finite-shell shim";

/// Write shims for every entry in `current_bin_dir` into `shim_dir`,
/// exec-ing through `exec_bin_dir` (the `/data/generations/current/bin`
/// symlinked path). Returns the shim names written. Existing files in
/// `shim_dir` are only ever replaced or removed if they carry the shim
/// marker — the shell never clobbers foreign binaries.
pub fn write_shims(
    shim_dir: &Path,
    current_bin_dir: &Path,
    exec_bin_dir: &Path,
) -> Result<Vec<String>, ShellError> {
    fs::create_dir_all(shim_dir)?;
    let mut written = Vec::new();
    if current_bin_dir.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(current_bin_dir)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let file_type = entry.file_type()?;
            if !(file_type.is_file() || file_type.is_symlink()) {
                continue;
            }
            let shim_path = shim_dir.join(&name);
            if shim_path.exists() && !is_shim(&shim_path) {
                return Err(ShellError::Contract(format!(
                    "refusing to overwrite non-shim file {} with a shim",
                    shim_path.display()
                )));
            }
            // One-shot invocations (`container exec finite-agentd ...`) do not
            // inherit the env the shell gives its supervised child, so the
            // shim must carry the payload root itself or payload-relative
            // defaults fall back to paths that no longer exist in the image.
            let payload_root = exec_bin_dir.parent().ok_or_else(|| {
                ShellError::Contract(format!(
                    "shim exec dir {} has no parent",
                    exec_bin_dir.display()
                ))
            })?;
            let body = format!(
                "#!/bin/sh\n{SHIM_MARKER}\nexport FINITE_PAYLOAD_ROOT=\"{}\"\nexec \"{}/{name}\" \"$@\"\n",
                payload_root.display(),
                exec_bin_dir.display()
            );
            let mut temporary = tempfile::NamedTempFile::new_in(shim_dir)?;
            temporary.write_all(body.as_bytes())?;
            temporary
                .as_file()
                .set_permissions(fs::Permissions::from_mode(0o755))?;
            temporary
                .persist(&shim_path)
                .map_err(|error| ShellError::Io(error.error))?;
            written.push(name);
        }
    }

    // Remove stale shims for binaries the current generation no longer ships.
    for entry in fs::read_dir(shim_dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if written.contains(&name) {
            continue;
        }
        let path = entry.path();
        if is_shim(&path) {
            fs::remove_file(&path)?;
        }
    }
    Ok(written)
}

fn is_shim(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|contents| contents.lines().nth(1) == Some(SHIM_MARKER))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shims_are_written_removed_when_stale_and_never_clobber_foreign_files() {
        let dir = tempfile::tempdir().unwrap();
        let shim_dir = dir.path().join("usr-local-bin");
        let bin_v1 = dir.path().join("gen-v1/bin");
        fs::create_dir_all(&bin_v1).unwrap();
        fs::write(bin_v1.join("finite-agentd"), "#!/bin/sh\n").unwrap();
        fs::write(bin_v1.join("fbrain"), "#!/bin/sh\n").unwrap();
        fs::create_dir_all(&shim_dir).unwrap();
        fs::write(shim_dir.join("python3"), "real interpreter").unwrap();

        let exec_dir = Path::new("/data/generations/current/bin");
        let written = write_shims(&shim_dir, &bin_v1, exec_dir).unwrap();
        assert_eq!(written, ["fbrain", "finite-agentd"]);
        let shim = fs::read_to_string(shim_dir.join("finite-agentd")).unwrap();
        assert_eq!(
            shim,
            "#!/bin/sh\n# finite-shell shim\nexport FINITE_PAYLOAD_ROOT=\"/data/generations/current\"\nexec \"/data/generations/current/bin/finite-agentd\" \"$@\"\n"
        );
        let mode = fs::metadata(shim_dir.join("finite-agentd"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);

        // v2 drops fbrain; its shim must disappear, the foreign python3
        // must survive.
        let bin_v2 = dir.path().join("gen-v2/bin");
        fs::create_dir_all(&bin_v2).unwrap();
        fs::write(bin_v2.join("finite-agentd"), "#!/bin/sh\n").unwrap();
        let written = write_shims(&shim_dir, &bin_v2, exec_dir).unwrap();
        assert_eq!(written, ["finite-agentd"]);
        assert!(!shim_dir.join("fbrain").exists(), "stale shim removed");
        assert_eq!(
            fs::read_to_string(shim_dir.join("python3")).unwrap(),
            "real interpreter",
            "foreign files are never treated as shims"
        );

        // A generation shipping a name that collides with a foreign binary
        // is refused rather than clobbering it.
        fs::write(bin_v2.join("python3"), "#!/bin/sh\n").unwrap();
        let error = write_shims(&shim_dir, &bin_v2, exec_dir).unwrap_err();
        assert!(matches!(error, ShellError::Contract(_)));
        assert_eq!(
            fs::read_to_string(shim_dir.join("python3")).unwrap(),
            "real interpreter"
        );
    }
}
