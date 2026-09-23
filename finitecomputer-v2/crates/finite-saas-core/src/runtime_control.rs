//! Runtime-control requests, statuses, and kind-checked completions.

use crate::{
    AgentRuntime, CoreError, CoreResult, RunnerLeaseCapacity, RuntimeArtifact,
    RuntimeCapabilitiesEnvelope, RuntimeRetirementSnapshotReceipt, RuntimeSpecEnvelope,
    id_from_parts, trim_to_option, wire_enum,
};
use serde::Deserialize;
use serde::Serialize;

wire_enum! {
    RuntimeControlKind {
    Restart => "restart",
    RecoverKnownGoodChatRuntime => "recover_known_good_chat_runtime",
    Upgrade => "upgrade",
    Stop => "stop",
    Destroy => "destroy",
    }
    parse: parse_runtime_control_kind
}

/// Canonical runtime-control lifecycle state (2026-08 audit item H1).
///
/// One state machine owns every Runtime control operation:
/// `Requested → Launching → ComputeUp → Ready → Succeeded` for operations
/// that bring compute up (Restart, RecoverKnownGoodChatRuntime, Upgrade),
/// `Requested → Launching → Stopped` for operations that take compute down
/// (Stop, Destroy), and `Failed` (always carrying a named
/// [`RuntimeLifecycleStage`]) from any non-terminal state. `Succeeded`,
/// `Stopped`, and `Failed` are terminal. `succeeded` is only reachable by
/// passing through `Ready`, so it can never again mean "compute exists" —
/// see the 2026-08-18 rollout postmortem (Agent M).
///
/// This enum is hand-written rather than `wire_enum!` because the parse side
/// deliberately accepts the legacy `"running"` wire value as an alias for
/// [`RuntimeControlRequestStatus::Launching`]: N-1 Runner binaries still
/// receive `"running"` inside lease responses from an N-1 Core during the
/// deploy window. Serialization always emits the canonical `"launching"`.
/// Delete condition for the alias: every Runner in the fleet runs a post-H1
/// binary (the same coordination window the `upgrade` kind addition used).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeControlRequestStatus {
    Requested,
    Launching,
    ComputeUp,
    Ready,
    Succeeded,
    Stopped,
    Failed,
}

impl RuntimeControlRequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Launching => "launching",
            Self::ComputeUp => "compute_up",
            Self::Ready => "ready",
            Self::Succeeded => "succeeded",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }

    /// A terminal state has no outgoing transitions; the unique
    /// one-active-per-runtime index and every in-flight scan key off this.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Stopped | Self::Failed)
    }

    pub fn is_active(self) -> bool {
        !self.is_terminal()
    }
}

impl Serialize for RuntimeControlRequestStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RuntimeControlRequestStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        parse_runtime_control_request_status(&value).ok_or_else(|| {
            serde::de::Error::custom(format!("invalid runtime control request status {value}"))
        })
    }
}

pub fn parse_runtime_control_request_status(value: &str) -> Option<RuntimeControlRequestStatus> {
    match value {
        "requested" => Some(RuntimeControlRequestStatus::Requested),
        // N-1 deploy bridge (see the enum's doc comment): a legacy Runner or
        // Core still says "running" for the leased-and-launching phase.
        "launching" | "running" => Some(RuntimeControlRequestStatus::Launching),
        "compute_up" => Some(RuntimeControlRequestStatus::ComputeUp),
        "ready" => Some(RuntimeControlRequestStatus::Ready),
        "succeeded" => Some(RuntimeControlRequestStatus::Succeeded),
        "stopped" => Some(RuntimeControlRequestStatus::Stopped),
        "failed" => Some(RuntimeControlRequestStatus::Failed),
        _ => None,
    }
}

wire_enum! {
/// The named stage a runtime-control operation failed in. Every `failed`
/// request carries one; `Unknown` is reserved for legacy rows and N-1
/// writers that predate named stages.
    RuntimeLifecycleStage {
    Launch => "launch",
    Compute => "compute",
    Readiness => "readiness",
    Retirement => "retirement",
    Unknown => "unknown",
    }
    parse: parse_runtime_lifecycle_stage
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeControlRequest {
    pub id: String,
    pub project_id: String,
    pub agent_runtime_id: String,
    pub source_host_id: String,
    pub source_machine_id: String,
    /// None denotes Core-authorized automatic recovery, never an owner action.
    pub requested_by_user_id: Option<String>,
    pub kind: RuntimeControlKind,
    /// Present only for an explicit Upgrade operation. Restart deliberately
    /// remains bound to the Runtime's current artifact.
    #[serde(default)]
    pub target_runtime_artifact_id: Option<String>,
    pub status: RuntimeControlRequestStatus,
    /// The named failure stage; present exactly when `status` is `Failed`.
    /// `RuntimeLifecycleStage::Unknown` marks legacy rows and N-1 writers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_stage: Option<RuntimeLifecycleStage>,
    pub runner_id: Option<String>,
    pub lease_token: Option<String>,
    pub lease_expires_at: Option<String>,
    pub failure_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeControlLease {
    pub request: RuntimeControlRequest,
    pub runtime: AgentRuntime,
    /// Desired provider-neutral contract for this exact lifecycle operation.
    /// Absent only for N-1 rows during the expand/rollback window.
    #[serde(default)]
    pub runtime_spec: Option<RuntimeSpecEnvelope>,
    /// Core-resolved immutable target for Upgrade. Runner adapters never choose
    /// a product release from process-global configuration while handling an
    /// existing Runtime.
    #[serde(default)]
    pub target_runtime_artifact: Option<RuntimeArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RequestRuntimeRestartInput {
    pub verified_email: String,
    pub workos_user_id: String,
    pub project_id: String,
    pub now: Option<String>,
}

pub type RequestRuntimeRecoverKnownGoodChatInput = RequestRuntimeRestartInput;

pub type RequestRuntimeStopInput = RequestRuntimeRestartInput;

pub type RequestRuntimeDestroyInput = RequestRuntimeRestartInput;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LeaseRuntimeControlRequestInput {
    pub runner_id: String,
    pub lease_token: String,
    pub lease_seconds: Option<i64>,
    pub source_host_id: Option<String>,
    pub runner_capacity: Option<RunnerLeaseCapacity>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RenewRuntimeControlRequestInput {
    pub request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    pub lease_seconds: Option<i64>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompleteRuntimeControlRequestInput {
    pub request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    /// Required for Upgrade and rejected when it does not exactly match the
    /// Core-bound target artifact/schema. Other lifecycle operations leave
    /// these fields empty.
    pub runtime_artifact_id: Option<String>,
    pub state_schema_version: Option<String>,
    /// Optional expand-generation refresh, accepted only on successful Kata
    /// Upgrade completion. Omission preserves the persisted N-1 envelope.
    #[serde(default)]
    pub runtime_capabilities: Option<RuntimeCapabilitiesEnvelope>,
    pub runtime_host: Option<String>,
    pub published_app_urls: Option<Vec<String>>,
    /// Required only for Destroy. Core stores this immutable receipt in the
    /// same transaction that offboards the Runtime.
    #[serde(default)]
    pub retirement_snapshot: Option<RuntimeRetirementSnapshotReceipt>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailRuntimeControlRequestInput {
    pub request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    pub failure_message: String,
    /// Named failure stage. Optional on the wire so N-1 Runners keep working;
    /// Core records `RuntimeLifecycleStage::Unknown` when it is absent.
    #[serde(default)]
    pub failure_stage: Option<RuntimeLifecycleStage>,
    pub now: Option<String>,
}

/// The completion shape of a runtime-control request, parsed once at the
/// store boundary from the flat wire input. The three shapes cannot be
/// confused: Upgrade facts only exist on an Upgrade completion, and the
/// retirement receipt only on a Destroy completion. The flat
/// [`CompleteRuntimeControlRequestInput`] stays the runner wire format
/// unchanged; this enum is what Core's state machine consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeControlCompletion {
    /// Restart, RecoverKnownGoodChatRuntime, and Stop carry no facts.
    Plain,
    /// Upgrade reports the artifact facts it swapped to.
    Upgrade(Box<RuntimeUpgradeCompletionFacts>),
    /// Destroy carries the verified retirement receipt.
    Destroy(Box<RuntimeRetirementSnapshotReceipt>),
}

/// The runner-reported facts of a completed Upgrade. Core validates them
/// against the Core-bound target before they become durable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeUpgradeCompletionFacts {
    pub runtime_artifact_id: String,
    pub state_schema_version: String,
    pub runtime_host: String,
    pub published_app_urls: Vec<String>,
    pub runtime_capabilities: Option<RuntimeCapabilitiesEnvelope>,
}

impl RuntimeControlCompletion {
    /// Parse the flat wire input into the one completion shape the request's
    /// kind allows. Shape/kind confusion is rejected here, once, with the
    /// same errors the previous inline checks produced.
    pub fn parse(
        kind: RuntimeControlKind,
        input: &CompleteRuntimeControlRequestInput,
    ) -> CoreResult<Self> {
        let has_upgrade_facts = input.runtime_artifact_id.is_some()
            || input.state_schema_version.is_some()
            || input.runtime_capabilities.is_some()
            || input.runtime_host.is_some()
            || input.published_app_urls.is_some();
        match kind {
            RuntimeControlKind::Destroy => {
                if has_upgrade_facts {
                    return Err(CoreError::RuntimeUpgradeCompletionMismatch);
                }
                let receipt = input
                    .retirement_snapshot
                    .clone()
                    .ok_or(CoreError::RuntimeRetirementSnapshotMismatch)?;
                Ok(Self::Destroy(Box::new(receipt)))
            }
            RuntimeControlKind::Upgrade => {
                if input.retirement_snapshot.is_some() {
                    return Err(CoreError::RuntimeRetirementSnapshotMismatch);
                }
                Ok(Self::Upgrade(Box::new(RuntimeUpgradeCompletionFacts {
                    runtime_artifact_id: trim_to_option(input.runtime_artifact_id.as_deref())
                        .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?,
                    state_schema_version: trim_to_option(input.state_schema_version.as_deref())
                        .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?,
                    runtime_host: trim_to_option(input.runtime_host.as_deref())
                        .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?,
                    published_app_urls: input
                        .published_app_urls
                        .clone()
                        .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?,
                    runtime_capabilities: input.runtime_capabilities.clone(),
                })))
            }
            RuntimeControlKind::Restart
            | RuntimeControlKind::RecoverKnownGoodChatRuntime
            | RuntimeControlKind::Stop => {
                if input.retirement_snapshot.is_some() {
                    return Err(CoreError::RuntimeRetirementSnapshotMismatch);
                }
                if has_upgrade_facts {
                    return Err(CoreError::RuntimeUpgradeCompletionMismatch);
                }
                Ok(Self::Plain)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetryRuntimeControlRequestInput {
    pub request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    pub failure_message: String,
    pub now: Option<String>,
}

pub(crate) fn runtime_control_request_id_for(
    agent_runtime_id: &str,
    kind: RuntimeControlKind,
    created_at: &str,
) -> String {
    id_from_parts(
        "runtime_ctl",
        &[agent_runtime_id, kind.as_str(), created_at],
    )
}
