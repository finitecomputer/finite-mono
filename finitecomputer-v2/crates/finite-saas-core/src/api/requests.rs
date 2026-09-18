use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimImportsRequest {
    pub selected_candidate_ids: Vec<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateAgentRequest {
    pub display_name: String,
    pub launch_code: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub hosting_tier: Option<crate::HostingTier>,
    #[serde(default)]
    pub profile_picture_url: Option<String>,
    /// Owner hosted-chat account id (64 hex), pre-minted by the dashboard via
    /// the hosted-device app state so the lease-time runtime spec can carry
    /// `FINITECHAT_OWNER_NPUBS`. Missing is accepted (legacy allow-all chat
    /// admission); malformed is rejected with 400.
    #[serde(default)]
    pub owner_chat_account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueLaunchCodeBatchRequest {
    pub name: String,
    pub code_count: u32,
    pub expires_in_hours: Option<i64>,
    #[serde(default)]
    pub hosting_tier: Option<crate::HostingTier>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaseAgentCreationRequest {
    pub runner_id: String,
    pub lease_token: String,
    pub lease_seconds: Option<i64>,
    pub runner_capacity: Option<RunnerLeaseCapacity>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordProviderOperationTransitionRequest {
    pub runner_id: String,
    pub lease_token: String,
    pub correlation_id: String,
    pub placement: crate::RuntimePlacement,
    pub transition: ProviderOperationTransition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaseRuntimeControlRequest {
    pub runner_id: String,
    pub lease_token: String,
    pub lease_seconds: Option<i64>,
    pub source_host_id: Option<String>,
    pub runner_capacity: Option<RunnerLeaseCapacity>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteAgentCreationRequest {
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
    /// Additive: the launch-verified Agent Principal, seeding the
    /// standing-health attribution pin. N-1 runners omit it.
    #[serde(default)]
    pub agent_npub: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterAgentCreationRuntimeRequest {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailAgentCreationRequest {
    pub runner_id: String,
    pub lease_token: String,
    pub failure_message: String,
    pub provisioned_finite_private_api_key_id: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteRuntimeControlRequest {
    pub runner_id: String,
    pub lease_token: String,
    pub runtime_artifact_id: Option<String>,
    pub state_schema_version: Option<String>,
    #[serde(default)]
    pub runtime_capabilities: Option<RuntimeCapabilitiesEnvelope>,
    pub runtime_host: Option<String>,
    pub published_app_urls: Option<Vec<String>>,
    #[serde(default)]
    pub retirement_snapshot: Option<crate::RuntimeRetirementSnapshotReceipt>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailRuntimeControlRequest {
    pub runner_id: String,
    pub lease_token: String,
    pub failure_message: String,
    /// Named lifecycle failure stage; absent from N-1 Runners, which Core
    /// records as `unknown`.
    #[serde(default)]
    pub failure_stage: Option<crate::RuntimeLifecycleStage>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenewRuntimeControlRequest {
    pub runner_id: String,
    pub lease_token: String,
    pub lease_seconds: Option<i64>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryRuntimeControlRequest {
    pub runner_id: String,
    pub lease_token: String,
    pub failure_message: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeControlRequestView {
    pub id: String,
    pub project_id: String,
    pub agent_runtime_id: String,
    pub source_host_id: String,
    pub source_machine_id: String,
    pub requested_by_user_id: String,
    pub kind: crate::RuntimeControlKind,
    pub target_runtime_artifact_id: Option<String>,
    pub status: crate::RuntimeControlRequestStatus,
    /// The named failure stage; present exactly when `status` is `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_stage: Option<crate::RuntimeLifecycleStage>,
    pub failure_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

impl From<crate::RuntimeControlRequest> for RuntimeControlRequestView {
    fn from(request: crate::RuntimeControlRequest) -> Self {
        Self {
            id: request.id,
            project_id: request.project_id,
            agent_runtime_id: request.agent_runtime_id,
            source_host_id: request.source_host_id,
            source_machine_id: request.source_machine_id,
            requested_by_user_id: request.requested_by_user_id,
            kind: request.kind,
            target_runtime_artifact_id: request.target_runtime_artifact_id,
            status: request.status,
            failure_stage: request.failure_stage,
            failure_message: request.failure_message,
            created_at: request.created_at,
            updated_at: request.updated_at,
            completed_at: request.completed_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelAgentCreationRequest {
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertRuntimeArtifactRequest {
    pub kind: RuntimeArtifactKind,
    pub reference: String,
    pub version_label: String,
    pub source_git_sha: Option<String>,
    pub finitec_version: Option<String>,
    pub hermes_source_ref: Option<String>,
    pub finite_platform_plugin_ref: Option<String>,
    pub state_schema_version: String,
    pub base_image: Option<String>,
    #[serde(default)]
    pub recover_known_good_chat: bool,
    #[serde(default)]
    pub canary_runtime_id: Option<String>,
    pub promoted: bool,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveFinitePrivateGrantRequest {
    pub verified_email: String,
    pub workos_user_id: Option<String>,
    pub limit_profile_id: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueFinitePrivateApiKeyRequest {
    pub raw_key: String,
    pub project_id: Option<String>,
    pub agent_runtime_id: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionFinitePrivateRuntimeKeyRequest {
    pub runner_id: String,
    pub lease_token: String,
    pub source_host_id: Option<String>,
    pub source_machine_id: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotateFinitePrivateApiKeyRequest {
    pub raw_key: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimestampRequest {
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminRuntimeUpgradeRequest {
    pub target_runtime_artifact_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminIssueFinitePrivateFriendKeyRequest {
    pub email: String,
    pub limit_profile_id: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminAssignFinitePrivateLimitProfileRequest {
    pub limit_profile_id: String,
    pub now: Option<String>,
}

/// Response for admin key issue/rotate. The raw key is returned exactly once
/// here and is never stored or logged by Core.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminIssuedFinitePrivateKeyResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant: Option<FinitePrivateGrant>,
    pub api_key: FinitePrivateApiKey,
    pub raw_api_key: String,
    pub raw_api_key_note: String,
}

pub(super) const RAW_API_KEY_NOTE: &str =
    "This raw key is shown once and cannot be recovered. Copy it now and hand it off securely.";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReserveFinitePrivateUsageRequest {
    pub request_id: String,
    pub presented_api_key: String,
    pub endpoint: String,
    pub model: String,
    pub estimated_prompt_tokens: i64,
    pub estimated_completion_tokens: i64,
    pub estimated_usage_units: i64,
    pub usage_formula_version: String,
    pub dashboard_url: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettleFinitePrivateReservationRequest {
    pub request_id: String,
    pub settlement: FinitePrivateSettlementKind,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub usage_units: Option<i64>,
    pub usage_formula_version: String,
    pub upstream_status: Option<i32>,
    pub upstream_error_class: Option<String>,
    pub now: Option<String>,
}
