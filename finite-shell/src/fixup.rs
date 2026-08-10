//! Post-stage fixup: rewrite the Hermes venv's absolute paths for the
//! generation's real on-disk location. A small pure function over the
//! unpacked tree — it runs after every unpack (seed included) and is
//! idempotent.
//!
//! Three rewrites, nothing else:
//! 1. `hermes-venv/pyvenv.cfg` `home` points at the shell python's directory
//!    (the interpreter is shell-owned, not payload-owned).
//! 2. Shebang lines in `hermes-venv/bin/*` scripts point at
//!    `<generation>/hermes-venv/bin/python`.
//! 3. `hermes-venv/bin/python*` entries become symlinks to the shell python.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::ShellError;

pub const VENV_DIR_NAME: &str = "hermes-venv";

/// What the fixup changed, for logging and assertions.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FixupReport {
    pub pyvenv_home_set: bool,
    pub shebangs_rewritten: Vec<String>,
    pub python_links_repointed: Vec<String>,
}

/// Fix the venv inside the unpacked tree at `tree_root`.
///
/// `generation_real_path` is where this generation really lives (its final
/// `/data/generations/<version>` path — shebangs must not go through the
/// `current` symlink, or they would silently retarget on the next flip).
/// `shell_python` is the shell-owned interpreter (e.g.
/// `/usr/local/bin/python3`). Trees without a `hermes-venv` are untouched.
pub fn apply_venv_fixup(
    tree_root: &Path,
    generation_real_path: &Path,
    shell_python: &Path,
) -> Result<FixupReport, ShellError> {
    let mut report = FixupReport::default();
    let venv = tree_root.join(VENV_DIR_NAME);
    if !venv.is_dir() {
        return Ok(report);
    }

    let venv_python = generation_real_path.join(VENV_DIR_NAME).join("bin/python");
    let shell_python_dir = shell_python.parent().ok_or_else(|| {
        ShellError::Config(format!(
            "shell python {} has no parent directory",
            shell_python.display()
        ))
    })?;

    // 1. pyvenv.cfg home.
    let pyvenv_cfg = venv.join("pyvenv.cfg");
    if pyvenv_cfg.is_file() {
        let contents = fs::read_to_string(&pyvenv_cfg)?;
        let desired_home = format!("home = {}", shell_python_dir.display());
        let mut changed = false;
        let rewritten: Vec<String> = contents
            .lines()
            .map(|line| {
                let key = line.split('=').next().unwrap_or("").trim();
                if key == "home" && line.trim() != desired_home {
                    changed = true;
                    desired_home.clone()
                } else {
                    line.to_owned()
                }
            })
            .collect();
        if changed {
            fs::write(&pyvenv_cfg, format!("{}\n", rewritten.join("\n")))?;
            report.pyvenv_home_set = true;
        }
    }

    // 2 + 3. bin entries.
    let bin_dir = venv.join("bin");
    if !bin_dir.is_dir() {
        return Ok(report);
    }
    let mut entries: Vec<_> = fs::read_dir(&bin_dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let metadata = fs::symlink_metadata(&path)?;
        if name.starts_with("python") {
            // The interpreter entries always end up as symlinks to the
            // shell's python, whether the payload shipped them as stub
            // files or as relative symlinks.
            let already_correct = metadata.file_type().is_symlink()
                && fs::read_link(&path).is_ok_and(|target| target == shell_python);
            if !already_correct {
                if metadata.file_type().is_symlink() || metadata.is_file() {
                    fs::remove_file(&path)?;
                } else {
                    continue;
                }
                std::os::unix::fs::symlink(shell_python, &path)?;
                report.python_links_repointed.push(name);
            }
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let bytes = fs::read(&path)?;
        if !bytes.starts_with(b"#!") {
            continue;
        }
        let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') else {
            continue;
        };
        let Ok(first_line) = std::str::from_utf8(&bytes[..newline]) else {
            continue;
        };
        let desired = format!("#!{}", venv_python.display());
        if !first_line.contains("python") || first_line == desired {
            continue;
        }
        let mut rewritten = desired.into_bytes();
        rewritten.extend_from_slice(&bytes[newline..]);
        let permissions = metadata.permissions();
        fs::write(&path, rewritten)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(permissions.mode()))?;
        report.shebangs_rewritten.push(name);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_venv_tree(root: &Path) {
        let bin = root.join("hermes-venv/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(
            root.join("hermes-venv/pyvenv.cfg"),
            "home = /opt/old-build/python/bin\nversion = 3.11.9\n",
        )
        .unwrap();
        fs::write(bin.join("python3.11"), "stub interpreter").unwrap();
        std::os::unix::fs::symlink("python3.11", bin.join("python")).unwrap();
        std::os::unix::fs::symlink("python3.11", bin.join("python3")).unwrap();
        fs::write(
            bin.join("hermes"),
            "#!/opt/old-build/hermes-venv/bin/python\nprint('hi')\n",
        )
        .unwrap();
        fs::set_permissions(bin.join("hermes"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(bin.join("activate"), "# shell script, no shebang\n").unwrap();
        fs::write(bin.join("native-tool"), b"\x7fELF binary bytes").unwrap();
    }

    #[test]
    fn fixup_rewrites_home_shebangs_and_python_links_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("tree");
        build_venv_tree(&tree);
        let generation = Path::new("/data/generations/v2");
        let shell_python = Path::new("/usr/local/bin/python3");

        let report = apply_venv_fixup(&tree, generation, shell_python).unwrap();
        assert!(report.pyvenv_home_set);
        assert_eq!(report.shebangs_rewritten, ["hermes"]);
        assert_eq!(
            report.python_links_repointed,
            ["python", "python3", "python3.11"]
        );

        let cfg = fs::read_to_string(tree.join("hermes-venv/pyvenv.cfg")).unwrap();
        assert!(cfg.contains("home = /usr/local/bin"), "{cfg}");
        assert!(cfg.contains("version = 3.11.9"), "other keys survive");

        let hermes = fs::read_to_string(tree.join("hermes-venv/bin/hermes")).unwrap();
        assert!(
            hermes.starts_with("#!/data/generations/v2/hermes-venv/bin/python\n"),
            "shebang must use the generation's real path, not `current`: {hermes}"
        );
        assert!(hermes.ends_with("print('hi')\n"));
        let mode = fs::metadata(tree.join("hermes-venv/bin/hermes"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755, "executable bits survive the rewrite");

        for name in ["python", "python3", "python3.11"] {
            let link = tree.join("hermes-venv/bin").join(name);
            assert!(
                fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(fs::read_link(&link).unwrap(), shell_python);
        }
        assert_eq!(
            fs::read_to_string(tree.join("hermes-venv/bin/activate")).unwrap(),
            "# shell script, no shebang\n",
            "non-shebang files are untouched"
        );
        assert_eq!(
            fs::read(tree.join("hermes-venv/bin/native-tool")).unwrap(),
            b"\x7fELF binary bytes",
            "binaries are untouched"
        );

        // Idempotent: a second run reports nothing to do.
        let second = apply_venv_fixup(&tree, generation, shell_python).unwrap();
        assert_eq!(second, FixupReport::default());
    }

    #[test]
    fn a_tree_without_a_venv_is_untouched() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("bin")).unwrap();
        fs::write(dir.path().join("bin/finite-agentd"), "#!/bin/sh\n").unwrap();
        let report = apply_venv_fixup(
            dir.path(),
            Path::new("/data/generations/v1"),
            Path::new("/usr/local/bin/python3"),
        )
        .unwrap();
        assert_eq!(report, FixupReport::default());
        assert_eq!(
            fs::read_to_string(dir.path().join("bin/finite-agentd")).unwrap(),
            "#!/bin/sh\n",
            "fixup never touches anything outside hermes-venv"
        );
    }
}
