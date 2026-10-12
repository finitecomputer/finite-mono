//! Read-only lifecycle-control probe for one Kata/containerd Runtime.
//!
//! An Agent's `/contact` endpoint returning HTTP 200 proves the guest is
//! serving; it says nothing about whether the platform can stop or replace
//! that guest. This probe answers the second question and only the second:
//! it gates upgrade eligibility, never serving.
//!
//! READ-ONLY BY CONSTRUCTION: the only provider commands this module can
//! construct are `nerdctl inspect`, `nerdctl ps --all --format {{.Names}}`,
//! `ctr tasks list`, and `ctr tasks ps`, plus bounded reads of on-disk Kata
//! sandbox state, CNI netns records, and `/proc` entries. On containerd 2.x
//! `ctr tasks list` asks every shim for its State and silently omits a task
//! whose State call fails, so a missing row is not absence; the probe then
//! reads that one task with `ctr tasks ps`, a no-op read the shim answers
//! over ttrpc, the same channel a stop would need. There is no code path
//! here that can stop, signal, restart, remove, or otherwise mutate a
//! runtime, its durable data, or any containerd/CNI/Kata state.
//!
//! Every read is a point-in-time observation. Nothing here fences a task
//! generation: a task can stop, start, or be recreated under the same
//! container id between any two reads, so a report never authorizes acting
//! on what it observed.

use crate::{sanitize_sandbox_name, wait_with_captured_output};
use serde::Serialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Versioned report schema consumed by `scripts/rollout-lat1-runtime-artifact`,
/// `scripts/finite_status.py`, and eventually the provider-neutral lifecycle
/// contract. Additive fields only within `v1`.
pub const LIFECYCLE_PROBE_SCHEMA: &str = "finite.lifecycle-probe.v1";

const MAX_INSPECT_BYTES: usize = 256 * 1024;
const MAX_TASK_LIST_BYTES: usize = 256 * 1024;
const MAX_CONTAINER_LIST_BYTES: usize = 64 * 1024;
const MAX_SANDBOX_STATE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct LifecycleProbeConfig {
    pub nerdctl_bin: PathBuf,
    pub ctr_bin: PathBuf,
    pub namespace: String,
    pub source_host_id: String,
    pub work_root: PathBuf,
    /// Kata sandbox persist state root (`<sandbox_root>/<sandbox-id>/persist.json`;
    /// `/run/vc/sbs` on a real Kata host).
    pub sandbox_root: PathBuf,
    /// CNI netns record root (stale records outlive dead tasks).
    pub netns_root: PathBuf,
    /// Proc filesystem root; configurable so fixtures never touch the host's.
    pub proc_root: PathBuf,
    /// Bound on each individual provider command.
    pub command_timeout: Duration,
    /// Bound on the whole probe, including the N serial inspects of the
    /// duplicate-writer scan.
    pub overall_timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct LifecycleProbeRequest {
    pub project_id: String,
    pub agent_runtime_id: String,
    pub source_machine_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleVerdict {
    Operable,
    Degraded,
    Inoperable,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Skip,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleProbeCheck {
    pub name: &'static str,
    pub status: CheckStatus,
    /// Stable snake_case identifier when the check failed; `None` otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finding: Option<&'static str>,
    pub detail: String,
    pub evidence: serde_json::Value,
}

impl LifecycleProbeCheck {
    fn pass(name: &'static str, detail: impl Into<String>, evidence: serde_json::Value) -> Self {
        Self {
            name,
            status: CheckStatus::Pass,
            finding: None,
            detail: detail.into(),
            evidence,
        }
    }

    fn fail(
        name: &'static str,
        finding: &'static str,
        detail: impl Into<String>,
        evidence: serde_json::Value,
    ) -> Self {
        Self {
            name,
            status: CheckStatus::Fail,
            finding: Some(finding),
            detail: detail.into(),
            evidence,
        }
    }

    fn skip(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: CheckStatus::Skip,
            finding: None,
            detail: detail.into(),
            evidence: serde_json::json!({}),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleProbeReport {
    pub schema: &'static str,
    pub runtime: LifecycleProbeRuntime,
    pub verdict: LifecycleVerdict,
    /// Stable snake_case reason for non-operable verdicts; `None` for operable.
    pub reason: Option<String>,
    pub checks: Vec<LifecycleProbeCheck>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleProbeRuntime {
    pub project_id: String,
    pub agent_runtime_id: String,
    pub source_machine_id: String,
    pub container_name: String,
}

/// The severity a finding contributes to the aggregate verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Severity {
    Degraded,
    Unknown,
    Inoperable,
}

fn finding_severity(finding: &str) -> Severity {
    match finding {
        "provider_handle_missing"
        | "provider_handle_mismatch"
        | "orphaned_task"
        | "control_channel_closed"
        | "vmm_process_without_task"
        | "duplicate_durable_writer" => Severity::Inoperable,
        "provider_handle_invalid"
        | "provider_inspect_error"
        | "task_list_error"
        | "task_list_incomplete"
        | "task_state_unavailable"
        | "control_channel_error"
        | "sandbox_state_unreadable"
        | "topology_scan_error"
        | "cni_inventory_unreadable"
        | "vmm_process_unreadable"
        | "vmm_pid_unavailable"
        | "runtime_id_missing"
        | "probe_deadline_exceeded" => Severity::Unknown,
        _ => Severity::Degraded,
    }
}

/// A container id is joined into on-disk state paths; it must never be able
/// to traverse them.
fn valid_container_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id != ".."
        && !id.contains("..")
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

/// Whether `ctr tasks ps <id>` stderr says containerd's task manager holds no
/// task for exactly this container. containerd 2.3 answers that case with
/// NotFound "task <id> not found" (plugins/services/tasks/local.go
/// `getTaskFromContainer`), which ctr prints as one line under its
/// `no running task found:` prefix. That prefix is shared with every other
/// NotFound: a shim whose ttrpc connection closed surfaces as bare
/// `ctr: no running task found: not found` (core/runtime/v2/shim.go `State`),
/// which carries no task identity and is not absence. So only the whole line
/// for this id counts; any other wording reads unknown.
fn task_manager_reports_absence(stderr: &str, container_id: &str) -> bool {
    let expected = format!("ctr: no running task found: task {container_id} not found");
    stderr.lines().any(|line| line.trim() == expected)
}

/// Whether an observed `/proc/<pid>/comm` can be the expected hypervisor's
/// VMM process. The kernel truncates comm to 15 bytes and the Nix wrapper
/// prepends a dot to the binary name, so the fleet's real QEMU shapes are
/// `qemu-system-x86_64`, `qemu-system-x8`, and `.qemu-system-x8` (verified
/// live on lat3). Matching is therefore on the hypervisor-family prefix
/// after stripping one wrapper dot: every truncation of the real binary name
/// that survives at 15 bytes still carries the `qemu-system-` prefix, and
/// anything that does not (an unrelated process at the VMM pid) is a genuine
/// anomaly, not a wrapped name.
fn vmm_comm_matches_expected(comm: &str, hypervisor_type: &str) -> bool {
    let family = match hypervisor_type {
        "" | "qemu" => "qemu-system-",
        other => other,
    };
    let normalized = comm.strip_prefix('.').unwrap_or(comm);
    normalized.len() >= family.len() && normalized.starts_with(family)
}

/// Run the probe. Always returns a report: internal failures surface as
/// `unknown` verdicts carrying their evidence, never as a hard error, so
/// callers distinguish "probe ran and could not determine" from "probe could
/// not be run at all" (transport/CLI failure, handled by the caller).
pub fn probe_runtime_lifecycle(
    config: &LifecycleProbeConfig,
    request: &LifecycleProbeRequest,
) -> LifecycleProbeReport {
    let container_name = sanitize_sandbox_name(&request.source_machine_id).to_ascii_lowercase();
    let runtime = LifecycleProbeRuntime {
        project_id: request.project_id.clone(),
        agent_runtime_id: request.agent_runtime_id.clone(),
        source_machine_id: request.source_machine_id.clone(),
        container_name: container_name.clone(),
    };
    let probe = Probe { config };
    let checks = probe.run(request, &container_name);

    let mut verdict = LifecycleVerdict::Operable;
    let mut reason: Option<String> = None;
    let mut severity: Option<Severity> = None;
    for check in &checks {
        let Some(finding) = check.finding else {
            continue;
        };
        let candidate = finding_severity(finding);
        if severity.is_none_or(|current| candidate > current) {
            severity = Some(candidate);
            reason = Some(finding.to_string());
        }
    }
    match severity {
        None => {}
        Some(Severity::Inoperable) => verdict = LifecycleVerdict::Inoperable,
        Some(Severity::Unknown) => verdict = LifecycleVerdict::Unknown,
        Some(Severity::Degraded) => verdict = LifecycleVerdict::Degraded,
    }

    LifecycleProbeReport {
        schema: LIFECYCLE_PROBE_SCHEMA,
        runtime,
        verdict,
        reason,
        checks,
    }
}

struct Probe<'a> {
    config: &'a LifecycleProbeConfig,
}

#[derive(Debug, serde::Deserialize)]
struct ProbeInspect {
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "Config")]
    config: ProbeInspectConfig,
    #[serde(rename = "State")]
    state: ProbeInspectState,
    #[serde(rename = "Mounts", default)]
    mounts: Vec<ProbeInspectMount>,
}

#[derive(Debug, serde::Deserialize)]
struct ProbeInspectConfig {
    #[serde(rename = "Labels", default)]
    labels: BTreeMap<String, String>,
    #[serde(rename = "Image", default)]
    image: String,
}

#[derive(Debug, serde::Deserialize)]
struct ProbeInspectState {
    #[serde(rename = "Status", default)]
    status: String,
}

#[derive(Debug, serde::Deserialize)]
struct ProbeInspectMount {
    #[serde(rename = "Source")]
    source: PathBuf,
    #[serde(rename = "Destination")]
    destination: PathBuf,
    #[serde(rename = "RW", default)]
    read_write: bool,
}

/// The readable subset of Kata's Go-serialized `persistapi.SandboxState`
/// (`<sandbox_root>/<sandbox-id>/persist.json`, tagless capitalized fields).
/// The on-disk directory name is the sandbox id; `SandboxContainer` is the
/// **container id** the sandbox was created for (verified on the live fleet:
/// it equals the nerdctl container id and the state-dir hash, not the
/// container name), and `HypervisorState.Pid` is the VMM process.
#[derive(Debug, serde::Deserialize)]
struct SandboxPersist {
    #[serde(rename = "State", default)]
    state: String,
    #[serde(rename = "SandboxContainer", default)]
    sandbox_container: String,
    #[serde(rename = "HypervisorState", default)]
    hypervisor: HypervisorPersist,
}

#[derive(Debug, Default, serde::Deserialize)]
struct HypervisorPersist {
    #[serde(rename = "Pid")]
    pid: Option<u64>,
    #[serde(rename = "Type", default)]
    hypervisor_type: String,
}

struct CanonicalHandle {
    container_name: String,
    container_id: String,
    status: String,
    state_root: PathBuf,
}

enum TaskState {
    Running,
    Other,
    /// When read, containerd's task manager held no task for this container,
    /// and nerdctl showed no task status before or after that read. That says
    /// nothing about whether its VM or durable-state writer is gone.
    Absent,
    /// The task inventory could not say: the read failed, the task was
    /// listed as UNKNOWN, or it was omitted from the list without a
    /// confirmed absence.
    Unknown,
}

/// What the VMM check established about the sandbox's recorded VMM process.
/// Retained sandbox and CNI records are stale only once that VMM is gone.
#[derive(Clone, Copy, PartialEq, Eq)]
enum VmmObservation {
    Visible,
    Gone,
    Unobserved,
}

/// Result of the shim-exercising no-op read (`ctr tasks ps`).
enum ChannelState {
    Live(usize),
    /// The incident signature (`ttrpc: closed`): the channel a stop needs is dead.
    Closed(String),
    Error(String),
}

impl Probe<'_> {
    fn run(
        &self,
        request: &LifecycleProbeRequest,
        container_name: &str,
    ) -> Vec<LifecycleProbeCheck> {
        let mut checks = Vec::new();
        let started = Instant::now();
        let deadline = || started.elapsed() >= self.config.overall_timeout;

        // 1. Control-state consistency, part one: the provider handle exists
        // and its ownership labels and durable /data bind match Core's
        // durable record (the request identity). Core sets the RuntimeSpec's
        // durable_state_id to the agent runtime id, so the durable state root
        // is named by the runtime id and by nothing else
        // (kata_launch_plan_for_source_machine derives it the same way and
        // refuses to plan without one). A request without a runtime id
        // therefore has no state root to check, not a machine-named one.
        let durable_state_id = sanitize_sandbox_name(&request.agent_runtime_id);
        if durable_state_id.is_empty() {
            checks.push(LifecycleProbeCheck::fail(
                "canonical_handle",
                "runtime_id_missing",
                "the probe request names no agent runtime id; the durable state root is derived from it alone, never from the container name",
                serde_json::json!({"container_name": container_name}),
            ));
            skip_handle_dependent_checks(&mut checks);
            return checks;
        }
        let state_root = self.config.work_root.join("kata").join(durable_state_id);
        // Every canonical-handle failure leaves no trustworthy handle, so all
        // dependent reads are gated on it.
        let canonical = match self.check_canonical_handle(request, container_name, &state_root) {
            Ok((check, handle)) => {
                checks.push(check);
                handle
            }
            Err(check) => {
                checks.push(check);
                skip_handle_dependent_checks(&mut checks);
                return checks;
            }
        };

        // 2. Control-channel liveness: the containerd task exists and the
        // shim answers a no-op read. This never signals the task.
        let (task_check, task_state) = self.check_containerd_task(&canonical);
        checks.push(task_check);

        // 3. Kata sandbox persist state is readable and self-consistent.
        let (sandbox_check, persist) = self.check_sandbox_state(&canonical, &task_state);
        let sandbox_index = checks.len();
        checks.push(sandbox_check);

        // The duplicate-writer scan issues one inspect per inventory member;
        // an overall budget keeps a large or slow inventory from turning the
        // probe into a hang. An exceeded budget is evidence, not silence.
        if deadline() {
            checks.push(LifecycleProbeCheck::fail(
                "duplicate_writers",
                "probe_deadline_exceeded",
                "the probe exceeded its overall deadline before the duplicate-writer scan",
                serde_json::json!({"overall_timeout_secs": self.config.overall_timeout.as_secs()}),
            ));
            for name in ["cni_namespace", "vmm_process"] {
                checks.push(LifecycleProbeCheck::skip(name, "probe deadline exceeded"));
            }
            return checks;
        }

        // 4. Duplicate-writer check: no second container owns this source
        // machine or mounts the same durable root (a second VM mounting the
        // same durable data is also a second writer of the same npub, since
        // the Agent identity lives in that data).
        checks.push(self.check_duplicate_writers(request, container_name, &canonical, &deadline));

        // 5. VMM process visibility. Name-based process matching is known to
        // miss wrapped QEMU process names, so the probe records what it can
        // actually see instead of matching on names alone. It is read before
        // the CNI check because retained records are stale only once the VMM
        // they belong to is gone; task-manager absence alone does not show it.
        let (vmm_check, vmm) = self.check_vmm_process(persist.as_ref(), &task_state);

        // 6. Stale CNI netns records only matter when the task they belong
        // to is gone or unhealthy.
        checks.push(self.check_cni_namespace(&canonical, &task_state, vmm));
        if matches!(task_state, TaskState::Absent)
            && vmm == VmmObservation::Gone
            && let Some(persist) = persist.as_ref()
        {
            checks[sandbox_index] = self.stale_sandbox_check(&canonical, persist);
        }
        checks.push(vmm_check);

        checks
    }

    fn check_canonical_handle(
        &self,
        request: &LifecycleProbeRequest,
        container_name: &str,
        state_root: &Path,
    ) -> Result<(LifecycleProbeCheck, CanonicalHandle), LifecycleProbeCheck> {
        let inspected = match self.inspect(container_name) {
            Ok(Some(inspected)) => inspected,
            Ok(None) => {
                return Err(LifecycleProbeCheck::fail(
                    "canonical_handle",
                    "provider_handle_missing",
                    format!("no container named {container_name} in the provider inventory"),
                    serde_json::json!({"container_name": container_name}),
                ));
            }
            Err(error) => {
                return Err(LifecycleProbeCheck::fail(
                    "canonical_handle",
                    "provider_inspect_error",
                    format!("could not inspect {container_name}: {error}"),
                    serde_json::json!({"container_name": container_name, "error": error}),
                ));
            }
        };
        let expected_labels = [
            ("computer.finite.v2.runtime", "true"),
            (
                "computer.finite.v2.source_host_id",
                self.config.source_host_id.as_str(),
            ),
            ("computer.finite.v2.source_machine_id", container_name),
            ("computer.finite.v2.project_id", request.project_id.as_str()),
        ];
        let mismatched: Vec<&str> = expected_labels
            .iter()
            .filter(|(key, value)| {
                inspected.config.labels.get(*key).map(String::as_str) != Some(*value)
            })
            .map(|(key, _)| *key)
            .collect();
        // The expectation is derived (runtime-id state root), but the truth
        // carried downstream is the provider-observed mount: the duplicate
        // writer scan compares against what is actually mounted.
        //
        // The durable root has exactly one name, the runtime id. A container
        // bound to a machine-named root — its own or anyone else's — is a
        // mismatch whether or not the runtime-id root exists yet: the Runner
        // moves a pre-runtime-id-era root to its runtime-id path in place
        // (one rename, once) and until that has happened this reads
        // inoperable rather than accepting a second name for the same data.
        let durable_bind = inspected.mounts.iter().find(|mount| {
            mount.destination == Path::new("/data")
                && mount.read_write
                && mount.source == state_root
        });
        if !mismatched.is_empty() || durable_bind.is_none() {
            return Err(LifecycleProbeCheck::fail(
                "canonical_handle",
                "provider_handle_mismatch",
                format!("container {container_name} does not match Core's durable record"),
                serde_json::json!({
                    "container_name": container_name,
                    "mismatched_labels": mismatched,
                    "expected_state_root": state_root,
                    "durable_bind_ok": durable_bind.is_some(),
                    "observed_data_mounts": inspected
                        .mounts
                        .iter()
                        .filter(|mount| mount.destination == Path::new("/data"))
                        .map(|mount| mount.source.clone())
                        .collect::<Vec<_>>(),
                }),
            ));
        }
        let observed_state_root = durable_bind
            .expect("durable bind checked above")
            .source
            .clone();
        if !valid_container_id(&inspected.id) {
            return Err(LifecycleProbeCheck::fail(
                "canonical_handle",
                "provider_handle_invalid",
                format!(
                    "container {container_name} reported an unusable container id; refusing to derive state paths from it"
                ),
                serde_json::json!({"container_name": container_name}),
            ));
        }
        Ok((
            LifecycleProbeCheck::pass(
                "canonical_handle",
                format!("container {container_name} matches Core's durable record"),
                serde_json::json!({
                    "container_name": container_name,
                    "container_id": inspected.id,
                    "status": inspected.state.status,
                    "image": inspected.config.image,
                    "state_root": observed_state_root,
                }),
            ),
            CanonicalHandle {
                container_name: container_name.to_string(),
                container_id: inspected.id,
                status: inspected.state.status,
                state_root: observed_state_root,
            },
        ))
    }

    fn check_containerd_task(
        &self,
        canonical: &CanonicalHandle,
    ) -> (LifecycleProbeCheck, TaskState) {
        let output = match self.execute_read(
            &self.config.ctr_bin,
            vec![
                OsString::from("--namespace"),
                OsString::from(self.config.namespace.trim()),
                OsString::from("tasks"),
                OsString::from("list"),
            ],
        ) {
            Ok(output) => output,
            Err(error) => {
                return (
                    LifecycleProbeCheck::fail(
                        "containerd_task",
                        "task_list_error",
                        format!("could not read the containerd task inventory: {error}"),
                        serde_json::json!({"error": error}),
                    ),
                    TaskState::Unknown,
                );
            }
        };
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if !output.status.success() {
            let combined = format!("{stdout} {stderr}").to_ascii_lowercase();
            if combined.contains("ttrpc") {
                // The incident signature: the shim's control channel is
                // closed, so a normal stop would fail the same way.
                return (
                    LifecycleProbeCheck::fail(
                        "containerd_task",
                        "control_channel_closed",
                        "the containerd control channel rejected a no-op status read",
                        serde_json::json!({
                            "container_id": canonical.container_id,
                            "stderr": stderr.trim(),
                        }),
                    ),
                    TaskState::Unknown,
                );
            }
            return (
                LifecycleProbeCheck::fail(
                    "containerd_task",
                    "task_list_error",
                    "the containerd task inventory read failed",
                    serde_json::json!({
                        "container_id": canonical.container_id,
                        "stderr": stderr.trim(),
                    }),
                ),
                TaskState::Unknown,
            );
        }
        if stdout.len() > MAX_TASK_LIST_BYTES {
            return (
                LifecycleProbeCheck::fail(
                    "containerd_task",
                    "task_list_error",
                    "the containerd task inventory exceeded its bounded limit",
                    serde_json::json!({"bytes": stdout.len()}),
                ),
                TaskState::Unknown,
            );
        }
        let mut status: Option<String> = None;
        for line in stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let mut columns = line.split_whitespace();
            let Some(task_id) = columns.next() else {
                continue;
            };
            if task_id == "TASK" || task_id != canonical.container_id {
                continue;
            }
            status = columns.last().map(str::to_string);
        }
        let Some(status) = status else {
            return self.check_unlisted_task(canonical);
        };
        // containerd lists a task as UNKNOWN when its shim's State call
        // failed with an error List does not drop, or returned a state it
        // does not recognise (plugins/services/tasks/local.go
        // `getProcessState`): the task exists, but its state is not known.
        let task_state = if status.eq_ignore_ascii_case("running") {
            TaskState::Running
        } else if status.eq_ignore_ascii_case("unknown") {
            TaskState::Unknown
        } else {
            TaskState::Other
        };
        // A listed task passed one State call; `tasks ps` is a no-op read the
        // shim answers over ttrpc, the same channel a stop would need.
        match self.task_ps(&canonical.container_id) {
            ChannelState::Live(process_count) => {
                let evidence = serde_json::json!({
                    "container_id": canonical.container_id,
                    "task_status": status,
                    "shim_process_count": process_count,
                });
                let check = match task_state {
                    TaskState::Running => LifecycleProbeCheck::pass(
                        "containerd_task",
                        "containerd task exists and the shim answers a no-op read",
                        evidence,
                    ),
                    TaskState::Unknown => LifecycleProbeCheck::fail(
                        "containerd_task",
                        "task_state_unavailable",
                        "containerd lists this task as UNKNOWN, so it could not read the task's state; the shim answered a no-op read",
                        evidence,
                    ),
                    _ => LifecycleProbeCheck::fail(
                        "containerd_task",
                        "task_not_running",
                        format!("containerd task exists but its status is {status}"),
                        evidence,
                    ),
                };
                (check, task_state)
            }
            ChannelState::Closed(detail) => (
                LifecycleProbeCheck::fail(
                    "containerd_task",
                    "control_channel_closed",
                    "the shim control channel rejected a no-op read; a normal stop would fail the same way",
                    serde_json::json!({
                        "container_id": canonical.container_id,
                        "task_status": status,
                        "stderr": detail,
                    }),
                ),
                task_state,
            ),
            ChannelState::Error(detail) => (
                LifecycleProbeCheck::fail(
                    "containerd_task",
                    "control_channel_error",
                    "the shim control channel read failed",
                    serde_json::json!({
                        "container_id": canonical.container_id,
                        "task_status": status,
                        "stderr": detail,
                    }),
                ),
                task_state,
            ),
        }
    }

    /// A row missing from `ctr tasks list` is not absence. containerd 2.x
    /// List calls each shim's State under a short timeout and omits any task
    /// whose State fails (NotFound, Unavailable or DeadlineExceeded) while
    /// the RPC itself succeeds (plugins/services/tasks/local.go `addTasks`).
    /// One targeted read of this task separates the cases. Only containerd's
    /// own "task <id> not found" is read as absence, and only when nerdctl
    /// showed the same container with no task status both before and after
    /// it. nerdctl 2.3 reports a status only after loading the container's
    /// task and reading that task's status, and an empty status otherwise
    /// (pkg/containerinspector `Inspect`, inspecttypes/dockercompat
    /// `ContainerFromNative`). A non-empty status is therefore newer evidence
    /// of a task, and a changed status is a stop, start or restart racing the
    /// reads; both read unknown. Even a consistent result is task-manager
    /// absence at one instant: rechecking the id and status narrows the race
    /// but is no generation fence.
    fn check_unlisted_task(&self, canonical: &CanonicalHandle) -> (LifecycleProbeCheck, TaskState) {
        let channel = self.task_ps(&canonical.container_id);
        let task_read_at = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default();
        let stderr = match channel {
            ChannelState::Live(process_count) => {
                return (
                    LifecycleProbeCheck::fail(
                        "containerd_task",
                        "task_list_incomplete",
                        "the containerd task inventory omitted this task, but a targeted read answered; the inventory is incomplete",
                        serde_json::json!({
                            "container_id": canonical.container_id,
                            "container_status": canonical.status,
                            "shim_process_count": process_count,
                            "task_read_at": task_read_at,
                        }),
                    ),
                    TaskState::Unknown,
                );
            }
            ChannelState::Closed(stderr) | ChannelState::Error(stderr) => stderr,
        };
        let unavailable = |detail: String, evidence: serde_json::Value| {
            (
                LifecycleProbeCheck::fail(
                    "containerd_task",
                    "task_state_unavailable",
                    detail,
                    evidence,
                ),
                TaskState::Unknown,
            )
        };
        if !task_manager_reports_absence(&stderr, &canonical.container_id) {
            return unavailable(
                "the containerd task inventory omitted this task and a targeted read failed; the task may still be running".to_string(),
                serde_json::json!({
                    "container_id": canonical.container_id,
                    "container_status": canonical.status,
                    "stderr": stderr,
                    "task_read_at": task_read_at,
                }),
            );
        }
        let current = match self.inspect(&canonical.container_name) {
            Ok(Some(current)) => current,
            Ok(None) => {
                return unavailable(
                    "the container vanished while the probe read its task".to_string(),
                    serde_json::json!({
                        "container_id": canonical.container_id,
                        "current_container_id": null,
                        "container_status": canonical.status,
                        "stderr": stderr,
                        "task_read_at": task_read_at,
                    }),
                );
            }
            Err(error) => {
                return unavailable(
                    "could not re-read the provider handle after its task read".to_string(),
                    serde_json::json!({
                        "container_id": canonical.container_id,
                        "error": error,
                        "stderr": stderr,
                        "task_read_at": task_read_at,
                    }),
                );
            }
        };
        let mut evidence = serde_json::json!({
            "container_id": canonical.container_id,
            "current_container_id": current.id,
            "container_status": canonical.status,
            "container_status_after_task_read": current.state.status,
            "stderr": stderr,
            "task_read_at": task_read_at,
        });
        if current.id != canonical.container_id {
            return unavailable(
                "the container was replaced under a new container id while the probe read its task"
                    .to_string(),
                evidence,
            );
        }
        if current.state.status != canonical.status {
            return unavailable(
                format!(
                    "the container's status changed from {:?} to {:?} while the probe read its task; a stop, start or restart raced the read",
                    canonical.status, current.state.status
                ),
                evidence,
            );
        }
        if !current.state.status.is_empty() {
            return unavailable(
                format!(
                    "containerd's task manager reported no task, but nerdctl reported task status {} for this container before and after that read; a task may have been recreated under the same id",
                    current.state.status
                ),
                evidence,
            );
        }
        evidence["task_manager_absence"] = serde_json::json!(true);
        (
            LifecycleProbeCheck::fail(
                "containerd_task",
                "orphaned_task",
                format!(
                    "at {task_read_at} containerd's task manager held no task for this container, and nerdctl showed no task status before or after that read. This is a point-in-time observation, not a fence: a task can start under this id afterwards. It does not show that the VM or durable-state writer is gone"
                ),
                evidence,
            ),
            TaskState::Absent,
        )
    }

    fn task_ps(&self, container_id: &str) -> ChannelState {
        let output = match self.execute_read(
            &self.config.ctr_bin,
            vec![
                OsString::from("--namespace"),
                OsString::from(self.config.namespace.trim()),
                OsString::from("tasks"),
                OsString::from("ps"),
                OsString::from(container_id),
            ],
        ) {
            Ok(output) => output,
            Err(error) => return ChannelState::Error(error),
        };
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if !output.status.success() {
            let combined = format!("{stdout} {stderr}").to_ascii_lowercase();
            if combined.contains("ttrpc") {
                return ChannelState::Closed(stderr.trim().to_string());
            }
            return ChannelState::Error(stderr.trim().to_string());
        }
        if stdout.len() > MAX_TASK_LIST_BYTES {
            return ChannelState::Error("process list exceeded its bounded limit".to_string());
        }
        ChannelState::Live(
            stdout
                .lines()
                .filter(|line| !line.trim().is_empty() && !line.starts_with("PID"))
                .count(),
        )
    }

    fn sandbox_persist_path(&self, canonical: &CanonicalHandle) -> PathBuf {
        self.config
            .sandbox_root
            .join(&canonical.container_id)
            .join("persist.json")
    }

    fn check_sandbox_state(
        &self,
        canonical: &CanonicalHandle,
        task_state: &TaskState,
    ) -> (LifecycleProbeCheck, Option<SandboxPersist>) {
        let path = self.sandbox_persist_path(canonical);
        let contents = match read_bounded(&path, MAX_SANDBOX_STATE_BYTES) {
            Ok(contents) => contents,
            Err(ReadError::Absent) => {
                return (
                    match task_state {
                        TaskState::Running => LifecycleProbeCheck::fail(
                            "sandbox_state",
                            "sandbox_state_missing",
                            "the task is running but no Kata sandbox persist state exists",
                            serde_json::json!({"path": path}),
                        ),
                        _ => LifecycleProbeCheck::pass(
                            "sandbox_state",
                            "no Kata sandbox persist state remains for this runtime",
                            serde_json::json!({"path": path}),
                        ),
                    },
                    None,
                );
            }
            Err(ReadError::Unreadable(error)) => {
                return (
                    LifecycleProbeCheck::fail(
                        "sandbox_state",
                        "sandbox_state_unreadable",
                        format!("could not read the Kata sandbox persist state: {error}"),
                        serde_json::json!({"path": path, "error": error}),
                    ),
                    None,
                );
            }
        };
        let persist: SandboxPersist = match serde_json::from_slice(&contents) {
            Ok(persist) => persist,
            Err(error) => {
                return (
                    LifecycleProbeCheck::fail(
                        "sandbox_state",
                        "sandbox_state_inconsistent",
                        format!("the Kata sandbox persist state is not self-consistent: {error}"),
                        serde_json::json!({"path": path}),
                    ),
                    None,
                );
            }
        };
        if persist.sandbox_container.is_empty() {
            return (
                LifecycleProbeCheck::fail(
                    "sandbox_state",
                    "sandbox_state_inconsistent",
                    "the Kata sandbox persist state does not name its sandbox container",
                    serde_json::json!({"path": path}),
                ),
                None,
            );
        }
        // Reality-first: the persist record's SandboxContainer is a container
        // id, so it is compared against the live inspected container's actual
        // id from nerdctl, never against the container name.
        if persist.sandbox_container != canonical.container_id {
            return (
                LifecycleProbeCheck::fail(
                    "sandbox_state",
                    "stale_sandbox_state",
                    format!(
                        "the Kata sandbox persist state belongs to container id {} but the live container id is {}",
                        persist.sandbox_container, canonical.container_id
                    ),
                    serde_json::json!({
                        "path": path,
                        "persist_sandbox_container": persist.sandbox_container,
                        "container_id": canonical.container_id,
                    }),
                ),
                None,
            );
        }
        // Without a containerd task this state is stale only if its VMM is
        // gone too; `run` settles that once the VMM check has looked.
        (
            LifecycleProbeCheck::pass(
                "sandbox_state",
                match task_state {
                    TaskState::Running => {
                        "Kata sandbox persist state is readable and names the live sandbox"
                    }
                    TaskState::Absent => {
                        "Kata sandbox persist state names this container although containerd held no task for it; the VMM check decides whether it is stale"
                    }
                    TaskState::Other | TaskState::Unknown => {
                        "Kata sandbox persist state is readable and names this container; it does not show whether the sandbox is live"
                    }
                },
                serde_json::json!({
                    "path": path,
                    "sandbox_container": persist.sandbox_container,
                    "state": persist.state,
                    "hypervisor_type": persist.hypervisor.hypervisor_type,
                }),
            ),
            Some(persist),
        )
    }

    /// containerd held no task and no process was found at the VMM pid its
    /// sandbox recorded, but the sandbox state remains: the stale Kata
    /// persist.json the lifecycle-exception Agent carried.
    fn stale_sandbox_check(
        &self,
        canonical: &CanonicalHandle,
        persist: &SandboxPersist,
    ) -> LifecycleProbeCheck {
        LifecycleProbeCheck::fail(
            "sandbox_state",
            "stale_sandbox_state",
            "containerd held no task and no process was found at the recorded VMM pid, but Kata sandbox persist state remains",
            serde_json::json!({
                "path": self.sandbox_persist_path(canonical),
                "persist_sandbox_container": persist.sandbox_container,
                "container_id": canonical.container_id,
                "vmm_pid": persist.hypervisor.pid,
            }),
        )
    }

    fn check_duplicate_writers(
        &self,
        request: &LifecycleProbeRequest,
        container_name: &str,
        canonical: &CanonicalHandle,
        deadline: &dyn Fn() -> bool,
    ) -> LifecycleProbeCheck {
        let names = match self.container_names() {
            Ok(names) => names,
            Err(error) => {
                return LifecycleProbeCheck::fail(
                    "duplicate_writers",
                    "topology_scan_error",
                    format!("could not enumerate the provider inventory: {error}"),
                    serde_json::json!({"error": error}),
                );
            }
        };
        let mut duplicates = Vec::new();
        for name in names.iter().filter(|name| name.as_str() != container_name) {
            if deadline() {
                return LifecycleProbeCheck::fail(
                    "duplicate_writers",
                    "probe_deadline_exceeded",
                    "the probe exceeded its overall deadline during the duplicate-writer scan",
                    serde_json::json!({
                        "overall_timeout_secs": self.config.overall_timeout.as_secs(),
                        "inventory_size": names.len(),
                    }),
                );
            }
            let inspected = match self.inspect(name) {
                Ok(Some(inspected)) => inspected,
                Ok(None) => continue,
                Err(error) => {
                    return LifecycleProbeCheck::fail(
                        "duplicate_writers",
                        "topology_scan_error",
                        format!("could not inspect topology member {name}: {error}"),
                        serde_json::json!({"container_name": name, "error": error}),
                    );
                }
            };
            let same_source = inspected
                .config
                .labels
                .get("computer.finite.v2.source_machine_id")
                .map(String::as_str)
                == Some(container_name)
                && inspected
                    .config
                    .labels
                    .get("computer.finite.v2.project_id")
                    .map(String::as_str)
                    == Some(request.project_id.as_str());
            let same_durable_root = inspected.mounts.iter().any(|mount| {
                mount.destination == Path::new("/data") && mount.source == canonical.state_root
            });
            if same_source || same_durable_root {
                duplicates.push(serde_json::json!({
                    "container_name": name,
                    "same_source_machine": same_source,
                    "same_durable_root": same_durable_root,
                }));
            }
        }
        if duplicates.is_empty() {
            LifecycleProbeCheck::pass(
                "duplicate_writers",
                "no second runtime owns this source machine or mounts its durable data",
                serde_json::json!({"inventory_size": names.len()}),
            )
        } else {
            LifecycleProbeCheck::fail(
                "duplicate_writers",
                "duplicate_durable_writer",
                format!(
                    "{} other runtime(s) own this source machine or mount its durable data",
                    duplicates.len()
                ),
                serde_json::json!({"duplicates": duplicates}),
            )
        }
    }

    fn check_cni_namespace(
        &self,
        canonical: &CanonicalHandle,
        task_state: &TaskState,
        vmm: VmmObservation,
    ) -> LifecycleProbeCheck {
        let entries = match std::fs::read_dir(&self.config.netns_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return LifecycleProbeCheck::pass(
                    "cni_namespace",
                    "no CNI netns record root exists on this host",
                    serde_json::json!({"netns_root": self.config.netns_root}),
                );
            }
            Err(error) => {
                // An evidence-gathering error is an Unknown-severity finding,
                // never a silent skip: an unreadable inventory must not yield
                // `operable`.
                return LifecycleProbeCheck::fail(
                    "cni_namespace",
                    "cni_inventory_unreadable",
                    format!(
                        "could not list the CNI netns record root {}: {error}",
                        self.config.netns_root.display()
                    ),
                    serde_json::json!({
                        "netns_root": self.config.netns_root,
                        "error": error.to_string(),
                    }),
                );
            }
        };
        let mut records = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.contains(&canonical.container_id) {
                records.push(name);
            }
        }
        // A listed task that is paused, created or stopped can keep its VM and
        // network namespace, and a task-manager absence says nothing about
        // the VM either, so records are stale only once the recorded VMM
        // process is observed gone.
        let stale = matches!(task_state, TaskState::Other | TaskState::Absent)
            && vmm == VmmObservation::Gone;
        if records.is_empty() || matches!(task_state, TaskState::Running) {
            LifecycleProbeCheck::pass(
                "cni_namespace",
                if records.is_empty() {
                    "no CNI netns records reference this runtime".to_string()
                } else {
                    "CNI netns records match the live task".to_string()
                },
                serde_json::json!({"records": records}),
            )
        } else if !stale {
            LifecycleProbeCheck::pass(
                "cni_namespace",
                "CNI netns records remain; they are not called stale unless the task is not running and its recorded VMM process was observed gone",
                serde_json::json!({"records": records, "stale_evaluated": false}),
            )
        } else {
            LifecycleProbeCheck::fail(
                "cni_namespace",
                "stale_cni_namespace",
                "CNI netns records remain although the containerd task is not running and no process was found at its recorded VMM pid",
                serde_json::json!({
                    "records": records,
                    "container_id": canonical.container_id,
                }),
            )
        }
    }

    fn check_vmm_process(
        &self,
        persist: Option<&SandboxPersist>,
        task_state: &TaskState,
    ) -> (LifecycleProbeCheck, VmmObservation) {
        let task_running = matches!(task_state, TaskState::Running);
        let Some(persist) = persist else {
            // The sandbox check already produced a finding; nothing more can
            // be derived here.
            return (
                LifecycleProbeCheck::skip(
                    "vmm_process",
                    "no self-consistent sandbox persist state to derive the VMM pid from",
                ),
                VmmObservation::Unobserved,
            );
        };
        let Some(pid) = persist.hypervisor.pid else {
            if task_running {
                return (
                    LifecycleProbeCheck::fail(
                        "vmm_process",
                        "vmm_pid_unavailable",
                        "the task is running but the sandbox persist state records no hypervisor pid",
                        serde_json::json!({}),
                    ),
                    VmmObservation::Unobserved,
                );
            }
            return (
                LifecycleProbeCheck::skip(
                    "vmm_process",
                    "the sandbox persist state does not record a hypervisor pid",
                ),
                VmmObservation::Unobserved,
            );
        };
        let comm_path = self.config.proc_root.join(pid.to_string()).join("comm");
        let comm = match read_bounded(&comm_path, 4096) {
            Ok(contents) => String::from_utf8_lossy(&contents).trim().to_string(),
            Err(ReadError::Absent) if task_running => {
                return (
                    LifecycleProbeCheck::fail(
                        "vmm_process",
                        "vmm_process_missing",
                        format!("the task is running but no VMM process {pid} exists"),
                        serde_json::json!({"pid": pid}),
                    ),
                    VmmObservation::Gone,
                );
            }
            Err(ReadError::Absent) => {
                return (
                    LifecycleProbeCheck::pass(
                        "vmm_process",
                        format!(
                            "no process was found at the sandbox's recorded VMM pid {pid}; this reads that pid once and does not show that no other VM serves this sandbox"
                        ),
                        serde_json::json!({"pid": pid}),
                    ),
                    VmmObservation::Gone,
                );
            }
            Err(ReadError::Unreadable(error)) => {
                return (
                    LifecycleProbeCheck::fail(
                        "vmm_process",
                        "vmm_process_unreadable",
                        format!("could not read {}: {error}", comm_path.display()),
                        serde_json::json!({"pid": pid, "error": error}),
                    ),
                    VmmObservation::Unobserved,
                );
            }
        };
        // The runner's Kata control path never identifies VMM processes by
        // name (liveness comes from containerd), so this matcher is
        // probe-local and deliberately narrow: it accepts exactly the comm
        // shapes Kata/QEMU on NixOS produces — the full binary name and its
        // 15-char kernel comm truncations, with or without the Nix wrapper's
        // leading dot — and rejects everything else.
        let expected = if persist.hypervisor.hypervisor_type.is_empty() {
            "qemu"
        } else {
            persist.hypervisor.hypervisor_type.as_str()
        };
        let evidence = serde_json::json!({"pid": pid, "comm": comm, "hypervisor_type": expected});
        if !vmm_comm_matches_expected(&comm, expected) {
            return (
                LifecycleProbeCheck::fail(
                    "vmm_process",
                    "wrapped_vmm_process_name",
                    format!(
                        "the process at the sandbox's recorded VMM pid {pid} is visible as {comm}, which does not match a {expected} hypervisor process; it may be the VMM under a wrapped name or an unrelated process that reused the pid"
                    ),
                    evidence,
                ),
                VmmObservation::Unobserved,
            );
        }
        if matches!(task_state, TaskState::Absent) {
            // The name match alone cannot exclude PID reuse; it is enough to
            // stop calling the sandbox stale, not to identify the VM.
            return (
                LifecycleProbeCheck::fail(
                    "vmm_process",
                    "vmm_process_without_task",
                    format!(
                        "containerd held no task, but a process named {comm} is visible at the sandbox's recorded VMM pid {pid}; a name cannot exclude pid reuse, so its VM may still be running and writing durable state"
                    ),
                    evidence,
                ),
                VmmObservation::Visible,
            );
        }
        (
            LifecycleProbeCheck::pass(
                "vmm_process",
                format!(
                    "a process named {comm} is visible at the sandbox's recorded VMM pid {pid}"
                ),
                evidence,
            ),
            VmmObservation::Visible,
        )
    }

    fn inspect(&self, container_name: &str) -> Result<Option<ProbeInspect>, String> {
        let output = self.execute_read(
            &self.config.nerdctl_bin,
            vec![
                OsString::from("--namespace"),
                OsString::from(self.config.namespace.trim()),
                OsString::from("inspect"),
                OsString::from(container_name),
            ],
        )?;
        if !output.status.success() {
            let combined = format!(
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
            .to_ascii_lowercase();
            if combined.contains("not found")
                || combined.contains("no such")
                || combined.contains("does not exist")
            {
                return Ok(None);
            }
            return Err(format!("container inspect failed: {combined}"));
        }
        if output.stdout.len() > MAX_INSPECT_BYTES {
            return Err("container inspect exceeded its bounded limit".to_string());
        }
        let records: Vec<ProbeInspect> = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("nerdctl inspect returned invalid JSON: {error}"))?;
        records
            .into_iter()
            .next()
            .map(Some)
            .ok_or_else(|| format!("nerdctl inspect returned no record for {container_name}"))
    }

    fn container_names(&self) -> Result<Vec<String>, String> {
        let output = self.execute_read(
            &self.config.nerdctl_bin,
            vec![
                OsString::from("--namespace"),
                OsString::from(self.config.namespace.trim()),
                OsString::from("ps"),
                OsString::from("--all"),
                OsString::from("--format"),
                OsString::from("{{.Names}}"),
            ],
        )?;
        if !output.status.success() {
            return Err(format!(
                "container inventory failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        if output.stdout.len() > MAX_CONTAINER_LIST_BYTES {
            return Err("container inventory exceeded its bounded limit".to_string());
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// The single execution choke point. Only called with the fixed read-only
    /// argument vectors above; nothing in this module can construct a
    /// mutating provider invocation.
    fn execute_read(
        &self,
        program: &Path,
        args: Vec<OsString>,
    ) -> Result<std::process::Output, String> {
        let mut command = Command::new(program);
        command.args(&args);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        // Linux can refuse to exec a freshly written executable with ETXTBSY
        // while another thread's fork still holds its write handle between
        // fork and exec (the CI harness hits this on the fixture's fake
        // provider). The condition clears within milliseconds; retry a few
        // times before treating it as a provider failure.
        let mut attempt = 0u32;
        let child = loop {
            match command.spawn() {
                Ok(child) => break child,
                Err(error) if error.raw_os_error() == Some(26) && attempt < 5 => {
                    attempt += 1;
                    std::thread::sleep(Duration::from_millis(20 * u64::from(attempt)));
                }
                Err(error) => return Err(format!("{}: {error}", program.display())),
            }
        };
        wait_with_captured_output(child, program, self.config.command_timeout)
            .map_err(|error| error.to_string())
    }
}

/// Without a trustworthy canonical handle none of the dependent reads can
/// say anything about this runtime; they are recorded as skipped, not absent.
fn skip_handle_dependent_checks(checks: &mut Vec<LifecycleProbeCheck>) {
    for name in [
        "containerd_task",
        "sandbox_state",
        "duplicate_writers",
        "cni_namespace",
        "vmm_process",
    ] {
        checks.push(LifecycleProbeCheck::skip(
            name,
            "canonical provider handle is not established",
        ));
    }
}

#[derive(Debug)]
enum ReadError {
    Absent,
    Unreadable(String),
}

fn read_bounded(path: &Path, max_bytes: u64) -> Result<Vec<u8>, ReadError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(ReadError::Absent),
        Err(error) => return Err(ReadError::Unreadable(error.to_string())),
    };
    if metadata.len() > max_bytes {
        return Err(ReadError::Unreadable(format!(
            "{} exceeds its bounded limit",
            path.display()
        )));
    }
    std::fs::read(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ReadError::Absent
        } else {
            ReadError::Unreadable(error.to_string())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(name: &'static str, finding: Option<&'static str>) -> LifecycleProbeCheck {
        match finding {
            Some(finding) => {
                LifecycleProbeCheck::fail(name, finding, "detail", serde_json::json!({}))
            }
            None => LifecycleProbeCheck::pass(name, "detail", serde_json::json!({})),
        }
    }

    #[test]
    fn finding_severities_are_stable() {
        assert_eq!(finding_severity("orphaned_task"), Severity::Inoperable);
        assert_eq!(
            finding_severity("control_channel_closed"),
            Severity::Inoperable
        );
        assert_eq!(
            finding_severity("vmm_process_without_task"),
            Severity::Inoperable
        );
        assert_eq!(finding_severity("task_list_incomplete"), Severity::Unknown);
        assert_eq!(
            finding_severity("task_state_unavailable"),
            Severity::Unknown
        );
        assert_eq!(
            finding_severity("duplicate_durable_writer"),
            Severity::Inoperable
        );
        assert_eq!(finding_severity("task_list_error"), Severity::Unknown);
        assert_eq!(
            finding_severity("wrapped_vmm_process_name"),
            Severity::Degraded
        );
        assert_eq!(
            finding_severity("cni_inventory_unreadable"),
            Severity::Unknown
        );
        assert_eq!(
            finding_severity("vmm_process_unreadable"),
            Severity::Unknown
        );
        assert_eq!(finding_severity("vmm_pid_unavailable"), Severity::Unknown);
        assert_eq!(finding_severity("runtime_id_missing"), Severity::Unknown);
        assert_eq!(
            finding_severity("probe_deadline_exceeded"),
            Severity::Unknown
        );
        assert_eq!(
            finding_severity("provider_handle_invalid"),
            Severity::Unknown
        );
    }

    #[test]
    fn only_containerds_exact_task_not_found_line_is_task_manager_absence() {
        let id = "a".repeat(64);
        // containerd 2.3 getTaskFromContainer, printed by ctr (logrus lines
        // may precede it on stderr).
        assert!(task_manager_reports_absence(
            &format!("ctr: no running task found: task {id} not found\n"),
            &id
        ));
        assert!(task_manager_reports_absence(
            &format!(
                "time=\"…\" level=warning msg=\"unrelated\"\nctr: no running task found: task {id} not found\n"
            ),
            &id
        ));
        for stderr in [
            // Closed shim ttrpc connection: bare NotFound, no task identity.
            "ctr: no running task found: not found\n".to_string(),
            // State timeout while loading the task.
            "ctr: context deadline exceeded\n".to_string(),
            // containerd 1.x clients print a second `: not found` suffix;
            // the matcher is bound to the 2.3 wording and reads it unknown.
            format!("ctr: no running task found: task {id} not found: not found\n"),
            // A different container's absence.
            format!(
                "ctr: no running task found: task {} not found\n",
                "b".repeat(64)
            ),
            // The id as a prefix of another id.
            format!("ctr: no running task found: task {id}0 not found\n"),
            // Container metadata absence is not task-manager absence.
            format!("ctr: container \"{id}\" in namespace \"finite\": not found\n"),
            // A shim-side NotFound that merely mentions the id.
            format!("ctr: no running task found: container does not exist {id}: not found\n"),
            String::new(),
        ] {
            assert!(
                !task_manager_reports_absence(&stderr, &id),
                "must not read as absence: {stderr:?}"
            );
        }
    }

    #[test]
    fn container_ids_never_traverse_state_paths() {
        assert!(valid_container_id("machine-a-id"));
        assert!(valid_container_id(&"a".repeat(64)));
        assert!(!valid_container_id(""));
        assert!(!valid_container_id("../escape"));
        assert!(!valid_container_id(".."));
        assert!(!valid_container_id("a/b"));
        assert!(!valid_container_id("a\\b"));
        assert!(!valid_container_id("a b"));
    }

    #[test]
    fn vmm_comm_matching_accepts_fleet_shapes_and_rejects_strangers() {
        // Full binary name and both kernel-truncated shapes (verified live on
        // lat3: the Nix wrapper yields comm `.qemu-system-x8`).
        assert!(vmm_comm_matches_expected("qemu-system-x86_64", "qemu"));
        assert!(vmm_comm_matches_expected("qemu-system-x8", "qemu"));
        assert!(vmm_comm_matches_expected(".qemu-system-x8", "qemu"));
        assert!(vmm_comm_matches_expected("qemu-system-x86", "qemu"));
        // Empty persist type falls back to the qemu family.
        assert!(vmm_comm_matches_expected(".qemu-system-x8", ""));
        // Non-qemu hypervisor types match their own family prefix.
        assert!(vmm_comm_matches_expected("firecracker", "firecracker"));
        // Genuinely wrong processes at the VMM pid are real anomalies.
        assert!(!vmm_comm_matches_expected("sshd", "qemu"));
        assert!(!vmm_comm_matches_expected("wrap-qemu-syste", "qemu"));
        assert!(!vmm_comm_matches_expected("kata-shim", "qemu"));
        assert!(!vmm_comm_matches_expected("", "qemu"));
        assert!(!vmm_comm_matches_expected(".", "qemu"));
        assert!(!vmm_comm_matches_expected("qemu-syst", "qemu"));
    }

    #[test]
    fn verdict_serialization_is_snake_case() {
        assert_eq!(
            serde_json::to_value(LifecycleVerdict::Operable).unwrap(),
            "operable"
        );
        assert_eq!(
            serde_json::to_value(LifecycleVerdict::Degraded).unwrap(),
            "degraded"
        );
        assert_eq!(
            serde_json::to_value(LifecycleVerdict::Inoperable).unwrap(),
            "inoperable"
        );
        assert_eq!(
            serde_json::to_value(LifecycleVerdict::Unknown).unwrap(),
            "unknown"
        );
    }

    #[test]
    fn report_serializes_every_verdict_with_evidence() {
        let report = LifecycleProbeReport {
            schema: LIFECYCLE_PROBE_SCHEMA,
            runtime: LifecycleProbeRuntime {
                project_id: "project".to_string(),
                agent_runtime_id: "runtime".to_string(),
                source_machine_id: "machine".to_string(),
                container_name: "machine".to_string(),
            },
            verdict: LifecycleVerdict::Inoperable,
            reason: Some("orphaned_task".to_string()),
            checks: vec![check("containerd_task", Some("orphaned_task"))],
        };
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["schema"], "finite.lifecycle-probe.v1");
        assert_eq!(value["verdict"], "inoperable");
        assert_eq!(value["reason"], "orphaned_task");
        assert_eq!(value["checks"][0]["finding"], "orphaned_task");
        assert!(value["checks"][0]["evidence"].is_object());
    }

    #[test]
    fn read_bounded_distinguishes_absent_from_unreadable() {
        let temporary = tempfile::TempDir::new().unwrap();
        let missing = temporary.path().join("missing");
        assert!(matches!(read_bounded(&missing, 16), Err(ReadError::Absent)));
        let present = temporary.path().join("present");
        std::fs::write(&present, b"ok").unwrap();
        assert_eq!(read_bounded(&present, 16).unwrap(), b"ok");
        assert!(matches!(
            read_bounded(&present, 1),
            Err(ReadError::Unreadable(_))
        ));
    }
}
