//! Host publication fence. An interrupted mutation remains marked until a new,
//! quiescent systemd invocation proves its predecessor's commands have exited.
//! The lock alone is insufficient: Runner can die before nerdctl/CNI children.
use crate::{RunnerError, wait_with_captured_output};
use fs4::fs_std::FileExt;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

type Result<T> = std::result::Result<T, RunnerError>;
fn refused(reason: &str) -> RunnerError {
    RunnerError::RuntimeLaunch(format!("hosted Hermes lifecycle fence: {reason}"))
}

pub(crate) struct HostedHermesGuard {
    _lock: File,
    root: PathBuf,
    cgroup: PathBuf,
    invocation: String,
    proxy_unit: String,
}

impl HostedHermesGuard {
    pub(crate) fn acquire(root: &Path, runner_unit: &str, proxy_unit: &str) -> Result<Self> {
        if !cfg!(target_os = "linux") || !root.is_absolute() {
            return Err(refused(
                "requires a Linux systemd host and an absolute state directory",
            ));
        }
        for unit in [runner_unit, proxy_unit] {
            if !unit.ends_with(".service")
                || !unit
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
            {
                return Err(refused("invalid service identity"));
            }
        }
        std::fs::create_dir_all(root)
            .map_err(|_| refused("cannot create private state directory"))?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("host.lock"))
            .map_err(|_| refused("cannot open host lock"))?;
        file.try_lock_exclusive()
            .map_err(|_| refused("another host operation owns the fence"))?;
        let state = service_state(runner_unit)?;
        let cgroup = validate_runner(
            &state,
            std::process::id(),
            &std::fs::read_to_string("/proc/self/cgroup")
                .map_err(|_| refused("cannot read current cgroup"))?,
        )?;
        let guard = Self {
            _lock: file,
            root: root.into(),
            cgroup,
            invocation: state["InvocationID"].clone(),
            proxy_unit: proxy_unit.into(),
        };
        guard.quiescent()?;
        match std::fs::read_to_string(guard.marker()) {
            Ok(previous) => {
                if previous == guard.invocation {
                    return Err(refused(
                        "an earlier command in this invocation is unresolved",
                    ));
                }
                // A new main PID alone is not proof: check the entire cgroup
                // above, including leftovers after a failed systemd stop.
                guard.stop_proxy()?;
                guard.clear_marker()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(refused("cannot read interrupted-operation marker")),
        }
        Ok(guard)
    }

    pub(crate) fn before_publication(&self) -> Result<()> {
        self.quiescent()?;
        if self
            .marker()
            .try_exists()
            .map_err(|_| refused("cannot inspect operation marker"))?
        {
            return Err(refused("provider mutation is unresolved"));
        }
        Ok(())
    }

    /// Call before any container mutation. Long stops and starts retain port
    /// ownership; only address release requires ingress downtime before IO.
    pub(crate) fn begin_mutation(&self, releases_address: bool) -> Result<()> {
        self.before_publication()?;
        let mut marker = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.marker())
            .map_err(|_| refused("cannot record provider mutation"))?;
        marker
            .write_all(self.invocation.as_bytes())
            .and_then(|_| marker.sync_all())
            .map_err(|_| refused("cannot persist provider mutation"))?;
        self.sync_directory()?;
        if releases_address {
            self.stop_proxy()?;
        }
        Ok(())
    }

    /// Success at the CLI is not enough if a detached child can still mutate.
    /// On any command error, leave the marker for a new service invocation.
    pub(crate) fn finish_mutation(&self) -> Result<()> {
        self.quiescent()?;
        self.clear_marker()
    }

    pub(crate) fn stop_proxy(&self) -> Result<()> {
        let before = service_state(&self.proxy_unit)?;
        if before.get("LoadState").map(String::as_str) != Some("loaded")
            || before.get("KillMode").map(String::as_str) != Some("control-group")
            || before.get("SendSIGKILL").map(String::as_str) != Some("yes")
            || before.get("Restart").map(String::as_str) != Some("no")
        {
            return Err(refused(
                "dedicated proxy service has an unsafe lifetime policy",
            ));
        }
        systemctl(&["stop", &self.proxy_unit])?;
        let after = service_state(&self.proxy_unit)?;
        if after.get("MainPID").map(String::as_str) != Some("0")
            || !matches!(
                after.get("ActiveState").map(String::as_str),
                Some("inactive" | "failed")
            )
        {
            return Err(refused("dedicated proxy has not exited"));
        }
        // Check the old path too; systemd may clear ControlGroup on transition.
        for state in [&before, &after] {
            if let Some(group) = state.get("ControlGroup").filter(|group| !group.is_empty())
                && !cgroup_processes(&cgroup_path(group)?)?.is_empty()
            {
                return Err(refused("dedicated proxy still has live processes"));
            }
        }
        Ok(())
    }

    fn quiescent(&self) -> Result<()> {
        if cgroup_processes(&self.cgroup)? != BTreeSet::from([std::process::id()]) {
            return Err(refused(
                "Runner has surviving commands or an unreadable cgroup",
            ));
        }
        Ok(())
    }
    fn marker(&self) -> PathBuf {
        self.root.join("mutation-in-progress")
    }
    fn clear_marker(&self) -> Result<()> {
        std::fs::remove_file(self.marker()).map_err(|_| refused("cannot clear mutation marker"))?;
        self.sync_directory()
    }
    fn sync_directory(&self) -> Result<()> {
        File::open(&self.root)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| refused("cannot persist fence directory"))
    }
}

fn validate_runner(
    state: &BTreeMap<String, String>,
    pid: u32,
    proc_cgroup: &str,
) -> Result<PathBuf> {
    for (key, expected) in [
        ("LoadState", "loaded"),
        ("Type", "exec"),
        ("ExitType", "cgroup"),
        ("KillMode", "control-group"),
        ("SendSIGKILL", "yes"),
        ("Delegate", "no"),
        ("ActiveState", "active"),
    ] {
        if state.get(key).map(String::as_str) != Some(expected) {
            return Err(refused(
                "Runner must use the reviewed systemd cgroup lifetime",
            ));
        }
    }
    if state.get("MainPID") != Some(&pid.to_string()) {
        return Err(refused(
            "only the service main process may manage hosted routing",
        ));
    }
    let invocation = state
        .get("InvocationID")
        .ok_or_else(|| refused("missing invocation identity"))?;
    if invocation.len() != 32 || !invocation.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(refused("invalid invocation identity"));
    }
    let group = state
        .get("ControlGroup")
        .filter(|group| !group.is_empty())
        .ok_or_else(|| refused("missing cgroup identity"))?;
    if !proc_cgroup
        .lines()
        .any(|line| line.strip_prefix("0::") == Some(group.as_str()))
    {
        return Err(refused("service and kernel cgroup identities disagree"));
    }
    cgroup_path(group)
}

fn cgroup_path(group: &str) -> Result<PathBuf> {
    if !group.starts_with('/')
        || group == "/"
        || Path::new(group)
            .components()
            .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
    {
        return Err(refused("invalid service cgroup path"));
    }
    Ok(Path::new("/sys/fs/cgroup").join(&group[1..]))
}

fn cgroup_processes(root: &Path) -> Result<BTreeSet<u32>> {
    let mut pending = vec![root.to_owned()];
    let mut processes = BTreeSet::new();
    let mut visited = 0;
    while let Some(path) = pending.pop() {
        visited += 1;
        if visited > 64 {
            return Err(refused("unexpected cgroup tree size"));
        }
        let pids = match std::fs::read_to_string(path.join("cgroup.procs")) {
            Ok(pids) => pids,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(refused("cannot inspect service processes")),
        };
        for pid in pids.lines() {
            processes.insert(
                pid.parse()
                    .map_err(|_| refused("invalid cgroup process identity"))?,
            );
            if processes.len() > 4096 {
                return Err(refused("unexpected cgroup process count"));
            }
        }
        for entry in
            std::fs::read_dir(&path).map_err(|_| refused("cannot inspect child cgroups"))?
        {
            let entry = entry.map_err(|_| refused("cannot inspect child cgroup"))?;
            if entry
                .file_type()
                .map_err(|_| refused("cannot inspect cgroup entry"))?
                .is_dir()
            {
                pending.push(entry.path());
            }
        }
    }
    Ok(processes)
}

pub(crate) fn systemctl(args: &[&str]) -> Result<String> {
    let program = Path::new("systemctl");
    let child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| refused("cannot execute systemctl"))?;
    let output = wait_with_captured_output(child, program, Duration::from_secs(15))?;
    if !output.status.success() || output.stdout.len() > 64 * 1024 {
        return Err(refused("systemctl did not complete successfully"));
    }
    String::from_utf8(output.stdout).map_err(|_| refused("invalid systemctl response"))
}

fn service_state(unit: &str) -> Result<BTreeMap<String, String>> {
    systemctl(&["show", unit, "--property=LoadState,ActiveState,Type,ExitType,KillMode,SendSIGKILL,Delegate,MainPID,ControlGroup,InvocationID,Restart"])?
        .lines().map(|line| line.split_once('=').map(|(key, value)| (key.into(), value.into()))
            .ok_or_else(|| refused("malformed service state"))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> BTreeMap<String, String> {
        [
            ("LoadState", "loaded"),
            ("Type", "exec"),
            ("ExitType", "cgroup"),
            ("KillMode", "control-group"),
            ("SendSIGKILL", "yes"),
            ("Delegate", "no"),
            ("ActiveState", "active"),
            ("MainPID", "42"),
            ("InvocationID", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            ("ControlGroup", "/system.slice/runner.service"),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect()
    }

    #[test]
    fn only_current_main_process_with_complete_cgroup_policy_can_publish() {
        let current = "0::/system.slice/runner.service\n";
        assert!(validate_runner(&state(), 42, current).is_ok());
        for (key, invalid) in [
            ("Type", "oneshot"),
            ("ExitType", "main"),
            ("KillMode", "process"),
            ("SendSIGKILL", "no"),
            ("Delegate", "yes"),
            ("ActiveState", "deactivating"),
            ("LoadState", "not-found"),
            ("MainPID", "41"),
            ("InvocationID", "not-an-invocation"),
        ] {
            let mut changed = state();
            changed.insert(key.into(), invalid.into());
            assert!(validate_runner(&changed, 42, current).is_err(), "{key}");
            changed.remove(key);
            assert!(
                validate_runner(&changed, 42, current).is_err(),
                "missing {key}"
            );
        }
        assert!(validate_runner(&state(), 42, "0::/other.service").is_err());
        for path in ["/", "relative", "/system.slice/../other"] {
            assert!(cgroup_path(path).is_err());
        }
    }

    #[test]
    fn detached_descendants_are_counted_even_when_parent_group_is_empty() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("cgroup.procs"), "").unwrap();
        let child = root.path().join("child");
        std::fs::create_dir(&child).unwrap();
        std::fs::write(child.join("cgroup.procs"), "71\n72\n").unwrap();
        assert_eq!(
            cgroup_processes(root.path()).unwrap(),
            BTreeSet::from([71, 72])
        );
        std::fs::write(child.join("cgroup.procs"), "unreadable PID").unwrap();
        assert!(cgroup_processes(root.path()).is_err());
    }
}
