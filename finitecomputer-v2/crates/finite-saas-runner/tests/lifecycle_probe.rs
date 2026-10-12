//! Incident-topology fixtures for the read-only lifecycle probe.
//!
//! The fake provider layer lives in `tests/fixtures/lifecycle/bin/` and is
//! deliberately reusable by later recovery work: state is one file per field
//! under `fake-state/`, and both fakes record and reject any non-read verb so
//! every scenario also proves the probe mutated nothing. The fake `ctr`
//! follows containerd 2.3: `tasks list` omits a task whose shim State call
//! fails, and `tasks ps` reports containerd's own absence text only for a task
//! its task manager does not hold. The fake `nerdctl` prints each container's
//! status verbatim, so a container without a task uses nerdctl 2.3's empty
//! status. The persist state fixtures mirror Kata's
//! real Go-serialized `persistapi.SandboxState` shape (capitalized, tagless
//! fields) under the real `/run/vc/sbs` layout.

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
/// nerdctl 2.3 inspect reports a status only after loading the container's
/// task; a container whose task it cannot load has this empty status.
const NO_TASK_STATUS: &str = "";

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
    /// Fleet reality (verified on lat1 and lat3): durable state roots are
    /// named by the agent runtime id (Core's `durable_state_id`), while the
    /// container is named by the source machine id. Encoding the divergence
    /// here means every scenario exercises it.
    fn state_root(&self) -> PathBuf {
        self.config.work_root.join("kata").join(RUNTIME)
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

    /// The runtime's container exists, but nerdctl can load no task for it.
    fn add_runtime_without_task(&self) {
        self.add_container(
            MACHINE,
            NO_TASK_STATUS,
            PROJECT,
            MACHINE,
            &self.state_root(),
        );
    }

    fn set_container_id(&self, name: &str, id: &str) {
        fs::write(self.state.join(format!("{name}.id")), id).unwrap();
    }

    fn add_task(&self, container_id: &str, status: &str) {
        let tasks = self.state.join("tasks");
        let mut contents = fs::read_to_string(&tasks)
            .unwrap_or_else(|_| "TASK                PID       STATUS\n".to_string());
        contents.push_str(&format!("{container_id}  4242      {status}\n"));
        fs::write(&tasks, contents).unwrap();
    }

    fn fail_tasks_with(&self, stderr: &str) {
        fs::write(self.state.join("tasks-error"), stderr).unwrap();
    }

    fn fail_task_ps_with(&self, stderr: &str) {
        fs::write(self.state.join("tasks-ps-error"), stderr).unwrap();
    }

    /// The task manager holds the task, but its shim's State call failed, so
    /// containerd 2.3's List omits it while still succeeding.
    fn omit_task_from_list(&self, container_id: &str) {
        fs::write(
            self.state.join("tasks-unlisted"),
            format!("{container_id}\n"),
        )
        .unwrap();
    }

    /// The named container is replaced while the probe reads its task.
    fn replace_container_during_task_read(&self, name: &str, new_id: &str) {
        fs::write(
            self.state.join("tasks-ps-replaces"),
            format!("{name} {new_id}\n"),
        )
        .unwrap();
    }

    /// The named container's task stops while the probe reads it.
    fn stop_task_during_task_read(&self, name: &str) {
        fs::write(self.state.join("tasks-ps-stops"), name).unwrap();
    }

    /// A new task starts under the named container's id while the probe
    /// reads its task.
    fn start_task_during_task_read(&self, name: &str) {
        fs::write(self.state.join("tasks-ps-starts"), name).unwrap();
    }

    /// The named container is removed while the probe reads its task.
    fn remove_container_during_task_read(&self, name: &str) {
        fs::write(self.state.join("tasks-ps-removes"), name).unwrap();
    }

    fn task_reads(&self) -> usize {
        fs::read_to_string(self.state.join("commands.log"))
            .unwrap()
            .lines()
            .filter(|line| *line == "tasks ps")
            .count()
    }

    /// Write Kata persist state in the real `persistapi.SandboxState` shape.
    fn write_persist(&self, sandbox_container: &str, pid: Option<u64>) {
        self.write_hash_persist(&container_id(), sandbox_container, pid);
    }

    /// Write persist state under an explicit sandbox directory: `dir_id` is
    /// the `/run/vc/sbs/<dir_id>` component (on the fleet, the container id)
    /// and `sandbox_container` is the record's `SandboxContainer` field.
    fn write_hash_persist(&self, dir_id: &str, sandbox_container: &str, pid: Option<u64>) {
        let dir = self.config.sandbox_root.join(dir_id);
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
        self.write_persist(&container_id(), Some(4242));
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
fn runtime_id_state_root_with_machine_named_container_is_operable() {
    // Production validation on lat3 (canary runtime_354cb67be83d38733cae):
    // the container is machine-named, its durable /data bind is named by the
    // agent runtime id, and its Kata persist record names the hash container
    // id (verified: SandboxContainer == the nerdctl container id, NOT the
    // container name). This exact divergence must read operable.
    let fixture = new_fixture();
    let machine = "finite-kata-0fb1fc92761b75188cfc";
    let runtime_id = "runtime_354cb67be83d38733cae";
    let container_id = "c4ac3912158f15196e403d848d0cfbbd52d2f1dce96bbe40c3dc8fced3f2ce25";
    fixture.add_container(
        machine,
        "running",
        PROJECT,
        machine,
        &fixture.config.work_root.join("kata").join(runtime_id),
    );
    fixture.set_container_id(machine, container_id);
    fixture.add_task(container_id, "RUNNING");
    fixture.write_hash_persist(container_id, container_id, Some(4242));
    fixture.add_vmm_comm(4242, "qemu-system-x86");

    let request = LifecycleProbeRequest {
        project_id: PROJECT.to_string(),
        agent_runtime_id: runtime_id.to_string(),
        source_machine_id: machine.to_string(),
    };
    let report = probe_runtime_lifecycle(&fixture.config, &request);
    assert_verdict(&report, LifecycleVerdict::Operable);
    fixture.assert_nothing_mutated();
}

#[test]
fn hash_named_persist_for_a_different_container_id_is_stale() {
    // Same topology, but the persist record names a DIFFERENT container id:
    // genuinely stale sandbox state, degraded with evidence.
    let fixture = new_fixture();
    let machine = "finite-kata-0fb1fc92761b75188cfc";
    let runtime_id = "runtime_354cb67be83d38733cae";
    let container_id = "c4ac3912158f15196e403d848d0cfbbd52d2f1dce96bbe40c3dc8fced3f2ce25";
    let stale_id = "9b1c4e2a7f0d4856a3e2c1b0d9f8a7c6e5d4c3b2a19081726354ffe0d1c2b3a4";
    fixture.add_container(
        machine,
        "running",
        PROJECT,
        machine,
        &fixture.config.work_root.join("kata").join(runtime_id),
    );
    fixture.set_container_id(machine, container_id);
    fixture.add_task(container_id, "RUNNING");
    fixture.write_hash_persist(container_id, stale_id, Some(4242));
    fixture.add_vmm_comm(4242, "qemu-system-x86");

    let request = LifecycleProbeRequest {
        project_id: PROJECT.to_string(),
        agent_runtime_id: runtime_id.to_string(),
        source_machine_id: machine.to_string(),
    };
    let report = probe_runtime_lifecycle(&fixture.config, &request);
    assert_verdict(&report, LifecycleVerdict::Degraded);
    assert_eq!(report.reason.as_deref(), Some("stale_sandbox_state"));
    let sandbox = fixture.check(&report, "sandbox_state");
    assert_eq!(
        sandbox.evidence["persist_sandbox_container"].as_str(),
        Some(stale_id)
    );
    assert_eq!(
        sandbox.evidence["container_id"].as_str(),
        Some(container_id)
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn own_machine_named_root_is_a_mismatch_even_when_the_runtime_id_root_is_absent() {
    // The durable root has exactly one name: the runtime id. A container
    // bound to its OWN machine-named root — the pre-runtime-id-era layout —
    // is a mismatch even while the runtime-id root does not exist yet. The
    // Runner migrates that root in place; the probe never accepts a second
    // name for the same data, and never records one as legitimate.
    let fixture = new_fixture();
    let machine_named_root = fixture.config.work_root.join("kata").join(MACHINE);
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &machine_named_root);
    fixture.add_task(&container_id(), "RUNNING");
    fixture.write_persist(&container_id(), Some(4242));
    fixture.add_vmm_comm(4242, "qemu-system-x86");
    assert!(!fixture.state_root().exists());

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Inoperable);
    assert_eq!(report.reason.as_deref(), Some("provider_handle_mismatch"));
    let canonical = fixture.check(&report, "canonical_handle");
    assert_eq!(canonical.evidence["durable_bind_ok"], false);
    assert!(
        canonical.evidence["expected_state_root"]
            .as_str()
            .unwrap()
            .ends_with(&format!("kata/{RUNTIME}")),
        "expected the runtime-id root in evidence; full report:\n{}",
        serde_json::to_string_pretty(&report).unwrap()
    );
    assert_eq!(
        canonical.evidence["observed_data_mounts"][0].as_str(),
        Some(machine_named_root.to_str().unwrap())
    );
    assert!(
        canonical
            .evidence
            .get("legacy_machine_named_root")
            .is_none(),
        "no legacy acceptance may be recorded; full report:\n{}",
        serde_json::to_string_pretty(&report).unwrap()
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn machine_named_root_is_a_mismatch_when_the_runtime_id_root_exists() {
    // Same bind, runtime-id root present on disk: still a mismatch, and the
    // evidence names the root the container should have been bound to.
    let fixture = new_fixture();
    let machine_named_root = fixture.config.work_root.join("kata").join(MACHINE);
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &machine_named_root);
    fs::create_dir_all(fixture.state_root()).unwrap();

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Inoperable);
    assert_eq!(report.reason.as_deref(), Some("provider_handle_mismatch"));
    let canonical = fixture.check(&report, "canonical_handle");
    assert_eq!(canonical.evidence["durable_bind_ok"], false);
    assert!(
        canonical.evidence["expected_state_root"]
            .as_str()
            .unwrap()
            .ends_with(&format!("kata/{RUNTIME}")),
        "expected the runtime-id root in evidence; full report:\n{}",
        serde_json::to_string_pretty(&report).unwrap()
    );
    assert_eq!(
        canonical.evidence["observed_data_mounts"][0].as_str(),
        Some(machine_named_root.to_str().unwrap())
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn foreign_machine_named_root_is_still_a_mismatch() {
    // A machine-named root belonging to a DIFFERENT container is never a
    // durable bind for this one, whether or not the runtime-id root exists.
    let fixture = new_fixture();
    let foreign_root = fixture
        .config
        .work_root
        .join("kata")
        .join("finite-kata-somebody-else");
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &foreign_root);

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Inoperable);
    assert_eq!(report.reason.as_deref(), Some("provider_handle_mismatch"));
    let canonical = fixture.check(&report, "canonical_handle");
    assert_eq!(canonical.evidence["durable_bind_ok"], false);
    fixture.assert_nothing_mutated();
}

#[test]
fn empty_runtime_id_is_unknown_and_never_derives_a_machine_named_root() {
    // No runtime id means no durable root to check. The probe must not fall
    // back to the container name, and it must not touch the provider at all.
    let fixture = new_fixture();
    fixture.make_healthy();
    let report = probe_runtime_lifecycle(
        &fixture.config,
        &LifecycleProbeRequest {
            project_id: PROJECT.to_string(),
            agent_runtime_id: "   ".to_string(),
            source_machine_id: MACHINE.to_string(),
        },
    );
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("runtime_id_missing"));
    assert_eq!(
        fixture.check(&report, "containerd_task").status,
        CheckStatus::Skip
    );
    assert!(
        !fixture.state.join("commands.log").exists(),
        "the probe must not consult the provider without a runtime id"
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
    // 2026-08-01 incident topology 1: the container has no containerd task.
    // nerdctl shows it without a task status before and after the targeted
    // read. Here the recorded VMM is gone too, so the retained sandbox and
    // CNI records are stale.
    let fixture = new_fixture();
    fixture.add_runtime_without_task();
    fixture.write_persist(&container_id(), Some(4242));
    fixture.add_netns_record(&container_id());

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Inoperable);
    assert_eq!(report.reason.as_deref(), Some("orphaned_task"));
    let task = fixture.check(&report, "containerd_task");
    assert_eq!(task.finding, Some("orphaned_task"));
    // A missing row alone is never absence: the targeted read confirmed it.
    assert_eq!(fixture.task_reads(), 1);
    assert_eq!(task.evidence["task_manager_absence"], true);
    // The absence is an observation at one instant, and predicts nothing.
    let read_at = task.evidence["task_read_at"].as_str().unwrap();
    assert!(!read_at.is_empty() && task.detail.contains(read_at));
    assert!(!task.detail.contains("would time out"), "{}", task.detail);
    assert_eq!(
        task.evidence["stderr"].as_str(),
        Some(
            format!(
                "ctr: no running task found: task {} not found",
                container_id()
            )
            .as_str()
        )
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
    assert_eq!(
        fixture.check(&report, "vmm_process").status,
        CheckStatus::Pass
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn task_manager_absence_with_a_visible_vmm_never_calls_its_state_stale() {
    // Task-manager absence is not VM absence: containerd can lose its shim
    // while the VM keeps serving and writing. The VMM check still runs, and
    // nothing that VM may be using is called stale.
    let fixture = new_fixture();
    fixture.add_runtime_without_task();
    fixture.write_persist(&container_id(), Some(4242));
    fixture.add_vmm_comm(4242, ".qemu-system-x8");
    fixture.add_netns_record(&container_id());

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Inoperable);
    assert_eq!(report.reason.as_deref(), Some("orphaned_task"));
    let vmm = fixture.check(&report, "vmm_process");
    assert_eq!(vmm.finding, Some("vmm_process_without_task"));
    assert_eq!(vmm.evidence["pid"], 4242);
    let sandbox = fixture.check(&report, "sandbox_state");
    assert_eq!(sandbox.status, CheckStatus::Pass);
    let cni = fixture.check(&report, "cni_namespace");
    assert_eq!(cni.status, CheckStatus::Pass);
    assert_eq!(cni.evidence["stale_evaluated"], false);
    fixture.assert_nothing_mutated();
}

#[test]
fn state_timeout_omitted_from_the_task_list_is_unknown_not_orphaned() {
    // The Pancake topology (2026-10-10): the shim answers Pids and the guest
    // serves, but State times out, so containerd 2.3's List drops the row
    // and ctr's Get-before-Pids fails the same way, as does nerdctl's task
    // load, so its status is empty. The VM is live; nothing here may read as
    // absent or stale.
    let fixture = new_fixture();
    fixture.add_runtime_without_task();
    fixture.omit_task_from_list(&container_id());
    fixture.fail_task_ps_with("ctr: context deadline exceeded\n");
    fixture.write_persist(&container_id(), Some(4242));
    fixture.add_vmm_comm(4242, ".qemu-system-x8");
    fixture.add_netns_record(&container_id());

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("task_state_unavailable"));
    assert_eq!(fixture.task_reads(), 1);
    let task = fixture.check(&report, "containerd_task");
    assert_eq!(
        task.evidence["stderr"].as_str(),
        Some("ctr: context deadline exceeded")
    );
    for name in ["sandbox_state", "cni_namespace", "vmm_process"] {
        assert_eq!(
            fixture.check(&report, name).status,
            CheckStatus::Pass,
            "{name} must not fail; full report:\n{}",
            serde_json::to_string_pretty(&report).unwrap()
        );
    }
    assert_eq!(
        fixture.check(&report, "vmm_process").evidence["comm"].as_str(),
        Some(".qemu-system-x8")
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn closed_shim_omitted_from_the_task_list_is_unknown_not_closed_or_orphaned() {
    // containerd 2.3 maps a closed shim ttrpc connection to bare NotFound, so
    // ctr prints only the shared `no running task found:` prefix. The text
    // carries no task identity: it is neither absence nor a proven closed
    // channel, and the stderr is kept as evidence. nerdctl's task load gets
    // the same NotFound, so it shows no task status.
    let fixture = new_fixture();
    fixture.add_runtime_without_task();
    fixture.omit_task_from_list(&container_id());
    fixture.fail_task_ps_with("ctr: no running task found: not found\n");
    fixture.write_persist(&container_id(), Some(4242));
    fixture.add_netns_record(&container_id());

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("task_state_unavailable"));
    let task = fixture.check(&report, "containerd_task");
    assert_eq!(
        task.evidence["stderr"].as_str(),
        Some("ctr: no running task found: not found")
    );
    assert_eq!(
        fixture.check(&report, "sandbox_state").status,
        CheckStatus::Pass
    );
    assert_eq!(
        fixture.check(&report, "cni_namespace").status,
        CheckStatus::Pass
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn omitted_task_that_answers_a_targeted_read_is_an_incomplete_inventory() {
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &fixture.state_root());
    fixture.omit_task_from_list(&container_id());
    fixture.write_persist(&container_id(), Some(4242));
    fixture.add_vmm_comm(4242, "qemu-system-x86");

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("task_list_incomplete"));
    assert_eq!(
        fixture.check(&report, "containerd_task").evidence["shim_process_count"],
        1
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn replacement_racing_the_task_read_is_unknown_not_orphaned() {
    // A stop or replacement between the two reads can make containerd
    // truthfully report no task for the old id. Absence is only reported for
    // the handle the probe started from.
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &fixture.state_root());
    fixture.replace_container_during_task_read(MACHINE, "machine-a-replacement-id");

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("task_state_unavailable"));
    assert_eq!(
        fixture.check(&report, "containerd_task").evidence["current_container_id"].as_str(),
        Some("machine-a-replacement-id")
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn new_task_under_the_same_id_after_the_absence_read_is_unknown() {
    // A task can start under the same container id after containerd reported
    // none, whether the container showed a task before (a restart) or not (a
    // start). nerdctl's running status after the read is newer evidence of a
    // task, so the earlier absence is not reported, and nothing the new VM
    // may use is called stale or orphaned.
    for before in ["running", NO_TASK_STATUS] {
        let fixture = new_fixture();
        fixture.add_container(MACHINE, before, PROJECT, MACHINE, &fixture.state_root());
        fixture.start_task_during_task_read(MACHINE);
        fixture.write_persist(&container_id(), Some(5252));
        fixture.add_vmm_comm(5252, ".qemu-system-x8");
        fixture.add_netns_record(&container_id());

        let report = fixture.probe();
        assert_verdict(&report, LifecycleVerdict::Unknown);
        assert_eq!(report.reason.as_deref(), Some("task_state_unavailable"));
        let task = fixture.check(&report, "containerd_task");
        assert_eq!(task.evidence["container_status"].as_str(), Some(before));
        assert_eq!(
            task.evidence["container_status_after_task_read"].as_str(),
            Some("running")
        );
        assert!(task.evidence.get("task_manager_absence").is_none());
        assert!(task.evidence["task_read_at"].as_str().is_some());
        assert_eq!(
            fixture.check(&report, "vmm_process").status,
            CheckStatus::Pass
        );
        assert_eq!(
            fixture.check(&report, "cni_namespace").evidence["stale_evaluated"],
            false
        );
        fixture.assert_nothing_mutated();
    }
}

#[test]
fn same_id_stop_racing_the_task_read_is_unknown_not_orphaned() {
    // A normal stop between the probe's first inspect and its targeted read
    // makes containerd truthfully report no task, and nerdctl then shows the
    // empty no-task status. That is a stop in progress, not an orphan.
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &fixture.state_root());
    fixture.stop_task_during_task_read(MACHINE);

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("task_state_unavailable"));
    let task = fixture.check(&report, "containerd_task");
    assert_eq!(task.evidence["container_status"].as_str(), Some("running"));
    assert_eq!(
        task.evidence["container_status_after_task_read"].as_str(),
        Some(NO_TASK_STATUS)
    );
    assert!(task.evidence.get("task_manager_absence").is_none());
    fixture.assert_nothing_mutated();
}

#[test]
fn container_removed_racing_the_task_read_is_unknown_and_named_vanished() {
    let fixture = new_fixture();
    fixture.add_runtime_without_task();
    fixture.remove_container_during_task_read(MACHINE);

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("task_state_unavailable"));
    let task = fixture.check(&report, "containerd_task");
    assert!(task.evidence["current_container_id"].is_null());
    assert!(task.detail.contains("vanished"), "{}", task.detail);
    fixture.assert_nothing_mutated();
}

#[test]
fn absence_without_an_observed_gone_vmm_never_calls_records_stale() {
    // Task-manager absence for an unchanged task-less container, where the
    // recorded VMM pid is unreadable, held by an unrelated process, missing
    // from persist state, or has no persist state at all: none of these show
    // the VMM gone, so neither sandbox nor CNI records are called stale.
    for mode in ["unreadable", "reused", "no-pid", "no-persist"] {
        let fixture = new_fixture();
        fixture.add_runtime_without_task();
        fixture.add_netns_record(&container_id());
        match mode {
            "unreadable" => {
                fixture.write_persist(&container_id(), Some(4242));
                // A directory at the comm path fails the read with an error
                // other than NotFound.
                fs::create_dir_all(fixture.config.proc_root.join("4242/comm")).unwrap();
            }
            "reused" => {
                fixture.write_persist(&container_id(), Some(4242));
                fixture.add_vmm_comm(4242, "bash");
            }
            "no-pid" => fixture.write_persist(&container_id(), None),
            _ => {}
        }

        let report = fixture.probe();
        assert_verdict(&report, LifecycleVerdict::Inoperable);
        assert_eq!(report.reason.as_deref(), Some("orphaned_task"), "{mode}");
        assert_eq!(
            fixture.check(&report, "sandbox_state").status,
            CheckStatus::Pass,
            "{mode}"
        );
        let cni = fixture.check(&report, "cni_namespace");
        assert_eq!(cni.status, CheckStatus::Pass, "{mode}");
        assert_eq!(cni.evidence["stale_evaluated"], false, "{mode}");
        if mode == "reused" {
            // An unrelated process at the recorded pid is not named the VMM.
            let vmm = fixture.check(&report, "vmm_process");
            assert!(vmm.detail.contains("reused the pid"), "{}", vmm.detail);
        }
        fixture.assert_nothing_mutated();
    }
}

#[test]
fn targeted_task_read_timeout_is_unknown() {
    let mut fixture = new_fixture();
    // Long enough for the fixture's other reads on a contended runner; the
    // blocked read is killed at this bound.
    fixture.config.command_timeout = Duration::from_secs(5);
    fixture.add_runtime_without_task();
    fixture.omit_task_from_list(&container_id());
    fs::write(fixture.state.join("tasks-ps-sleep"), "120").unwrap();
    fixture.write_persist(&container_id(), Some(4242));
    fixture.add_netns_record(&container_id());

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("task_state_unavailable"));
    assert_eq!(
        fixture.check(&report, "cni_namespace").evidence["stale_evaluated"],
        false
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn oversized_task_inventory_is_unknown_without_a_targeted_read() {
    let fixture = new_fixture();
    fixture.add_runtime_without_task();
    fs::write(fixture.state.join("tasks"), "x".repeat(256 * 1024 + 1)).unwrap();
    fixture.add_netns_record(&container_id());

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("task_list_error"));
    assert_eq!(fixture.task_reads(), 0);
    assert_eq!(
        fixture.check(&report, "cni_namespace").evidence["stale_evaluated"],
        false
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn ttrpc_closed_control_channel_is_inoperable() {
    // 2026-08-01 incident topology 2: old-VM shutdown repeatedly failed with
    // `ttrpc: closed`. A listed task passed one State call, yet its shim can
    // still close the channel before the no-op read (older containerd also
    // listed dead shims from metadata); only that read sees it.
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
    // A stopped task whose recorded VMM process is gone leaves its CNI netns
    // records behind.
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "exited", PROJECT, MACHINE, &fixture.state_root());
    fixture.add_task(&container_id(), "STOPPED");
    fixture.write_persist(&container_id(), Some(4242));
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
fn listed_paused_task_with_a_live_vmm_keeps_its_cni_records() {
    // A paused Kata task keeps its VM and network namespace. Its records are
    // not stale, and the paused task still never reads operable.
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "paused", PROJECT, MACHINE, &fixture.state_root());
    fixture.add_task(&container_id(), "PAUSED");
    fixture.write_persist(&container_id(), Some(4242));
    fixture.add_vmm_comm(4242, ".qemu-system-x8");
    fixture.add_netns_record(&container_id());

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Degraded);
    assert_eq!(report.reason.as_deref(), Some("task_not_running"));
    let cni = fixture.check(&report, "cni_namespace");
    assert_eq!(cni.status, CheckStatus::Pass);
    assert_eq!(cni.evidence["stale_evaluated"], false);
    fixture.assert_nothing_mutated();
}

#[test]
fn listed_unknown_task_status_is_unknown_not_degraded() {
    // containerd 2.3 lists a task as UNKNOWN when its shim's State call fails
    // with an error List does not drop. Its state is unknown, not stopped:
    // the verdict is unknown and nothing it may still use is called stale.
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "unknown", PROJECT, MACHINE, &fixture.state_root());
    fixture.add_task(&container_id(), "UNKNOWN");
    fixture.write_persist(&container_id(), Some(4242));
    fixture.add_netns_record(&container_id());

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("task_state_unavailable"));
    let task = fixture.check(&report, "containerd_task");
    assert_eq!(task.evidence["task_status"].as_str(), Some("UNKNOWN"));
    assert_eq!(task.evidence["shim_process_count"], 1);
    let cni = fixture.check(&report, "cni_namespace");
    assert_eq!(cni.status, CheckStatus::Pass);
    assert_eq!(cni.evidence["stale_evaluated"], false);
    fixture.assert_nothing_mutated();
}

#[test]
fn stale_kata_persist_state_is_detected() {
    let fixture = new_fixture();
    fixture.make_healthy();
    // Persist state belonging to a superseded container id.
    fixture.write_persist("superseded-container-id", Some(4242));

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Degraded);
    assert_eq!(report.reason.as_deref(), Some("stale_sandbox_state"));
    let sandbox = fixture.check(&report, "sandbox_state");
    assert_eq!(
        sandbox.evidence["persist_sandbox_container"].as_str(),
        Some("superseded-container-id")
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
    fixture.write_persist(&container_id(), Some(4242));
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
fn nix_wrapped_truncated_qemu_comm_is_accepted() {
    // The fleet's real shape (verified live on lat3): the Nix wrapper plus
    // the kernel's 15-char comm truncation turns
    // `/nix/store/.../qemu-system-x86_64` into comm `.qemu-system-x8`. This
    // exact production value must NOT raise wrapped_vmm_process_name.
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &fixture.state_root());
    fixture.add_task(&container_id(), "RUNNING");
    fixture.write_persist(&container_id(), Some(4242));
    fixture.add_vmm_comm(4242, ".qemu-system-x8");

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Operable);
    assert_eq!(
        fixture.check(&report, "vmm_process").status,
        CheckStatus::Pass,
        "production comm wrongly rejected; full report:\n{}",
        serde_json::to_string_pretty(&report).unwrap()
    );
    fixture.assert_nothing_mutated();
}

#[test]
fn non_qemu_process_at_vmm_pid_is_an_anomaly() {
    // An unrelated process at the recorded VMM pid is a real anomaly, not a
    // wrapped name: the finding must still fire.
    let fixture = new_fixture();
    fixture.add_container(MACHINE, "running", PROJECT, MACHINE, &fixture.state_root());
    fixture.add_task(&container_id(), "RUNNING");
    fixture.write_persist(&container_id(), Some(4242));
    fixture.add_vmm_comm(4242, "sshd");

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Degraded);
    assert_eq!(report.reason.as_deref(), Some("wrapped_vmm_process_name"));
    assert_eq!(
        fixture.check(&report, "vmm_process").evidence["comm"].as_str(),
        Some("sshd")
    );
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
    fixture.add_netns_record(&container_id());

    let report = fixture.probe();
    assert_verdict(&report, LifecycleVerdict::Unknown);
    assert_eq!(report.reason.as_deref(), Some("task_list_error"));
    // An unreadable inventory says nothing about whether records are stale.
    assert_eq!(
        fixture.check(&report, "cni_namespace").status,
        CheckStatus::Pass
    );
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
    fixture.write_persist(&container_id(), None);

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
