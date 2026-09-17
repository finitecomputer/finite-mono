//! Agent admission requests, launch configuration, leases, and completion inputs.

use crate::{
    HostingTier, InFlightCapacityReservationEnvelope, Project, ProviderOperationEnvelope,
    ProviderRuntimeHandleEnvelope, RunnerClass, RunnerLeaseCapacity, RuntimeCapabilitiesEnvelope,
    RuntimePlacement, RuntimeRelocationEnvelope, RuntimeSpecEnvelope, RuntimeSummaryStatus,
    wire_enum,
};
use serde::Deserialize;
use serde::Serialize;

pub(crate) const DEFAULT_AGENT_CREATION_LEASE_SECONDS: i64 = 10 * 60;

pub(crate) const MAX_AGENT_CREATION_LEASE_SECONDS: i64 = 60 * 60;

wire_enum! {
    AgentCreationRequestStatus {
    Requested => "requested",
    Launching => "launching",
    Running => "running",
    Failed => "failed",
    Cancelled => "cancelled",
    }
    parse: parse_agent_creation_request_status
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCreationEntitlement {
    pub id: String,
    pub customer_org_id: String,
    #[serde(default)]
    pub hosting_tier: Option<HostingTier>,
    pub allowed_new_agent_runtimes: i32,
    pub launch_code: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCreationRequest {
    pub id: String,
    pub customer_org_id: String,
    pub owner_user_id: String,
    pub project_id: String,
    pub idempotency_key: String,
    pub display_name: String,
    /// Legacy dual-write retained for the N-1 Runner during expansion.
    pub runner_class: RunnerClass,
    #[serde(default)]
    pub hosting_tier: Option<HostingTier>,
    #[serde(default)]
    pub placement: Option<RuntimePlacement>,
    #[serde(default)]
    pub desired_runtime_artifact_id: Option<String>,
    #[serde(default)]
    pub runtime_spec: Option<RuntimeSpecEnvelope>,
    /// Optional creation-queue partition. Relocation always names its target
    /// host; ordinary creation remains unpinned.
    #[serde(default)]
    pub target_source_host_id: Option<String>,
    /// Operator-only cold relocation contract. Ordinary creation leaves this
    /// absent. The target Runner must verify the staged durable state and the
    /// existing Agent Principal before Core replaces the source binding.
    #[serde(default)]
    pub relocation: Option<RuntimeRelocationEnvelope>,
    pub profile_picture_url: Option<String>,
    /// Owner hosted-chat account id (64 lowercase hex), submitted by the
    /// dashboard at creation time. Injected into the lease-time runtime spec
    /// environment as `FINITECHAT_OWNER_NPUBS`; absent keeps the legacy
    /// allow-all chat admission for pre-existing requests.
    #[serde(default)]
    pub owner_chat_account_id: Option<String>,
    pub status: AgentCreationRequestStatus,
    pub requested_launch_code: Option<String>,
    pub agent_runtime_id: Option<String>,
    pub runner_id: Option<String>,
    pub lease_token: Option<String>,
    pub lease_expires_at: Option<String>,
    pub failure_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RequestAgentCreationInput {
    pub verified_email: String,
    pub workos_user_id: String,
    pub display_name: String,
    pub launch_code: String,
    pub idempotency_key: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCreationConfiguration {
    /// Internal-only placement override for local/provider conformance tests.
    /// User-scoped HTTP requests never populate this field.
    pub placement: Option<RuntimePlacement>,
    /// Customer-visible product choice. Core compares it with the tier granted
    /// by billing or the submitted Launch Code before creating any durable
    /// agent state; provider placement remains Core-owned.
    pub requested_hosting_tier: Option<HostingTier>,
    pub profile_picture_url: Option<String>,
    /// Owner hosted-chat account id (64 hex), pre-minted and submitted by the
    /// dashboard so the lease-time runtime spec can carry
    /// `FINITECHAT_OWNER_NPUBS`. Absent keeps legacy allow-all chat admission.
    pub owner_chat_account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RequestAgentCreationResult {
    pub project: Project,
    pub request: AgentCreationRequest,
    pub reused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LeaseAgentCreationRequestInput {
    pub runner_id: String,
    pub lease_token: String,
    pub lease_seconds: Option<i64>,
    pub runner_capacity: Option<RunnerLeaseCapacity>,
    /// Partition key for the claim: a runner declaring a source host only leases
    /// requests routable to it (a request's `target_source_host_id` is `NULL` =
    /// any runner, else must match). `None` preserves the shared-pool default.
    #[serde(default)]
    pub source_host_id: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCreationLease {
    pub project: Project,
    pub request: AgentCreationRequest,
    /// Present after a current runner reserves its provider correlation. N-1
    /// workers ignore the additive field; re-leases receive the exact durable
    /// acknowledgment needed to reconcile an interrupted provider call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_operation: Option<ProviderOperationEnvelope>,
    /// Present for Runner classes whose provider inventory can lag a paid
    /// creation. Current Phala workers require this acknowledgement before
    /// their first provider mutation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_flight_capacity_reservation: Option<InFlightCapacityReservationEnvelope>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompleteAgentCreationRequestInput {
    pub request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    pub source_host_id: String,
    pub source_machine_id: String,
    pub runtime_artifact_id: Option<String>,
    pub state_schema_version: Option<String>,
    #[serde(default)]
    pub provider_runtime_handle: Option<ProviderRuntimeHandleEnvelope>,
    #[serde(default)]
    pub contact_endpoint: Option<String>,
    #[serde(default)]
    pub runtime_capabilities: Option<RuntimeCapabilitiesEnvelope>,
    pub display_name: Option<String>,
    pub hostname: Option<String>,
    pub runtime_host: Option<String>,
    pub runtime_status: Option<RuntimeSummaryStatus>,
    pub active_inference_profile: Option<String>,
    pub hermes_available: Option<bool>,
    pub published_app_urls: Vec<String>,
    /// The Agent Principal npub the launch path verified at `/contact`, when
    /// it did. Seeds the standing-health attribution pin. Additive: an N-1
    /// runner omits it and the pin is then taken from the first report.
    #[serde(default)]
    pub agent_npub: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegisterAgentCreationRuntimeInput {
    pub request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    pub source_host_id: String,
    pub source_machine_id: String,
    pub runtime_artifact_id: Option<String>,
    pub state_schema_version: Option<String>,
    #[serde(default)]
    pub provider_runtime_handle: Option<ProviderRuntimeHandleEnvelope>,
    #[serde(default)]
    pub contact_endpoint: Option<String>,
    #[serde(default)]
    pub runtime_capabilities: Option<RuntimeCapabilitiesEnvelope>,
    pub display_name: Option<String>,
    pub hostname: Option<String>,
    pub runtime_host: Option<String>,
    pub runtime_status: Option<RuntimeSummaryStatus>,
    pub active_inference_profile: Option<String>,
    pub hermes_available: Option<bool>,
    pub published_app_urls: Vec<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailAgentCreationRequestInput {
    pub request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    pub failure_message: String,
    pub provisioned_finite_private_api_key_id: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CancelAgentCreationRequestInput {
    pub request_id: String,
    pub now: Option<String>,
}

/// End one exact host reservation after a completed, healthy targeted canary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseLaunchHostInput {
    pub reservation_code_id: String,
    pub source_host_id: String,
    pub expected_canary_runtime_id: String,
    pub operator_email: String,
    pub operator_workos_user_id: String,
}

/// One retry of a consumed canary code whose untargeted creation completed
/// on the wrong host. All old identifiers are assertions, never selections.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryTargetedLaunchCodeInput {
    pub code_id: String,
    pub expected_batch_id: String,
    pub previous_code_id: String,
    pub expected_previous_request_id: String,
    pub expected_previous_project_id: String,
    pub expected_previous_runtime_id: String,
    pub expected_previous_source_host_id: String,
    pub target_source_host_id: String,
    pub operator_email: String,
    pub operator_workos_user_id: String,
}
