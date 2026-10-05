//! The launch identity of each relocation attempt, and the proof that all of
//! a relocation request's target compute is gone.
//!
//! The record lives beside the Runtime's other host-only metadata. The proof
//! is positive: containerd has no container for the exact recorded id, and no
//! process on this host belongs to that sandbox (by cgroup, containerd bundle
//! or Kata runtime directory) or has the durable tree mounted or open. Any
//! observation that fails or cannot be made is unknown, and unknown refuses.
//! Launch calibrates the process observations against the live sandbox, so an
//! identity that no process observation ever saw alive cannot be proved gone.
//! The durable-tree checks only ever add reasons to refuse.
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const RECORD_SCHEMA: &str = "relocation_attempt.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttemptRecord {
    schema: String,
    request_id: String,
    container_name: String,
    state_root: PathBuf,
    leases: Vec<LeaseAttempt>,
}

/// One lease of the request. Only the lease token's SHA-256 is kept.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct LeaseAttempt {
    lease_sha256: String,
    provider_work_started: bool,
    compute: Vec<ComputeIdentity>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ComputeIdentity {
    /// The containerd container id, which Kata also uses as the sandbox id
    /// for a single-container sandbox.
    container_id: String,
    /// The signals that saw this compute alive right after it started.
    signals: Vec<Signal>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Signal {
    /// A process whose cgroup names the sandbox id.
    Cgroup,
    /// A process working in the sandbox's containerd bundle: its shim.
    ShimBundle,
    /// A process holding the sandbox's Kata runtime or shared directories:
    /// its hypervisor or virtiofsd.
    SandboxDirectory,
    /// A process with the durable tree or the sandbox's shared directory
    /// mounted, in any mount namespace.
    Mount,
    /// A process with a descriptor, working directory or root in the tree.
    OpenTree,
}

impl Signal {
    /// Signals that correlate a process with the exact sandbox.
    fn identifies_process(self) -> bool {
        matches!(
            self,
            Signal::Cgroup | Signal::ShimBundle | Signal::SandboxDirectory
        )
    }
}

pub(crate) fn lease_sha256(lease: &AgentCreationLease) -> Result<String, RunnerError> {
    let token = lease.request.lease_token.as_deref().ok_or_else(|| {
        RunnerError::RuntimeLaunch("relocation attempt requires a lease token".to_string())
    })?;
    Ok(format!("{:x}", Sha256::digest(token.as_bytes())))
}

/// `Ok(None)` only when no record exists. An unreadable or malformed record
/// is an error.
pub(crate) fn load(path: &Path) -> Result<Option<AttemptRecord>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let record: AttemptRecord = serde_json::from_slice(&bytes)
        .map_err(|error| format!("malformed {}: {error}", path.display()))?;
    if record.schema != RECORD_SCHEMA {
        return Err(format!("{} has an unknown schema", path.display()));
    }
    Ok(Some(record))
}

fn store(path: &Path, record: &AttemptRecord) -> Result<(), RunnerError> {
    let failed = |error: std::io::Error| {
        RunnerError::RuntimeLaunch(format!("cannot write {}: {error}", path.display()))
    };
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(record)
        .map_err(|error| RunnerError::RuntimeLaunch(error.to_string()))?;
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temporary).map_err(failed)?;
    file.write_all(&bytes).map_err(failed)?;
    file.sync_all().map_err(failed)?;
    std::fs::rename(&temporary, path).map_err(failed)
}

/// Adds this lease to the request's record, creating the record if needed.
/// Entries only move forward: an existing entry is never reset.
pub(crate) fn record_lease(
    path: &Path,
    plan: &KataLaunchPlan,
    lease: &AgentCreationLease,
    provider_work_started: bool,
) -> Result<(), RunnerError> {
    let mut record = load(path)
        .map_err(RunnerError::RuntimeLaunch)?
        .unwrap_or_else(|| AttemptRecord {
            schema: RECORD_SCHEMA.to_string(),
            request_id: lease.request.id.clone(),
            container_name: plan.container_name.clone(),
            state_root: plan.state_root.clone(),
            leases: Vec::new(),
        });
    let sha = lease_sha256(lease)?;
    match record
        .leases
        .iter_mut()
        .find(|entry| entry.lease_sha256 == sha)
    {
        Some(entry) => entry.provider_work_started |= provider_work_started,
        None => record.leases.push(LeaseAttempt {
            lease_sha256: sha,
            provider_work_started,
            compute: Vec::new(),
        }),
    }
    store(path, &record)
}

/// Records the compute this lease just started and the signals that see it
/// alive now.
pub(crate) fn record_compute(
    path: &Path,
    lease: &AgentCreationLease,
    container_id: &str,
    signals: Vec<Signal>,
) -> Result<(), RunnerError> {
    let mut record = load(path)
        .map_err(RunnerError::RuntimeLaunch)?
        .ok_or_else(|| {
            RunnerError::RuntimeLaunch("relocation attempt record vanished during launch".into())
        })?;
    let sha = lease_sha256(lease)?;
    let entry = record
        .leases
        .iter_mut()
        .find(|entry| entry.lease_sha256 == sha)
        .ok_or_else(|| RunnerError::RuntimeLaunch("relocation lease was not recorded".into()))?;
    entry.provider_work_started = true;
    entry.compute.push(ComputeIdentity {
        container_id: container_id.to_string(),
        signals,
    });
    store(path, &record)
}

/// The container id `nerdctl run --detach` printed.
pub(crate) fn container_id_from_run_output(stdout: &str) -> Result<String, RunnerError> {
    let id = stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty());
    match id {
        Some(id) if !id.contains(char::is_whitespace) && !id.contains('/') => Ok(id.to_string()),
        _ => Err(RunnerError::RuntimeLaunch(
            "nerdctl run did not report a container id".to_string(),
        )),
    }
}

struct SandboxPaths {
    id: String,
    bundle: PathBuf,
    runtime_directories: Vec<PathBuf>,
    shared: PathBuf,
    tree: PathBuf,
}

fn sandbox_paths(config: &KataConfig, id: &str, tree: &Path) -> SandboxPaths {
    let view = &config.host_view;
    let shared = view.kata_shared_sandboxes_root.join(id);
    SandboxPaths {
        id: id.to_string(),
        bundle: view
            .containerd_task_root
            .join(config.namespace.trim())
            .join(id),
        runtime_directories: vec![
            view.kata_vm_root.join(id),
            view.kata_sandbox_state_root.join(id),
            shared.clone(),
        ],
        shared,
        tree: tree.to_path_buf(),
    }
}

/// Every signal any process on this host shows for the sandbox. An error
/// means an observation failed, which the caller treats as unknown.
fn observe(proc_root: &Path, paths: &SandboxPaths) -> Result<BTreeSet<Signal>, String> {
    let entries = std::fs::read_dir(proc_root)
        .map_err(|error| format!("cannot list {}: {error}", proc_root.display()))?;
    let mut seen = BTreeSet::new();
    let mut processes = 0usize;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("cannot list {}: {error}", proc_root.display()))?;
        let is_pid = entry
            .file_name()
            .to_str()
            .is_some_and(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()));
        if !is_pid {
            continue;
        }
        processes += 1;
        observe_process(&entry.path(), paths, &mut seen)?;
    }
    if processes == 0 {
        return Err(format!("{} shows no processes", proc_root.display()));
    }
    Ok(seen)
}

/// A field that vanished belongs to a process that exited or a kernel thread;
/// any other failure is unknown.
fn optional<T>(result: std::io::Result<T>, what: &Path) -> Result<Option<T>, String> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", what.display())),
    }
}

fn observe_process(
    dir: &Path,
    paths: &SandboxPaths,
    seen: &mut BTreeSet<Signal>,
) -> Result<(), String> {
    let cgroup_path = dir.join("cgroup");
    if let Some(cgroup) = optional(std::fs::read_to_string(&cgroup_path), &cgroup_path)?
        && cgroup.contains(&paths.id)
    {
        seen.insert(Signal::Cgroup);
    }
    for link in ["cwd", "root"] {
        let path = dir.join(link);
        if let Some(target) = optional(std::fs::read_link(&path), &path)? {
            classify_open(&target, paths, link == "cwd", seen);
        }
    }
    let fd_dir = dir.join("fd");
    if let Some(fds) = optional(std::fs::read_dir(&fd_dir), &fd_dir)? {
        for fd in fds {
            let fd = fd.map_err(|error| format!("cannot list {}: {error}", fd_dir.display()))?;
            if let Some(target) = optional(std::fs::read_link(fd.path()), &fd.path())? {
                classify_open(&target, paths, false, seen);
            }
        }
    }
    let mountinfo_path = dir.join("mountinfo");
    if let Some(mountinfo) = optional(std::fs::read_to_string(&mountinfo_path), &mountinfo_path)? {
        for line in mountinfo.lines() {
            let fields: Vec<&str> = line.split(' ').collect();
            if fields.len() < 5 {
                continue;
            }
            let root = unescape_mountinfo(fields[3]);
            let mount_point = unescape_mountinfo(fields[4]);
            if mount_point.starts_with(&paths.shared)
                || mount_point.starts_with(&paths.tree)
                || mentions_tree(&root, &paths.tree)
            {
                seen.insert(Signal::Mount);
            }
        }
    }
    Ok(())
}

fn classify_open(target: &Path, paths: &SandboxPaths, is_cwd: bool, seen: &mut BTreeSet<Signal>) {
    if is_cwd && target.starts_with(&paths.bundle) {
        seen.insert(Signal::ShimBundle);
    }
    if paths
        .runtime_directories
        .iter()
        .any(|directory| target.starts_with(directory))
    {
        seen.insert(Signal::SandboxDirectory);
    }
    if target.starts_with(&paths.tree) {
        seen.insert(Signal::OpenTree);
    }
}

/// A bind mount's root is the tree's path within its filesystem, so it ends
/// with the tree's own directory name. Matching generously only ever refuses.
fn mentions_tree(root: &Path, tree: &Path) -> bool {
    let Some(name) = tree.file_name() else {
        return false;
    };
    root != Path::new("/") && root.components().any(|part| part.as_os_str() == name)
}

fn unescape_mountinfo(field: &str) -> PathBuf {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && index + 3 < bytes.len()
            && bytes[index + 1..index + 4]
                .iter()
                .all(|b| (b'0'..=b'7').contains(b))
        {
            let value = u32::from(bytes[index + 1] - b'0') * 64
                + u32::from(bytes[index + 2] - b'0') * 8
                + u32::from(bytes[index + 3] - b'0');
            out.push(u8::try_from(value).unwrap_or(b'?'));
            index += 4;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    PathBuf::from(String::from_utf8_lossy(&out).into_owned())
}

/// The signals that see the sandbox alive right after it started. A failed
/// observation calibrates nothing, which later makes the proof unknown.
pub(crate) fn calibrate(config: &KataConfig, container_id: &str, tree: &Path) -> Vec<Signal> {
    let paths = sandbox_paths(config, container_id, tree);
    observe(&config.host_view.proc_root, &paths)
        .map(|seen| seen.into_iter().collect())
        .unwrap_or_default()
}

/// Removes, by recorded id, the compute started by the leases up to and
/// including `through`. Later leases' compute is never touched.
pub(crate) fn remove_recorded_compute(
    launcher: &KataLauncher,
    plan: &KataLaunchPlan,
    project_id: &str,
    record: &AttemptRecord,
    through: usize,
) -> Result<(), RunnerError> {
    for entry in &record.leases[..=through] {
        for identity in &entry.compute {
            let Some(inspected) = launcher.inspect(&identity.container_id)? else {
                continue;
            };
            launcher.validate_owned(plan, project_id, &inspected)?;
            launcher.remove_compute(&identity.container_id)?;
        }
    }
    Ok(())
}

pub(crate) fn lease_index(
    record: &AttemptRecord,
    lease: &AgentCreationLease,
) -> Result<usize, String> {
    let sha = lease_sha256(lease).map_err(|error| error.to_string())?;
    record
        .leases
        .iter()
        .position(|entry| entry.lease_sha256 == sha)
        .ok_or_else(|| "this lease has no entry in the relocation attempt record".to_string())
}

/// Positive proof that every compute any lease of the request started is
/// gone. `Err` carries the reason the proof is unknown or refused.
pub(crate) fn prove_request_compute_down(
    launcher: &KataLauncher,
    record: &AttemptRecord,
) -> Result<(), String> {
    let started: Vec<&LeaseAttempt> = record
        .leases
        .iter()
        .filter(|entry| entry.provider_work_started)
        .collect();
    // Only a record saying provider work never started counts as never staged.
    if started.is_empty() {
        return Ok(());
    }
    for entry in &started {
        if entry.compute.is_empty() {
            return Err("provider work started but no compute identity was recorded".to_string());
        }
    }
    let config = &launcher.config;
    for identity in started.iter().flat_map(|entry| &entry.compute) {
        let id = &identity.container_id;
        if !identity
            .signals
            .iter()
            .any(|signal| signal.identifies_process())
        {
            return Err(format!(
                "no process observation ever saw {id} alive, so its absence cannot be observed"
            ));
        }
        match launcher.inspect(id) {
            Ok(None) => {}
            Ok(Some(_)) => return Err(format!("containerd still has container {id}")),
            Err(error) => return Err(format!("containerd inventory for {id} failed: {error}")),
        }
        let paths = sandbox_paths(config, id, &record.state_root);
        let seen = observe(&config.host_view.proc_root, &paths)?;
        if !seen.is_empty() {
            return Err(format!("sandbox {id} is still observed: {seen:?}"));
        }
    }
    refuse_on_tree_evidence(launcher, &record.state_root)
}

/// The tree checks can only refuse: a running record binding the tree, a
/// live writer-lease holder, a changing tree, or a tree that cannot be read.
fn refuse_on_tree_evidence(launcher: &KataLauncher, tree: &Path) -> Result<(), String> {
    for container_name in launcher.container_names().map_err(|e| e.to_string())? {
        let Some(inspected) = launcher
            .inspect(&container_name)
            .map_err(|e| e.to_string())?
        else {
            continue;
        };
        let binds_tree = inspected
            .mounts
            .iter()
            .any(|mount| mount.destination == Path::new("/data") && mount.source == tree);
        if binds_tree && inspected.state.status == "running" {
            return Err(format!(
                "container {container_name} still runs against {}",
                tree.display()
            ));
        }
    }
    match std::fs::symlink_metadata(tree) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => return Err(format!("{} is not a directory", tree.display())),
        Err(error) => return Err(format!("cannot inspect {}: {error}", tree.display())),
    }
    let quiescence =
        durable_tree_is_quiescent_within(tree, launcher.config.durable_tree_quiescence_window)
            .map_err(|error| error.to_string())?;
    match quiescence.evidence() {
        Some(evidence) => Err(evidence),
        None => Ok(()),
    }
}
