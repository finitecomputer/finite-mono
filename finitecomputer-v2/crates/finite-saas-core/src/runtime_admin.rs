//! Explicit operator inputs and runtime inventory views.

use crate::{
    OffboardingPhase, RuntimeCapabilitiesV1, RuntimeHealthProjection, RuntimeSummaryStatus,
};
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminRuntimeUpgradeInput {
    pub admin_verified_email: String,
    pub admin_workos_user_id: String,
    pub project_id: String,
    pub target_runtime_artifact_id: String,
    pub now: Option<String>,
}

/// Operator-only upgrade input that binds enqueueing to the exact Runtime
/// observed during a rollout plan. The binding is checked in the same critical
/// section/transaction that creates the lifecycle request, so a changed active
/// Runtime fails closed instead of upgrading replacement compute.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminRuntimeUpgradeExactInput {
    pub admin_verified_email: String,
    pub admin_workos_user_id: String,
    pub project_id: String,
    pub expected_agent_runtime_id: String,
    pub expected_source_host_id: String,
    pub expected_source_machine_id: String,
    pub target_runtime_artifact_id: String,
    pub now: Option<String>,
}

/// Operator-only retirement input bound to the exact active Runtime observed
/// before enqueueing. Retirement still runs through the normal verified
/// Recovery Snapshot and offboarding lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminRuntimeRetireExactInput {
    pub admin_verified_email: String,
    pub admin_workos_user_id: String,
    pub project_id: String,
    pub expected_agent_runtime_id: String,
    pub expected_source_host_id: String,
    pub expected_source_machine_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeControlExpectedBinding {
    pub agent_runtime_id: String,
    pub source_host_id: String,
    pub source_machine_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminRuntimeControlInput {
    pub admin_verified_email: String,
    pub admin_workos_user_id: String,
    pub project_id: String,
    pub now: Option<String>,
}

/// Exact, operator-attested boundary for removing an unrecoverable legacy
/// Runtime from active inventory. This path never deletes retained Core rows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminArchiveUnrecoverableRuntimeInput {
    pub admin_verified_email: String,
    pub admin_workos_user_id: String,
    pub project_id: String,
    pub expected_agent_runtime_id: String,
    pub expected_source_host_id: String,
    pub expected_source_machine_id: String,
    pub expected_owner_email: String,
    pub operator_observed_compute_absent: bool,
    pub operator_observed_durable_state_absent: bool,
    pub owner_acknowledged_unrecoverable: bool,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnrecoverableRuntimeArchiveReceipt {
    pub project_id: String,
    pub agent_runtime_id: String,
    pub source_host_id: String,
    pub source_machine_id: String,
    pub owner_email: String,
    pub archived_at: String,
    pub revoked_finite_private_key_count: usize,
}

/// Exact, operator-attested repair boundary for a Runtime whose verified
/// retirement receipt is already stored but whose offboarding transaction
/// never ran (the Runtime link is still active with no surviving compute).
/// This path never creates, modifies, or deletes a retirement snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminOffboardRetiredRuntimeInput {
    pub admin_verified_email: String,
    pub admin_workos_user_id: String,
    pub project_id: String,
    pub expected_agent_runtime_id: String,
    pub expected_source_host_id: String,
    pub expected_source_machine_id: String,
    pub expected_owner_email: String,
    pub operator_observed_compute_absent: bool,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetiredRuntimeOffboardReceipt {
    pub project_id: String,
    pub agent_runtime_id: String,
    pub retirement_request_id: String,
    pub retirement_locator: String,
    pub offboarded_at: String,
    pub revoked_finite_private_key_count: usize,
}

/// One provisioned box as seen by dashboard operators.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdminRuntimeOverview {
    pub project_id: String,
    pub project_display_name: String,
    pub owner_email: Option<String>,
    pub agent_runtime_id: String,
    pub source_host_id: String,
    pub source_machine_id: String,
    pub runtime_artifact_id: Option<String>,
    pub runtime_artifact_version_label: Option<String>,
    /// Derived at read time from `runtime_health` freshness
    /// (`derive_runtime_summary_status`); never the lifecycle latch verbatim.
    pub runtime_status: RuntimeSummaryStatus,
    /// The raw lifecycle-latched fact (last control outcome), for operators.
    pub lifecycle_status: RuntimeSummaryStatus,
    pub last_heartbeat_at: Option<String>,
    pub status_updated_at: Option<String>,
    pub runtime_updated_at: String,
    pub hermes_available: Option<bool>,
    pub published_app_urls: Vec<String>,
    pub active_finite_private_key_count: i64,
    pub runtime_link_active: bool,
    pub runtime_capabilities: Option<RuntimeCapabilitiesV1>,
    #[serde(default)]
    pub offboarding_phase: Option<OffboardingPhase>,
    /// Runner-ferried standing readiness, projected at read time. `unknown`
    /// until the runner's standing poller first reports and `stale` once
    /// reports lapse, so this never displays a frozen last-known `ready`.
    pub runtime_health: RuntimeHealthProjection,
}
