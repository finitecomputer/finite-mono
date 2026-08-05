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
//! sandbox state, CNI netns records, and `/proc` entries. `ctr tasks list`
//! alone never contacts the shim (containerd answers from metadata, so a
//! dead-shim VM still lists RUNNING), so control-channel liveness uses
//! `ctr tasks ps`, a no-op read the shim answers over ttrpc — the same
//! channel a stop would need. There is no code path here that can stop,
//! signal, restart, remove, or otherwise mutate a runtime, its durable data,
//! or any containerd/CNI/Kata state.

use crate::{sanitize_sandbox_name, wait_with_captured_output};
use serde::Serialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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
        | "duplicate_durable_writer" => Severity::Inoperable,
        "provider_handle_invalid"
        | "provider_inspect_error"
        | "task_list_error"
        | "control_channel_error"
        | "sandbox_state_unreadable"
        | "topology_scan_error"
        | "cni_inventory_unreadable"
        | "vmm_process_unreadable"
        | "vmm_pid_unavailable"
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
/// The on-disk directory name is the sandbox id; `SandboxContainer` names the
/// container the sandbox was created for, and `HypervisorState.Pid` is the
/// VMM process.
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
    container_id: String,
    status: String,
    state_root: PathBuf,
}

enum TaskState {
    Running,
    Other,
    Absent,
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
        // is named by the runtime id, never by the container/machine name
        // (kata_launch_plan_for_source_machine derives it the same way).
        let durable_state_id = sanitize_sandbox_name(&request.agent_runtime_id);
        let state_root = self
            .config
            .work_root
            .join("kata")
            .join(if durable_state_id.is_empty() {
                container_name.to_string()
            } else {
                durable_state_id
            });
        // Every canonical-handle failure leaves no trustworthy handle, so all
        // dependent reads are gated on it.
        let canonical = match self.check_canonical_handle(request, container_name, &state_root) {
            Ok((check, handle)) => {
                checks.push(check);
                handle
            }
            Err(check) => {
                checks.push(check);
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
                return checks;
            }
        };

        // 2. Control-channel liveness: the containerd task exists and the
        // shim answers a no-op read. This never signals the task.
        let (task_check, task_state) = self.check_containerd_task(&canonical);
        let task_running = matches!(task_state, Some(TaskState::Running));
        checks.push(task_check);

        // 3. Kata sandbox persist state is readable and self-consistent.
        let (sandbox_check, persist) =
            self.check_sandbox_state(&canonical, container_name, task_state.as_ref());
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

        // 5. Stale CNI netns records only matter when the task they belong
        // to is gone or unhealthy.
        checks.push(self.check_cni_namespace(&canonical, task_state.as_ref()));

        // 6. VMM process visibility. Name-based process matching is known to
        // miss wrapped QEMU process names, so the probe records what it can
        // actually see instead of matching on names alone.
        checks.push(self.check_vmm_process(persist.as_ref(), task_running));

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
        let durable_bind = inspected.mounts.iter().find(|mount| {
            mount.destination == Path::new("/data")
                && mount.source == state_root
                && mount.read_write
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
                container_id: inspected.id,
                status: inspected.state.status,
                state_root: observed_state_root,
            },
        ))
    }

    fn check_containerd_task(
        &self,
        canonical: &CanonicalHandle,
    ) -> (LifecycleProbeCheck, Option<TaskState>) {
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
                    None,
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
                    None,
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
                None,
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
                None,
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
            return (
                LifecycleProbeCheck::fail(
                    "containerd_task",
                    "orphaned_task",
                    format!(
                        "container claims status {} but no containerd task exists; a normal stop would time out",
                        canonical.status
                    ),
                    serde_json::json!({
                        "container_id": canonical.container_id,
                        "container_status": canonical.status,
                    }),
                ),
                Some(TaskState::Absent),
            );
        };
        let task_state = if status.eq_ignore_ascii_case("running") {
            TaskState::Running
        } else {
            TaskState::Other
        };
        // `tasks list` is answered from containerd metadata and never
        // contacts the shim, so a dead-shim VM still lists RUNNING. `tasks
        // ps` is a no-op read the shim answers over ttrpc — the same channel
        // a stop would need.
        match self.task_ps(&canonical.container_id) {
            ChannelState::Live(process_count) => {
                if matches!(task_state, TaskState::Running) {
                    (
                        LifecycleProbeCheck::pass(
                            "containerd_task",
                            "containerd task exists and the shim answers a no-op read",
                            serde_json::json!({
                                "container_id": canonical.container_id,
                                "task_status": status,
                                "shim_process_count": process_count,
                            }),
                        ),
                        Some(task_state),
                    )
                } else {
                    (
                        LifecycleProbeCheck::fail(
                            "containerd_task",
                            "task_not_running",
                            format!("containerd task exists but its status is {status}"),
                            serde_json::json!({
                                "container_id": canonical.container_id,
                                "task_status": status,
                                "shim_process_count": process_count,
                            }),
                        ),
                        Some(task_state),
                    )
                }
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
                Some(task_state),
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
                Some(task_state),
            ),
        }
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

    fn check_sandbox_state(
        &self,
        canonical: &CanonicalHandle,
        container_name: &str,
        task_state: Option<&TaskState>,
    ) -> (LifecycleProbeCheck, Option<SandboxPersist>) {
        let path = self
            .config
            .sandbox_root
            .join(&canonical.container_id)
            .join("persist.json");
        let contents = match read_bounded(&path, MAX_SANDBOX_STATE_BYTES) {
            Ok(contents) => contents,
            Err(ReadError::Absent) => {
                return (
                    match task_state {
                        Some(TaskState::Running) => LifecycleProbeCheck::fail(
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
        if persist.sandbox_container != container_name {
            return (
                LifecycleProbeCheck::fail(
                    "sandbox_state",
                    "stale_sandbox_state",
                    format!(
                        "the Kata sandbox persist state belongs to {} but the live container is {}",
                        persist.sandbox_container, container_name
                    ),
                    serde_json::json!({
                        "path": path,
                        "persist_sandbox_container": persist.sandbox_container,
                        "container_name": container_name,
                        "container_id": canonical.container_id,
                    }),
                ),
                None,
            );
        }
        if matches!(task_state, Some(TaskState::Absent)) {
            // The task is gone but sandbox state remains: exactly the stale
            // Kata persist.json the lifecycle-exception Agent carried.
            return (
                LifecycleProbeCheck::fail(
                    "sandbox_state",
                    "stale_sandbox_state",
                    "the containerd task is gone but Kata sandbox persist state remains",
                    serde_json::json!({
                        "path": path,
                        "persist_sandbox_container": persist.sandbox_container,
                        "container_name": container_name,
                        "container_id": canonical.container_id,
                    }),
                ),
                None,
            );
        }
        (
            LifecycleProbeCheck::pass(
                "sandbox_state",
                "Kata sandbox persist state is readable and names the live sandbox",
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
        task_state: Option<&TaskState>,
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
        let task_healthy = matches!(task_state, Some(TaskState::Running));
        if records.is_empty() || task_healthy {
            LifecycleProbeCheck::pass(
                "cni_namespace",
                if records.is_empty() {
                    "no CNI netns records reference this runtime".to_string()
                } else {
                    "CNI netns records match the live task".to_string()
                },
                serde_json::json!({"records": records}),
            )
        } else {
            LifecycleProbeCheck::fail(
                "cni_namespace",
                "stale_cni_namespace",
                "CNI netns records remain although the containerd task is not running",
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
        task_running: bool,
    ) -> LifecycleProbeCheck {
        let Some(persist) = persist else {
            // The sandbox check already produced a finding; nothing more can
            // be derived here.
            return LifecycleProbeCheck::skip(
                "vmm_process",
                "no self-consistent sandbox persist state to derive the VMM pid from",
            );
        };
        let Some(pid) = persist.hypervisor.pid else {
            if task_running {
                return LifecycleProbeCheck::fail(
                    "vmm_process",
                    "vmm_pid_unavailable",
                    "the task is running but the sandbox persist state records no hypervisor pid",
                    serde_json::json!({}),
                );
            }
            return LifecycleProbeCheck::skip(
                "vmm_process",
                "the sandbox persist state does not record a hypervisor pid",
            );
        };
        let comm_path = self.config.proc_root.join(pid.to_string()).join("comm");
        let comm = match read_bounded(&comm_path, 4096) {
            Ok(contents) => String::from_utf8_lossy(&contents).trim().to_string(),
            Err(ReadError::Absent) if task_running => {
                return LifecycleProbeCheck::fail(
                    "vmm_process",
                    "vmm_process_missing",
                    format!("the task is running but no VMM process {pid} exists"),
                    serde_json::json!({"pid": pid}),
                );
            }
            Err(ReadError::Absent) => {
                return LifecycleProbeCheck::pass(
                    "vmm_process",
                    format!("no VMM process {pid} remains"),
                    serde_json::json!({"pid": pid}),
                );
            }
            Err(ReadError::Unreadable(error)) => {
                return LifecycleProbeCheck::fail(
                    "vmm_process",
                    "vmm_process_unreadable",
                    format!("could not read {}: {error}", comm_path.display()),
                    serde_json::json!({"pid": pid, "error": error}),
                );
            }
        };
        // Name-based process matching missed wrapped QEMU names during the
        // incident; compare against the hypervisor type the sandbox itself
        // recorded and surface the observed comm as evidence.
        let expected = if persist.hypervisor.hypervisor_type.is_empty() {
            "qemu"
        } else {
            persist.hypervisor.hypervisor_type.as_str()
        };
        if comm.starts_with(expected) {
            LifecycleProbeCheck::pass(
                "vmm_process",
                format!("VMM process {pid} is visible as {comm}"),
                serde_json::json!({"pid": pid, "comm": comm, "hypervisor_type": expected}),
            )
        } else {
            LifecycleProbeCheck::fail(
                "vmm_process",
                "wrapped_vmm_process_name",
                format!(
                    "VMM process {pid} is visible only under the wrapped name {comm}; name-based process matching for {expected} would miss it"
                ),
                serde_json::json!({"pid": pid, "comm": comm, "hypervisor_type": expected}),
            )
        }
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
        let child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("{}: {error}", program.display()))?;
        wait_with_captured_output(child, program, self.config.command_timeout)
            .map_err(|error| error.to_string())
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
