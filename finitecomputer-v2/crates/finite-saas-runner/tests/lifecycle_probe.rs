//! Incident-topology fixtures for the read-only lifecycle probe.
//!
//! The fake provider layer lives in `tests/fixtures/lifecycle/bin/` and is
//! deliberately reusable by later recovery work: state is one file per field
//! under `fake-state/`, and both fakes record and reject any non-read verb so
//! every scenario also proves the probe mutated nothing. The persist state
//! fixtures mirror Kata's real Go-serialized `persistapi.SandboxState` shape
//! (capitalized, tagless fields) under the real `/run/vc/sbs` layout.

use finite_saas_runner::lifecycle_probe::{
    CheckStatus, LifecycleProbeConfig, LifecycleProbeReport, LifecycleProbeRequest,
    LifecycleVerdict, probe_runtime_lifecycle,
};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const FAKE_NERDCTL: &str = include_str!("fixtures/lifecycle/bin/nerdctl");
const FAKE_CTR: &str = include_str!("fixtures/lifecycle/bin/ctr");

const PROJECT: &str = "project-a";
const RUNTIME: &str = "runtime-a";
const MACHINE: &str = "machine-a";

struct ProbeFixture {
    _root: tempfile::TempDir,
    config: LifecycleProbeConfig,
    state: PathBuf,
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn new_fixture() -> ProbeFixture {
    let root = tempfile::TempDir::new().unwrap();
    let bin = root.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_executable(&bin.join("nerdctl"), FAKE_NERDCTL);
    write_executable(&bin.join("ctr"), FAKE_CTR);
    let state = bin.join("fake-state");
    fs::create_dir_all(&state).unwrap();
    let work_root = root.path().join("work");
    let config = LifecycleProbeConfig {
        nerdctl_bin: bin.join("nerdctl"),
        ctr_bin: bin.join("ctr"),
        namespace: "finite".to_string(),
        source_host_id: "finite-lat-1".to_string(),
        work_root: work_root.clone(),
        sandbox_root: root.path().join("sbs"),
        netns_root: root.path().join("netns"),
        proc_root: root.path().join("proc"),
        // Generous bounds: CI runners are shared and heavily contended.
        command_timeout: Duration::from_secs(30),
        overall_timeout: Duration::from_secs(120),
    };
    ProbeFixture {
        _root: root,
        config,
        state,
    }
}

impl ProbeFixture {
    fn state_root(&self) -> PathBuf {
        self.config.work_root.join("kata").join(MACHINE)
    }

    /// Register one container with the fake provider layer.
    fn add_container(&self, name: &str, status: &str, project: &str, source: &str, mount: &Path) {
        let field = |suffix: &str, value: &str| {
            fs::write(self.state.join(format!("{name}.{suffix}")), value).unwrap();
        };
        field("image", "registry.example/runtime@sha256:aaa");
        field("status", status);
        field("project", project);
        field("source", source);
        field("mount", mount.to_str().unwrap());
    }

    fn set_container_id(&self, name: &str, id: &str) {
        fs::write(self.state.join(format!("{name}.id")), id).unwrap();
    }

    fn add_task(&self, container_id: &str, status: &str) {
        let tasks = self.state.join("tasks");
        let mut contents = fs::read_to_string(&tasks)
            .unwrap_or_else(|_| "TASK                PID       STATUS\n".to_string());
        contents.push_str(&format!("{container_id:<20}4242      {status}\n"));
        fs::write(&tasks, contents).unwrap();
    }

    fn fail_tasks_with(&self, stderr: &str) {
        fs::write(self.state.join("tasks-error"), stderr).unwrap();
    }

    fn fail_task_ps_with(&self, stderr: &str) {
        fs::write(self.state.join("tasks-ps-error"), stderr).unwrap();
    }

    /// Write Kata persist state in the real `persistapi.SandboxState` shape.
    fn write_persist(&self, sandbox_container: &str, pid: Option<u64>) {
        let dir = self.config.sandbox_root.join(container_id());
        fs::create_dir_all(&dir).unwrap();
        let hypervisor = match pid {
            Some(pid) => format!(r#","HypervisorState":{{"Pid":{pid},"Type":"qemu"}}"#),
            None => String::new(),
        };
        fs::write(
            dir.join("persist.json"),
            format!(
                r#"{{"State":"running","SandboxContainer":"{sandbox_container}","PersistVersion":2{hypervisor}}}"#
            ),
        )
        .unwrap();
    }

    fn add_netns_record(&self, container_id: &str) {
        fs::create_dir_all(&self.config.netns_root).unwrap();
        fs::write(
            self.config.netns_root.join(format!("cni-{container_id}")),
            "",
        )
        .unwrap();
    }

    fn add_vmm_comm(&self, pid: u64, comm: &str) {
        let dir = self.config.proc_root.join(pid.to_string());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
    }

    /// A healthy runtime: owned canonical container, running task, live shim
    /// channel, self-consistent persist state, visible QEMU VMM.
    fn make_healthy(&self) {
        self.add_container(MACHINE, "running", PROJECT, MACHINE, &self.state_root());
        self.add_task(&container_id(), "RUNNING");
        self.write_persist(MACHINE, Some(4242));
        self.add_vmm_comm(4242, "qemu-system-x86");
    }

    fn probe(&self) -> LifecycleProbeReport {
        probe_runtime_lifecycle(
            &self.config,
            &LifecycleProbeRequest {
                project_id: PROJECT.to_string(),
                agent_runtime_id: RUNTIME.to_string(),
                source_machine_id: MACHINE.to_string(),
            },
        )
    }

    fn check<'a>(
        &self,
        report: &'a LifecycleProbeReport,
        name: &str,
    ) -> &'a finite_saas_runner::lifecycle_probe::LifecycleProbeCheck {
        report
            .checks
            .iter()
            .find(|check| check.name == name)
            .unwrap_or_else(|| panic!("missing check {name} in {report:?}"))
    }

    /// The probe is read-only by construction: every scenario asserts the
    /// fake provider layer never saw a mutating verb.
    fn assert_nothing_mutated(&self) {
        let mutations = self.state.join("mutations.log");
        assert!(
            !mutations.exists(),
            "probe issued mutating provider calls: {:?}",
            fs::read_to_string(&mutations).unwrap()
        );
        let log = fs::read_to_string(self.state.join("commands.log")).unwrap();
        for line in log.lines() {
            let verb: Vec<&str> = line.split_whitespace().collect();
            assert!(
                matches!(
                    verb.first(),
                    Some(&"inspect") | Some(&"ps") | Some(&"tasks")
                ),
                "unexpected provider invocation in probe: {line}"
            );
        }
    }
}

fn container_id() -> String {
    format!("{MACHINE}-id")
}

/// Assertion failures must carry the whole report so CI logs are diagnosable.
fn assert_verdict(report: &LifecycleProbeReport, expected: LifecycleVerdict) {
    assert_eq!(
        report.verdict,
        expected,
        "unexpected verdict; full report:\n{}",
        serde_json::to_string_pretty(report).unwrap()
    );
}

#[test]
fn healthy_runtime_is_operable() {
    let fixture = new_fixture();
    fixture.make_healthy();

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Operable);
    assert_eq!(report.reason, None);
    assert_eq!(report.schema, "finite.lifecycle-probe.v1");
    for check in &report.checks {
        assert!(
            !matches!(check.status, CheckStatus::Fail),
            "check {} unexpectedly failed; full report:\n{}",
            check.name,
            serde_json::to_string_pretty(&report).unwrap()
        );
    }
    fixture.assert_nothing_mutated();
}

#[test]
fn orphaned_vm_without_healthy_task_is_inoperable() {
    // 2026-08-01 incident topology 1: the endpoint is live, the containerd
    // task is orphaned, and a normal stop would time out.
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &fixture.state_root());
    fixture.write_persist(MACHINE, Some(4242));
    fixture.add_netns_record(&container_id());

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Inoperable);
    assert_eq!(report.reason.as_deref(), Some("orphaned_task"));
    assert_eq!(
        fixture.check(&report, "containerd_task").finding,
        Some("orphaned_task")
    );
    // The stale sandbox and CNI records are surfaced as evidence, not hidden.
    assert_eq!(
        fixture.check(&report, "sandbox_state").finding,
        Some("stale_sandbox_state")
    );
    assert_eq!(
        fixture.check(&report, "cni_namespace").finding,
        Some("stale_cni_namespace")
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn ttrpc_closed_control_channel_is_inoperable() {
    // 2026-08-01 incident topology 2: old-VM shutdown repeatedly failed with
    // `ttrpc: closed`. `tasks list` is answered from containerd metadata and
    // still shows RUNNING for a dead-shim VM; only the shim-answered no-op
    // read sees the closed channel.
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &fixture.state_root());
    fixture.add_task(&container_id(), "RUNNING");
    fixture.fail_task_ps_with("rpc error: code = Unavailable desc = ttrpc: closed\n");

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Inoperable);
    assert_eq!(report.reason.as_deref(), Some("control_channel_closed"));
    let task = fixture.check(&report, "containerd_task");
    assert_eq!(task.finding, Some("control_channel_closed"));
    assert_eq!(task.evidence["task_status"].as_str(), Some("RUNNING"));
    assert!(task.evidence["stderr"].as_str().unwrap().contains("ttrpc"));
    fixture.assert_nothing_mutated();
}

#[test]
fn stale_cni_namespace_is_detected() {
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &fixture.state_root());
    fixture.add_task(&container_id(), "PAUSED");
    fixture.write_persist(MACHINE, None);
    fixture.add_netns_record(&container_id());

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Degraded);
    assert_eq!(report.reason.as_deref(), Some("task_not_running"));
    assert_eq!(
        fixture.check(&report, "cni_namespace").finding,
        Some("stale_cni_namespace")
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn stale_kata_persist_state_is_detected() {
    let fixture = new_fixture();
    fixture.make_healthy();
    // Persist state belonging to a superseded container.
    fixture.write_persist("superseded-container", Some(4242));

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Degraded);
    assert_eq!(report.reason.as_deref(), Some("stale_sandbox_state"));
    let sandbox = fixture.check(&report, "sandbox_state");
    assert_eq!(
        sandbox.evidence["persist_sandbox_container"].as_str(),
        Some("superseded-container")
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn inconsistent_kata_persist_state_is_detected() {
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &fixture.state_root());
    fixture.add_task(&container_id(), "RUNNING");
    let dir = fixture.config.sandbox_root.join(container_id());
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("persist.json"), b"{not json").unwrap();

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Degraded);
    assert_eq!(report.reason.as_deref(), Some("sandbox_state_inconsistent"));
    fixture.assert_nothing_mutated();
}

#[test]
fn wrapped_qemu_process_name_is_detected() {
    // The incident's wrapped QEMU process name defeats name-based process
    // matching; the probe reports the observed comm as evidence.
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &fixture.state_root());
    fixture.add_task(&container_id(), "RUNNING");
    fixture.write_persist(MACHINE, Some(4242));
    fixture.add_vmm_comm(4242, "wrap-qemu-syste");

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Degraded);
    assert_eq!(report.reason.as_deref(), Some("wrapped_vmm_process_name"));
    let vmm = fixture.check(&report, "vmm_process");
    assert_eq!(vmm.evidence["comm"].as_str(), Some("wrap-qemu-syste"));
    assert_eq!(vmm.evidence["hypervisor_type"].as_str(), Some("qemu"));
    fixture.assert_nothing_mutated();
}

#[test]
fn duplicate_durable_writer_is_inoperable() {
    let fixture = new_fixture();
    fixture.make_healthy();
    // A second container owns the same source machine and mounts the same
    // durable root (and therefore writes the same Agent npub).
    fixture.add_container(
        "machine-a-copy",
        "running",
        PROJECT,
        MACHINE,
        &fixture.state_root(),
    );

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Inoperable);
    assert_eq!(report.reason.as_deref(), Some("duplicate_durable_writer"));
    let duplicates = fixture.check(&report, "duplicate_writers");
    assert_eq!(
        duplicates.evidence["duplicates"][0]["container_name"].as_str(),
        Some("machine-a-copy")
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn missing_provider_handle_is_inoperable_and_gates_dependent_reads() {
    let fixture = new_fixture();

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Inoperable);
    assert_eq!(report.reason.as_deref(), Some("provider_handle_missing"));
    assert_eq!(
        fixture.check(&report, "containerd_task").status,
        CheckStatus::Skip
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn inconclusive_evidence_is_unknown_not_operable() {
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &fixture.state_root());
    fixture.fail_tasks_with("connection refused\n");

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("task_list_error"));
    fixture.assert_nothing_mutated();
}

#[test]
fn invalid_container_id_never_derives_state_paths() {
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &fixture.state_root());
    fixture.set_container_id(MACHINE, "../escape");

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("provider_handle_invalid"));
    assert_eq!(
        fixture.check(&report, "sandbox_state").status,
        CheckStatus::Skip
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn unreadable_cni_inventory_is_unknown_not_skipped() {
    let fixture = new_fixture();
    fixture.make_healthy();
    // A netns root that exists but is not a directory fails read_dir with a
    // non-NotFound error on every platform.
    fs::write(&fixture.config.netns_root, "not a directory").unwrap();

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("cni_inventory_unreadable"));
    fixture.assert_nothing_mutated();
}

#[test]
fn unreadable_vmm_comm_is_unknown_not_skipped() {
    let fixture = new_fixture();
    fixture.make_healthy();
    // A directory at the comm path fails the bounded read on every platform.
    let comm = fixture.config.proc_root.join("4242").join("comm");
    fs::remove_file(&comm).unwrap();
    fs::create_dir_all(&comm).unwrap();

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("vmm_process_unreadable"));
    fixture.assert_nothing_mutated();
}

#[test]
fn missing_vmm_pid_on_a_running_task_is_unknown_not_skipped() {
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &fixture.state_root());
    fixture.add_task(&container_id(), "RUNNING");
    fixture.write_persist(MACHINE, None);

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("vmm_pid_unavailable"));
    fixture.assert_nothing_mutated();
}

#[test]
fn exceeded_overall_deadline_is_unknown_evidence() {
    let fixture = new_fixture();
    fixture.make_healthy();
    let mut config = fixture.config.clone();
    config.overall_timeout = Duration::ZERO;
    let report = probe_runtime_lifecycle(
        &config,
        &LifecycleProbeRequest {
            project_id: PROJECT.to_string(),
            agent_runtime_id: RUNTIME.to_string(),
            source_machine_id: MACHINE.to_string(),
        },
    );
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("probe_deadline_exceeded"));
    assert_eq!(
        fixture.check(&report, "duplicate_writers").finding,
        Some("probe_deadline_exceeded")
    );
    fixture.assert_nothing_mutated();
}
