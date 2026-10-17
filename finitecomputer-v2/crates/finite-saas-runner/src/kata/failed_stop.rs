//! Host evidence that a Kata sandbox's VM is gone after a stop whose CLI
//! failed.
//!
//! Production signature (Lat3, 2026-10-17): `nerdctl stop` logged "force
//! cleanup timed out", Kata then declared the guest agent dead and killed
//! QEMU, and nerdctl exited 1 with "ttrpc: closed". The VM was gone, but the
//! Kata shim stayed alive holding the containerd task. containerd's task
//! record alone cannot tell "VM gone" from "VM still tearing down", so this
//! module reads the host: what can still write a Runtime's durable root is
//! the VM itself (QEMU and virtiofsd), which serves it through Kata's
//! per-sandbox shared directory. The proof requires that no live host process
//! other than the sandbox's own shim names the sandbox, and that Kata removed
//! every per-sandbox host directory. Anything unrecognised blocks the proof.
//!
//! This module only reads `/proc` and directory metadata. It never signals a
//! process and never changes provider or host state.

use std::path::{Path, PathBuf};

/// Kata's per-sandbox host directories, relative to the host root. Kata
/// removes all of them when it tears a sandbox down. The shared directory
/// carries the bind mounts through which virtiofsd serves `/data`.
const KATA_SANDBOX_DIRS: [&str; 4] = [
    "run/kata-containers/shared/sandboxes",
    "run/kata-containers/sandboxes",
    "run/vc/vm",
    "run/vc/sbs",
];

/// Bound on the processes one scan reads, far above a Runner host's count.
const MAX_SCANNED_PROCESSES: usize = 65_536;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct SandboxHostState {
    /// containerd shim processes for the sandbox. A shim that outlived a dead
    /// guest agent holds the containerd task, but serves no guest and does not
    /// share the durable root.
    pub shims: Vec<u32>,
    /// Every other live process naming the sandbox (QEMU, virtiofsd, or
    /// anything unrecognised) with its program name. Any entry blocks.
    pub others: Vec<(u32, String)>,
    /// Kata per-sandbox directories that still exist.
    pub sandbox_dirs: Vec<PathBuf>,
}

impl SandboxHostState {
    /// True when nothing on the host can still run the guest or serve it the
    /// durable root.
    pub fn vm_gone(&self) -> bool {
        self.others.is_empty() && self.sandbox_dirs.is_empty()
    }

    pub fn describe(&self) -> String {
        let mut facts = Vec::new();
        if !self.others.is_empty() {
            let processes = self
                .others
                .iter()
                .map(|(pid, program)| format!("{program} pid {pid}"))
                .collect::<Vec<_>>()
                .join(", ");
            facts.push(format!(
                "live processes still name the sandbox: {processes}"
            ));
        }
        if !self.sandbox_dirs.is_empty() {
            let dirs = self
                .sandbox_dirs
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            facts.push(format!("Kata sandbox directories still exist: {dirs}"));
        }
        if !self.shims.is_empty() {
            facts.push(format!("shim pids {:?} are still alive", self.shims));
        }
        if facts.is_empty() {
            "no live process or Kata directory names the sandbox".to_string()
        } else {
            facts.join("; ")
        }
    }
}

/// A containerd container ID as nerdctl prints it in full. Short or other
/// identifiers could match unrelated command lines, so they never qualify.
pub(super) fn valid_sandbox_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Read which host processes and Kata directories still refer to the
/// sandbox `sandbox_id`. `host_root` is `/` in production.
pub(super) fn sandbox_host_state(
    host_root: &Path,
    sandbox_id: &str,
) -> Result<SandboxHostState, String> {
    if !valid_sandbox_id(sandbox_id) {
        return Err(format!(
            "container ID {sandbox_id:?} is not a full containerd ID"
        ));
    }
    let mut state = SandboxHostState::default();
    let proc_root = host_root.join("proc");
    let entries = std::fs::read_dir(&proc_root)
        .map_err(|error| format!("cannot list {}: {error}", proc_root.display()))?;
    let mut scanned = 0_usize;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("cannot list {}: {error}", proc_root.display()))?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        scanned += 1;
        if scanned > MAX_SCANNED_PROCESSES {
            return Err("host process count exceeds the scan bound".to_string());
        }
        let cmdline = match std::fs::read(entry.path().join("cmdline")) {
            Ok(bytes) => bytes,
            Err(error) if process_vanished(&error) => continue,
            Err(error) => return Err(format!("cannot read process {pid}: {error}")),
        };
        let mut args = cmdline
            .split(|byte| *byte == 0)
            .filter(|arg| !arg.is_empty())
            .peekable();
        let program = args.peek().map(|arg| program_name(arg)).unwrap_or_default();
        if !args.any(|arg| contains(arg, sandbox_id.as_bytes())) {
            continue;
        }
        if is_containerd_shim(&program) {
            state.shims.push(pid);
        } else {
            state.others.push((pid, program));
        }
    }
    for dir in KATA_SANDBOX_DIRS {
        let path = host_root.join(dir).join(sandbox_id);
        match path.symlink_metadata() {
            Ok(_) => state.sandbox_dirs.push(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("cannot inspect {}: {error}", path.display())),
        }
    }
    state.shims.sort_unstable();
    state.others.sort();
    Ok(state)
}

/// The process exited between listing `/proc` and reading it.
fn process_vanished(error: &std::io::Error) -> bool {
    const ESRCH: i32 = 3;
    error.kind() == std::io::ErrorKind::NotFound || error.raw_os_error() == Some(ESRCH)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// argv[0]'s file name, without Nix's `.name-wrapped` decoration.
fn program_name(arg: &[u8]) -> String {
    let name = String::from_utf8_lossy(arg);
    let base = name.rsplit('/').next().unwrap_or_default();
    let base = base.strip_prefix('.').unwrap_or(base);
    base.strip_suffix("-wrapped").unwrap_or(base).to_string()
}

fn is_containerd_shim(program: &str) -> bool {
    program.starts_with("containerd-shim-")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "4c519ffa9c41a17f021ad36ae79b7de1e4d45407d9a7c9802464b5a1aeaaef9f";

    fn process(root: &Path, pid: u32, args: &[&str]) {
        let dir = root.join("proc").join(pid.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let mut cmdline = args.join("\0").into_bytes();
        cmdline.push(0);
        std::fs::write(dir.join("cmdline"), cmdline).unwrap();
    }

    fn host() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("proc")).unwrap();
        root
    }

    #[test]
    fn a_surviving_shim_alone_proves_the_vm_gone() {
        let root = host();
        process(
            root.path(),
            3431132,
            &[
                "/nix/store/x-kata-runtime-3.29.0/bin/containerd-shim-kata-v2",
                "-namespace",
                "finite",
                "-address",
                "/run/containerd/containerd.sock",
                "-id",
                ID,
            ],
        );
        // Unrelated processes and other sandboxes never count.
        process(root.path(), 7, &["/usr/bin/sleep", "infinity"]);
        process(
            root.path(),
            8,
            &[
                "qemu-system-x86_64",
                "-name",
                &format!("sandbox-{}", "a".repeat(64)),
            ],
        );
        // Kernel threads have an empty command line.
        std::fs::create_dir_all(root.path().join("proc/2")).unwrap();
        std::fs::write(root.path().join("proc/2/cmdline"), b"").unwrap();
        let state = sandbox_host_state(root.path(), ID).unwrap();
        assert!(state.vm_gone(), "{}", state.describe());
        assert_eq!(state.shims, vec![3431132]);
    }

    #[test]
    fn qemu_virtiofsd_or_any_unrecognised_holder_blocks_the_proof() {
        for (program, arg) in [
            (
                "/nix/store/q-qemu-11.0.1/bin/qemu-system-x86_64",
                format!("sandbox-{ID}"),
            ),
            (
                "/nix/store/q-qemu-11.0.1/bin/.qemu-system-x86_64-wrapped",
                format!("/run/vc/vm/{ID}/qmp.sock"),
            ),
            (
                "/nix/store/v-virtiofsd-1.13/bin/virtiofsd",
                format!("--shared-dir=/run/kata-containers/shared/sandboxes/{ID}/shared"),
            ),
            ("/usr/bin/nerdctl", ID.to_string()),
        ] {
            let root = host();
            process(root.path(), 42, &[program, &arg]);
            let state = sandbox_host_state(root.path(), ID).unwrap();
            assert!(!state.vm_gone(), "{program} must block the proof");
            assert_eq!(state.others.len(), 1, "{program}");
            assert!(state.describe().contains("pid 42"), "{}", state.describe());
        }
    }

    #[test]
    fn any_remaining_kata_sandbox_directory_blocks_the_proof() {
        for dir in KATA_SANDBOX_DIRS {
            let root = host();
            std::fs::create_dir_all(root.path().join(dir).join(ID)).unwrap();
            let state = sandbox_host_state(root.path(), ID).unwrap();
            assert!(!state.vm_gone(), "{dir} must block the proof");
            assert_eq!(state.sandbox_dirs.len(), 1);
        }
    }

    #[test]
    fn only_full_container_ids_are_scanned() {
        let root = host();
        for id in ["", "4c519ffa9c41", &ID.to_uppercase(), &format!("{ID}0")] {
            assert!(!valid_sandbox_id(id));
            assert!(sandbox_host_state(root.path(), id).is_err(), "{id}");
        }
        assert!(valid_sandbox_id(ID));
    }

    #[test]
    fn an_unreadable_process_table_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        assert!(sandbox_host_state(root.path(), ID).is_err());
    }
}
