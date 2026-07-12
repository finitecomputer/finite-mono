use crate::auth::{CoreAuth, VerifiedRunnerCredential, WorkosAuthError};
use crate::launch_codes::{
    IssueLaunchCodeBatchInput, LaunchCodeBatchDetails, RevokeLaunchCodeBatchInput,
};
use crate::store::{CoreStore, VisibleProject};
use crate::{
    AdminIssueFinitePrivateFriendKeyInput, AdminIssuedFinitePrivateKey,
    AdminResetFinitePrivateUsageWindowInput, AdminRevokeFinitePrivateApiKeyInput,
    AdminRotateFinitePrivateApiKeyInput, AdminRuntimeControlInput, AdminRuntimeOverview,
    AdminRuntimeUpgradeInput, AgentCreationConfiguration, AgentCreationLease, AgentCreationRequest,
    AgentRuntime, BillingOverview, BillingSubscriptionStatus, CancelAgentCreationRequestInput,
    ClaimProjectImportsInput, ClaimProjectImportsResult, CompleteAgentCreationRequestInput,
    CompleteRuntimeControlRequestInput, CoreError, CustomerBillingAccount,
    ExistingHostProjectImport, FailAgentCreationRequestInput, FailRuntimeControlRequestInput,
    FinitePrivateAdminAuditEvent, FinitePrivateAdminState, FinitePrivateApiKey, FinitePrivateGrant,
    FinitePrivateSettlementKind, FinitePrivateUsageDecision, HostingTier, ImportCandidateStatus,
    IssueFinitePrivateApiKeyInput, LeaseAgentCreationRequestInput, LeaseRuntimeControlRequestInput,
    LinkStripeCustomerInput, LinkVerifiedUserInput, Project, ProjectImportCandidate,
    ProviderRuntimeHandleEnvelope, ProvisionFinitePrivateRuntimeKeyInput,
    ProvisionFinitePrivateRuntimeKeyResult, ReconcileExistingHostImportsOptions,
    ReconcileExistingHostImportsReport, RegisterAgentCreationRuntimeInput,
    RequestAgentCreationInput, RequestAgentCreationResult, RequestRuntimeRecoverKnownGoodChatInput,
    RequestRuntimeRestartInput, ReserveFinitePrivateUsageInput, ResetFinitePrivateUsageWindowInput,
    RevokeFinitePrivateApiKeyInput, RevokeFinitePrivateGrantInput, RotateFinitePrivateApiKeyInput,
    RunnerLeaseCapacity, RuntimeArtifact, RuntimeArtifactKind, RuntimeCapabilitiesEnvelope,
    RuntimeCapabilitiesV1, RuntimeSummaryStatus, SettleFinitePrivateReservationInput,
    SettleFinitePrivateReservationResult, SourceHostRelayEndpoint, SyncStripeSubscriptionInput,
    UpsertRuntimeArtifactInput, UpsertSourceHostRelayEndpointInput, normalize_owner_email,
    normalize_runtime_contact_endpoint, normalize_source_host_id,
};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use finite_core::{
    CreateRelayChatConversationInput, CreateRelayEventInput, RelayBridgeDevice,
    RelayChatAttachmentData, RelayResult, RelayStore, SendRelayChatMessageInput,
    StoreRelayChatLogInput, StoreRelayChatSnapshotInput, StoreRelayResultInput,
    StoreRelayStatusSnapshotInput, UpdateRelayChatConversationInput,
};
use futures_util::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::convert::Infallible;
use std::env;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::sync::{Notify, broadcast};
use tokio::time::{Instant, timeout};

const SERVICE_AUTH_HEADER: &str = "authorization";
const WORKOS_USER_ID_HEADER: &str = "x-finite-workos-user-id";
const WORKOS_EMAIL_HEADER: &str = "x-finite-workos-email";
const WORKOS_EMAIL_VERIFIED_HEADER: &str = "x-finite-workos-email-verified";
const RELAY_MAX_REQUEST_BODY_BYTES: usize = 48 * 1024 * 1024;

#[derive(Clone)]
pub struct CoreApiState {
    store: CoreStore,
    auth: CoreAuth,
    standard_stripe_price_id: Option<String>,
    /// First-use deployment gate. Persisting `kind = 'upgrade'` crosses the
    /// rollback boundary for Core generations that predate that value.
    runtime_upgrades_enabled: bool,
    relay_store: RelayStore,
    result_waiters: RelayWaiters,
    chat_watchers: ChatWatchers,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
    /// Set only for internal errors we logged server-side; echoed back so a
    /// user can quote it in a support request and we can grep it out of the logs.
    correlation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileImportsRequest {
    pub records: Vec<ExistingHostProjectImport>,
    pub allowlisted_owner_emails: Vec<String>,
    pub now: Option<String>,
}

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
    pub profile_picture_url: Option<String>,
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
pub struct LinkStripeCustomerRequest {
    pub stripe_customer_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStripeSubscriptionRequest {
    pub customer_org_id: Option<String>,
    pub stripe_customer_id: String,
    pub stripe_subscription_id: String,
    pub stripe_price_id: Option<String>,
    pub subscription_status: BillingSubscriptionStatus,
    pub current_period_end: Option<String>,
    pub cancel_at_period_end: bool,
    pub stripe_event_id: Option<String>,
    pub stripe_event_created: Option<i64>,
    pub now: Option<String>,
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
    pub runtime_relay_token_hash: String,
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
    pub runtime_host: Option<String>,
    pub published_app_urls: Option<Vec<String>>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailRuntimeControlRequest {
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
pub struct UpsertSourceHostRelayRequest {
    pub url: String,
    pub admin_token: String,
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

const RAW_API_KEY_NOTE: &str =
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

#[derive(Clone, Default)]
struct RelayWaiters {
    inner: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
}

#[derive(Clone, Default)]
struct ChatWatchers {
    inner: Arc<Mutex<HashMap<String, broadcast::Sender<()>>>>,
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    after: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ChatInboxQuery {
    #[serde(rename = "projectAgentId")]
    project_agent_id: String,
    after: Option<u64>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ResultQuery {
    #[serde(rename = "waitMs")]
    wait_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ChatMessagesQuery {
    #[serde(rename = "projectAgentId")]
    project_agent_id: Option<String>,
    #[serde(rename = "bridgeAccountId")]
    bridge_account_id: String,
    #[serde(rename = "bridgeDeviceId")]
    bridge_device_id: String,
    limit: Option<usize>,
    before: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatConversationsQuery {
    #[serde(rename = "bridgeAccountId")]
    bridge_account_id: String,
    #[serde(rename = "bridgeDeviceId")]
    bridge_device_id: String,
}

#[derive(Debug, Deserialize)]
struct ChatStreamQuery {
    #[serde(rename = "bridgeAccountId")]
    bridge_account_id: String,
    #[serde(rename = "bridgeDeviceId")]
    bridge_device_id: String,
    since: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeResponse {
    pub email: String,
    pub workos_user_id: String,
    pub claimable_candidates: Vec<ClaimableProjectSummary>,
    pub projects: Vec<PublicVisibleProject>,
    pub agent_creation_requests: Vec<AgentCreationRequestSummary>,
}

/// A user-facing import choice. Source-host and machine identifiers stay on
/// the internal compatibility path and never become browser authorization or
/// navigation keys.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaimableProjectSummary {
    pub id: String,
    pub display_name: String,
    pub status: ImportCandidateStatus,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ProjectImportCandidate> for ClaimableProjectSummary {
    fn from(candidate: ProjectImportCandidate) -> Self {
        Self {
            id: candidate.id,
            display_name: candidate.host_facts.display_name,
            status: candidate.status,
            created_at: candidate.created_at,
            updated_at: candidate.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicProject {
    pub id: String,
    pub display_name: String,
    pub hosting_tier: Option<HostingTier>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<Project> for PublicProject {
    fn from(project: Project) -> Self {
        Self {
            id: project.id,
            display_name: project.display_name,
            hosting_tier: project.hosting_tier,
            created_at: project.created_at,
            updated_at: project.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRuntimeCapabilities {
    pub restart: bool,
    pub recover_known_good_chat: bool,
    pub runtime_upgrade: bool,
    pub stop: bool,
    pub runtime_retirement: bool,
}

impl From<&RuntimeCapabilitiesEnvelope> for PublicRuntimeCapabilities {
    fn from(capabilities: &RuntimeCapabilitiesEnvelope) -> Self {
        let capabilities = capabilities.v1();
        Self {
            restart: capabilities.restart,
            recover_known_good_chat: capabilities.recover_known_good_chat,
            runtime_upgrade: capabilities.runtime_upgrade,
            stop: capabilities.stop,
            runtime_retirement: capabilities.runtime_retirement,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicAgentRuntime {
    pub id: String,
    pub project_id: String,
    pub contact_endpoint: Option<String>,
    pub runtime_status: RuntimeSummaryStatus,
    pub hermes_available: Option<bool>,
    /// Populated only from Core's persisted, versioned Runtime capability
    /// record. N-1 rows remain absent and Dashboard fails closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_capabilities: Option<PublicRuntimeCapabilities>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<AgentRuntime> for PublicAgentRuntime {
    fn from(runtime: AgentRuntime) -> Self {
        let contact_endpoint = public_runtime_contact_endpoint(&runtime);
        let runtime_capabilities = runtime
            .runtime_capabilities
            .as_ref()
            .map(PublicRuntimeCapabilities::from);
        Self {
            id: runtime.id,
            project_id: runtime.project_id,
            contact_endpoint,
            runtime_status: runtime.host_facts.runtime_status,
            hermes_available: runtime.host_facts.hermes_available,
            runtime_capabilities,
            created_at: runtime.created_at,
            updated_at: runtime.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicVisibleProject {
    pub project: PublicProject,
    pub runtime: Option<PublicAgentRuntime>,
}

impl From<VisibleProject> for PublicVisibleProject {
    fn from(project: VisibleProject) -> Self {
        Self {
            project: project.project.into(),
            runtime: project.runtime.map(PublicAgentRuntime::from),
        }
    }
}

fn public_runtime_contact_endpoint(runtime: &AgentRuntime) -> Option<String> {
    // `contact_endpoint` is the public contract. Reading the first valid old
    // published URL is an N-1 compatibility bridge for rows created before
    // that field existed; new Runner generations write the explicit fact.
    normalize_runtime_contact_endpoint(runtime.contact_endpoint.as_deref())
        .ok()
        .flatten()
        .or_else(|| {
            runtime
                .host_facts
                .published_app_urls
                .iter()
                .find_map(|url| normalize_runtime_contact_endpoint(Some(url)).ok().flatten())
        })
}

fn public_visible_projects(projects: Vec<VisibleProject>) -> Vec<PublicVisibleProject> {
    projects
        .into_iter()
        .filter(|project| project.project.import_candidate_id.is_none())
        .map(PublicVisibleProject::from)
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeRouteResolution {
    pub project_id: String,
    pub runtime_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCreationRequestSummary {
    pub id: String,
    pub project_id: String,
    pub display_name: String,
    pub profile_picture_url: Option<String>,
    pub status: crate::AgentCreationRequestStatus,
    pub agent_runtime_id: Option<String>,
    pub failure_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<AgentCreationRequest> for AgentCreationRequestSummary {
    fn from(request: AgentCreationRequest) -> Self {
        Self {
            id: request.id,
            project_id: request.project_id,
            display_name: request.display_name,
            profile_picture_url: request.profile_picture_url,
            status: request.status,
            agent_runtime_id: request.agent_runtime_id,
            failure_message: request.failure_message,
            created_at: request.created_at,
            updated_at: request.updated_at,
        }
    }
}

pub fn router(store: CoreStore, auth: CoreAuth) -> Router {
    router_with_relay_state_dir(store, auth, default_relay_state_dir())
}

pub fn router_with_relay_state_dir(
    store: CoreStore,
    auth: CoreAuth,
    relay_state_dir: impl Into<PathBuf>,
) -> Router {
    let runtime_upgrades_enabled = env::var("FC_CORE_ENABLE_RUNTIME_UPGRADES")
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE"));
    router_with_runtime_upgrades(store, auth, relay_state_dir, runtime_upgrades_enabled)
}

pub fn router_with_runtime_upgrades(
    store: CoreStore,
    auth: CoreAuth,
    relay_state_dir: impl Into<PathBuf>,
    runtime_upgrades_enabled: bool,
) -> Router {
    let standard_stripe_price_id = optional_env_value("FC_CORE_STANDARD_STRIPE_PRICE_ID")
        .or_else(|| optional_env_value("STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID"));
    let state = CoreApiState {
        store,
        auth,
        standard_stripe_price_id,
        runtime_upgrades_enabled,
        relay_store: RelayStore::new(relay_state_dir.into()),
        result_waiters: RelayWaiters::default(),
        chat_watchers: ChatWatchers::default(),
    };

    Router::new()
        .route("/healthz", get(healthz))
        .route(
            "/api/core/v1/import-candidates/reconcile",
            post(reconcile_import_candidates),
        )
        .route(
            "/api/core/v1/source-host-relays/{source_host_id}",
            get(source_host_relay_endpoint).put(upsert_source_host_relay_endpoint),
        )
        .route(
            "/api/core/v1/runtime-artifacts/{artifact_id}",
            get(runtime_artifact).put(upsert_runtime_artifact),
        )
        .route(
            "/api/core/v1/finite-private/grants",
            post(approve_finite_private_grant),
        )
        .route(
            "/api/core/v1/finite-private/grants/{grant_id}/api-keys",
            post(issue_finite_private_api_key),
        )
        .route(
            "/api/core/v1/finite-private/grants/{grant_id}/revoke",
            post(revoke_finite_private_grant),
        )
        .route(
            "/api/core/v1/finite-private/grants/{grant_id}/reset",
            post(reset_finite_private_usage_window),
        )
        .route(
            "/api/core/v1/finite-private/api-keys/{key_id}/revoke",
            post(revoke_finite_private_api_key),
        )
        .route(
            "/api/core/v1/finite-private/api-keys/{key_id}/rotate",
            post(rotate_finite_private_api_key),
        )
        .route(
            "/api/core/v1/finite-private/admin-audit-events",
            get(finite_private_admin_audit_events),
        )
        .route(
            "/api/core/v1/finite-private/admin-state",
            get(finite_private_admin_state),
        )
        .route(
            "/internal/finite-private/v1/health",
            get(finite_private_usage_health),
        )
        .route(
            "/internal/finite-private/v1/reservations",
            post(reserve_finite_private_usage),
        )
        .route(
            "/internal/finite-private/v1/reservations/{reservation_id}/settle",
            post(settle_finite_private_reservation),
        )
        .route("/api/core/v1/admin/runtimes", get(admin_runtimes))
        .route(
            "/api/core/v1/admin/launch-code-batches",
            get(admin_list_launch_code_batches).post(admin_issue_launch_code_batch),
        )
        .route(
            "/api/core/v1/admin/launch-code-batches/{batch_id}/revoke",
            post(admin_revoke_launch_code_batch),
        )
        .route(
            "/api/core/v1/admin/projects/{project_id}/runtime/restart",
            post(admin_request_runtime_restart),
        )
        .route(
            "/api/core/v1/admin/projects/{project_id}/runtime/recover-known-good-chat",
            post(admin_request_runtime_recover_known_good_chat),
        )
        .route(
            "/api/core/v1/admin/projects/{project_id}/runtime/upgrade",
            post(admin_request_runtime_upgrade),
        )
        .route(
            "/api/core/v1/admin/finite-private/friend-keys",
            post(admin_issue_finite_private_friend_key),
        )
        .route(
            "/api/core/v1/admin/finite-private/keys/{key_id}/rotate",
            post(admin_rotate_finite_private_api_key),
        )
        .route(
            "/api/core/v1/admin/finite-private/keys/{key_id}/revoke",
            post(admin_revoke_finite_private_api_key),
        )
        .route(
            "/api/core/v1/admin/finite-private/grants/{grant_id}/window-reset",
            post(admin_reset_finite_private_usage_window),
        )
        .route("/api/core/v1/me", get(me))
        .route(
            "/api/core/v1/me/runtime-routes/{identifier}",
            get(resolve_runtime_route),
        )
        .route("/api/core/v1/me/billing", get(billing_overview))
        .route(
            "/api/core/v1/me/billing/stripe-customer",
            post(link_stripe_customer),
        )
        .route(
            "/api/core/v1/billing/stripe/subscription",
            post(sync_stripe_subscription),
        )
        .route("/api/core/v1/me/import-candidates", get(import_candidates))
        .route(
            "/api/core/v1/me/import-candidates/claim",
            post(claim_import_candidates),
        )
        .route(
            "/api/core/v1/me/agent-creation-requests",
            post(create_agent_request),
        )
        .route(
            "/api/core/v1/me/projects/{project_id}/archive",
            post(archive_imported_project),
        )
        .route(
            "/api/core/v1/me/projects/{project_id}/runtime/restart",
            post(request_runtime_restart),
        )
        .route(
            "/api/core/v1/me/projects/{project_id}/runtime/recover-known-good-chat",
            post(request_runtime_recover_known_good_chat),
        )
        .route(
            "/api/core/v1/me/projects/{project_id}/runtime/stop",
            post(request_runtime_stop),
        )
        .route(
            "/api/core/v1/me/projects/{project_id}/runtime/destroy",
            post(request_runtime_destroy),
        )
        .route(
            "/api/core/v1/agent-creation-requests/lease",
            post(lease_agent_creation_request),
        )
        .route(
            "/api/core/v1/runtime-control-requests/lease",
            post(lease_runtime_control_request),
        )
        .route(
            "/api/core/v1/runtime-control-requests/{request_id}/complete",
            post(complete_runtime_control_request),
        )
        .route(
            "/api/core/v1/runtime-control-requests/{request_id}/fail",
            post(fail_runtime_control_request),
        )
        .route(
            "/api/core/v1/agent-creation-requests/{request_id}/complete",
            post(complete_agent_creation_request),
        )
        .route(
            "/api/core/v1/agent-creation-requests/{request_id}/runtime",
            post(register_agent_creation_runtime),
        )
        .route(
            "/api/core/v1/agent-creation-requests/{request_id}/finite-private-key",
            post(provision_finite_private_runtime_key),
        )
        .route(
            "/api/core/v1/agent-creation-requests/{request_id}/fail",
            post(fail_agent_creation_request),
        )
        .route(
            "/api/core/v1/agent-creation-requests/{request_id}/cancel",
            post(cancel_agent_creation_request),
        )
        .route("/api/core/v1/me/projects", get(projects))
        .route("/api/finite/v1/heartbeat", post(runtime_heartbeat))
        .route("/api/finite/v1/events", get(runtime_events))
        .route(
            "/api/finite/v1/events/{event_id}/ack",
            post(runtime_ack_event),
        )
        .route("/api/finite/v1/results", post(runtime_store_result))
        .route("/api/finite/v1/chat/inbox", get(runtime_chat_inbox))
        .route("/api/finite/v1/chat/snapshot", post(runtime_chat_snapshot))
        .route(
            "/api/finite/v1/chat/log/messages",
            post(runtime_chat_log_messages),
        )
        .route("/api/finite/v1/chat/blobs/{sha256}", put(runtime_chat_blob))
        .route(
            "/api/finite/v1/chat/attachments/{attachment_id}",
            get(runtime_chat_attachment),
        )
        .route(
            "/api/finite/v1/status/snapshots",
            post(runtime_status_snapshot),
        )
        .route(
            "/api/finite/v1/machines/{machine_id}/events",
            post(admin_create_event),
        )
        .route(
            "/api/finite/v1/machines/{machine_id}/heartbeat",
            get(runtime_heartbeat_for_machine),
        )
        .route(
            "/api/finite/v1/machines/{machine_id}/results/{event_id}",
            get(admin_wait_result),
        )
        .route(
            "/api/finite/v1/machines/{machine_id}/chat/snapshot",
            get(admin_chat_snapshot),
        )
        .route(
            "/api/finite/v1/machines/{machine_id}/chat/conversations",
            get(admin_chat_conversations).post(admin_create_chat_conversation),
        )
        .route(
            "/api/finite/v1/machines/{machine_id}/chat/conversations/{conversation_id}",
            put(admin_update_chat_conversation),
        )
        .route(
            "/api/finite/v1/machines/{machine_id}/chat/conversations/{conversation_id}/messages",
            get(admin_chat_messages).post(admin_send_chat_message),
        )
        .route(
            "/api/finite/v1/machines/{machine_id}/chat/attachments/{attachment_id}",
            get(admin_chat_attachment),
        )
        .route(
            "/api/finite/v1/machines/{machine_id}/status/snapshots/{state_key}",
            get(admin_status_snapshot),
        )
        .route(
            "/api/finite/v1/machines/{machine_id}/chat/stream",
            get(admin_chat_stream),
        )
        .layer(DefaultBodyLimit::max(RELAY_MAX_REQUEST_BODY_BYTES))
        .with_state(state)
}

fn default_relay_state_dir() -> PathBuf {
    env::var_os("FC_CORE_RELAY_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/finite-saas-core/relay"))
}

fn optional_env_value(name: &str) -> Option<String> {
    env::var(name).ok().and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

async fn healthz() -> Json<serde_json::Value> {
    Json(json!({ "ok": true }))
}

async fn reconcile_import_candidates(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<ReconcileImportsRequest>,
) -> Result<Json<ReconcileExistingHostImportsReport>, ApiError> {
    require_service_auth(&state, &headers)?;
    let report = state
        .store
        .reconcile_existing_host_imports(
            input.records,
            ReconcileExistingHostImportsOptions {
                allowlisted_owner_emails: input.allowlisted_owner_emails,
                now: input.now,
            },
        )
        .await?;
    Ok(Json(report))
}

async fn source_host_relay_endpoint(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(source_host_id): Path<String>,
) -> Result<Json<SourceHostRelayEndpoint>, ApiError> {
    require_service_auth(&state, &headers)?;
    let Some(endpoint) = state
        .store
        .source_host_relay_endpoint(&source_host_id)
        .await?
    else {
        return Err(ApiError::not_found(
            "source host relay endpoint is not configured",
        ));
    };
    Ok(Json(endpoint))
}

async fn upsert_source_host_relay_endpoint(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(source_host_id): Path<String>,
    Json(input): Json<UpsertSourceHostRelayRequest>,
) -> Result<Json<SourceHostRelayEndpoint>, ApiError> {
    require_service_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .upsert_source_host_relay_endpoint(UpsertSourceHostRelayEndpointInput {
                source_host_id,
                url: input.url,
                admin_token: input.admin_token,
                now: input.now,
            })
            .await?,
    ))
}

async fn runtime_artifact(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(artifact_id): Path<String>,
) -> Result<Json<RuntimeArtifact>, ApiError> {
    let _credential = require_runner_auth(&state, &headers)?;
    let Some(artifact) = state.store.runtime_artifact(&artifact_id).await? else {
        return Err(ApiError::not_found("runtime artifact is not configured"));
    };
    Ok(Json(artifact))
}

async fn upsert_runtime_artifact(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(artifact_id): Path<String>,
    Json(input): Json<UpsertRuntimeArtifactRequest>,
) -> Result<Json<RuntimeArtifact>, ApiError> {
    require_service_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: artifact_id,
                kind: input.kind,
                reference: input.reference,
                version_label: input.version_label,
                source_git_sha: input.source_git_sha,
                finitec_version: input.finitec_version,
                hermes_source_ref: input.hermes_source_ref,
                finite_platform_plugin_ref: input.finite_platform_plugin_ref,
                state_schema_version: input.state_schema_version,
                base_image: input.base_image,
                promoted: input.promoted,
                now: input.now,
            })
            .await?,
    ))
}

async fn approve_finite_private_grant(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<ApproveFinitePrivateGrantRequest>,
) -> Result<Json<FinitePrivateGrant>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .approve_finite_private_grant(crate::ApproveFinitePrivateGrantInput {
                verified_email: input.verified_email,
                workos_user_id: input.workos_user_id,
                limit_profile_id: input.limit_profile_id,
                now: input.now,
            })
            .await?,
    ))
}

async fn issue_finite_private_api_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(grant_id): Path<String>,
    Json(input): Json<IssueFinitePrivateApiKeyRequest>,
) -> Result<Json<FinitePrivateApiKey>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id,
                raw_key: input.raw_key,
                project_id: input.project_id,
                agent_runtime_id: input.agent_runtime_id,
                now: input.now,
            })
            .await?,
    ))
}

async fn revoke_finite_private_grant(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(grant_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<FinitePrivateGrant>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .revoke_finite_private_grant(RevokeFinitePrivateGrantInput {
                grant_id,
                now: input.now,
            })
            .await?,
    ))
}

async fn reset_finite_private_usage_window(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(grant_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<FinitePrivateGrant>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .reset_finite_private_usage_window(ResetFinitePrivateUsageWindowInput {
                grant_id,
                now: input.now,
            })
            .await?,
    ))
}

async fn revoke_finite_private_api_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<FinitePrivateApiKey>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .revoke_finite_private_api_key(RevokeFinitePrivateApiKeyInput {
                key_id,
                now: input.now,
            })
            .await?,
    ))
}

async fn rotate_finite_private_api_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Json(input): Json<RotateFinitePrivateApiKeyRequest>,
) -> Result<Json<FinitePrivateApiKey>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .rotate_finite_private_api_key(RotateFinitePrivateApiKeyInput {
                key_id,
                raw_key: input.raw_key,
                now: input.now,
            })
            .await?,
    ))
}

async fn finite_private_admin_audit_events(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<FinitePrivateAdminAuditEvent>>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(state.store.finite_private_admin_audit_events().await?))
}

async fn finite_private_admin_state(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<FinitePrivateAdminState>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(state.store.finite_private_admin_state().await?))
}

async fn admin_runtimes(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AdminRuntimeOverview>>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(state.store.admin_runtime_overviews().await?))
}

async fn admin_list_launch_code_batches(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<LaunchCodeBatchDetails>>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(state.store.list_launch_code_batches().await?))
}

async fn admin_issue_launch_code_batch(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<IssueLaunchCodeBatchRequest>,
) -> Result<Response, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    let issued = state
        .store
        .issue_launch_code_batch(IssueLaunchCodeBatchInput {
            name: input.name,
            code_count: input.code_count,
            expires_in_hours: input.expires_in_hours,
            hosting_tier: input.hosting_tier,
            created_by_workos_user_id: identity.workos_user_id,
            now: None,
        })
        .await?;
    let mut response = Json(issued).into_response();
    response.headers_mut().insert(
        "cache-control",
        HeaderValue::from_static("no-store, private"),
    );
    Ok(response)
}

async fn admin_revoke_launch_code_batch(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(batch_id): Path<String>,
) -> Result<Json<LaunchCodeBatchDetails>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .revoke_launch_code_batch(RevokeLaunchCodeBatchInput {
                batch_id,
                revoked_by_workos_user_id: identity.workos_user_id,
                now: None,
            })
            .await?,
    ))
}

async fn admin_request_runtime_restart(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<RuntimeControlRequestView>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    let request = state
        .store
        .admin_request_runtime_restart(AdminRuntimeControlInput {
            admin_verified_email: identity.email,
            admin_workos_user_id: identity.workos_user_id,
            project_id,
            now: input.now,
        })
        .await?;
    Ok(Json(RuntimeControlRequestView::from(request)))
}

async fn admin_request_runtime_recover_known_good_chat(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<RuntimeControlRequestView>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    let request = state
        .store
        .admin_request_runtime_recover_known_good_chat(AdminRuntimeControlInput {
            admin_verified_email: identity.email,
            admin_workos_user_id: identity.workos_user_id,
            project_id,
            now: input.now,
        })
        .await?;
    Ok(Json(RuntimeControlRequestView::from(request)))
}

async fn admin_request_runtime_upgrade(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<AdminRuntimeUpgradeRequest>,
) -> Result<Json<RuntimeControlRequestView>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    if !state.runtime_upgrades_enabled {
        return Err(CoreError::RuntimeUpgradeNotEnabled.into());
    }
    let request = state
        .store
        .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
            admin_verified_email: identity.email,
            admin_workos_user_id: identity.workos_user_id,
            project_id,
            target_runtime_artifact_id: input.target_runtime_artifact_id,
            now: input.now,
        })
        .await?;
    Ok(Json(RuntimeControlRequestView::from(request)))
}

async fn admin_issue_finite_private_friend_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<AdminIssueFinitePrivateFriendKeyRequest>,
) -> Result<Json<AdminIssuedFinitePrivateKeyResponse>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    let raw_api_key = crate::generate_finite_private_api_key()?;
    let AdminIssuedFinitePrivateKey { grant, api_key } = state
        .store
        .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
            admin_verified_email: identity.email,
            friend_email: input.email,
            limit_profile_id: input.limit_profile_id,
            raw_key: raw_api_key.clone(),
            now: input.now,
        })
        .await?;
    Ok(Json(AdminIssuedFinitePrivateKeyResponse {
        grant: Some(grant),
        api_key,
        raw_api_key,
        raw_api_key_note: RAW_API_KEY_NOTE.to_string(),
    }))
}

async fn admin_rotate_finite_private_api_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<AdminIssuedFinitePrivateKeyResponse>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    let raw_api_key = crate::generate_finite_private_api_key()?;
    let api_key = state
        .store
        .admin_rotate_finite_private_api_key(AdminRotateFinitePrivateApiKeyInput {
            admin_verified_email: identity.email,
            key_id,
            raw_key: raw_api_key.clone(),
            now: input.now,
        })
        .await?;
    Ok(Json(AdminIssuedFinitePrivateKeyResponse {
        grant: None,
        api_key,
        raw_api_key,
        raw_api_key_note: RAW_API_KEY_NOTE.to_string(),
    }))
}

async fn admin_revoke_finite_private_api_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<FinitePrivateApiKey>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .admin_revoke_finite_private_api_key(AdminRevokeFinitePrivateApiKeyInput {
                admin_verified_email: identity.email,
                key_id,
                now: input.now,
            })
            .await?,
    ))
}

async fn admin_reset_finite_private_usage_window(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(grant_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<FinitePrivateGrant>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .admin_reset_finite_private_usage_window(AdminResetFinitePrivateUsageWindowInput {
                admin_verified_email: identity.email,
                grant_id,
                now: input.now,
            })
            .await?,
    ))
}

async fn finite_private_usage_health(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_finite_private_usage_auth(&state, &headers)?;
    Ok(Json(json!({ "ok": true })))
}

async fn reserve_finite_private_usage(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<ReserveFinitePrivateUsageRequest>,
) -> Result<Json<FinitePrivateUsageDecision>, ApiError> {
    require_finite_private_usage_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: input.request_id,
                presented_api_key: input.presented_api_key,
                endpoint: input.endpoint,
                model: input.model,
                estimated_prompt_tokens: input.estimated_prompt_tokens,
                estimated_completion_tokens: input.estimated_completion_tokens,
                estimated_usage_units: input.estimated_usage_units,
                usage_formula_version: input.usage_formula_version,
                dashboard_url: input
                    .dashboard_url
                    .unwrap_or_else(|| "https://finite.computer/dashboard".to_string()),
                now: input.now,
            })
            .await?,
    ))
}

async fn settle_finite_private_reservation(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(reservation_id): Path<String>,
    Json(input): Json<SettleFinitePrivateReservationRequest>,
) -> Result<Json<SettleFinitePrivateReservationResult>, ApiError> {
    require_finite_private_usage_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .settle_finite_private_reservation(SettleFinitePrivateReservationInput {
                reservation_id,
                request_id: input.request_id,
                settlement: input.settlement,
                prompt_tokens: input.prompt_tokens,
                completion_tokens: input.completion_tokens,
                usage_units: input.usage_units,
                usage_formula_version: input.usage_formula_version,
                upstream_status: input.upstream_status,
                upstream_error_class: input.upstream_error_class,
                now: input.now,
            })
            .await?,
    ))
}

async fn me(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<MeResponse>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    state
        .store
        .link_verified_user(LinkVerifiedUserInput {
            verified_email: identity.email.clone(),
            workos_user_id: identity.workos_user_id.clone(),
            now: None,
        })
        .await?;
    let claimable_candidates = state
        .store
        .claimable_candidates_for_email(Some(&identity.email))
        .await?;
    let projects = state
        .store
        .visible_projects_for_workos_user(&identity.workos_user_id)
        .await?;
    let agent_creation_requests = state
        .store
        .agent_creation_requests_for_workos_user(&identity.workos_user_id)
        .await?;
    Ok(Json(MeResponse {
        email: identity.email,
        workos_user_id: identity.workos_user_id,
        claimable_candidates: claimable_candidates
            .into_iter()
            .map(ClaimableProjectSummary::from)
            .collect(),
        projects: public_visible_projects(projects),
        agent_creation_requests: agent_creation_requests
            .into_iter()
            .map(AgentCreationRequestSummary::from)
            .collect(),
    }))
}

async fn resolve_runtime_route(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(identifier): Path<String>,
) -> Result<Json<RuntimeRouteResolution>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    state
        .store
        .link_verified_user(LinkVerifiedUserInput {
            verified_email: identity.email,
            workos_user_id: identity.workos_user_id.clone(),
            now: None,
        })
        .await?;
    let identifier = identifier.trim();
    let resolution = state
        .store
        .visible_projects_for_workos_user(&identity.workos_user_id)
        .await?
        .into_iter()
        .find_map(|project| {
            if project.project.import_candidate_id.is_some() {
                return None;
            }
            let runtime = project.runtime?;
            (project.project.id == identifier
                || runtime.id == identifier
                || runtime.source_machine_id == identifier)
                .then_some(RuntimeRouteResolution {
                    project_id: project.project.id,
                    runtime_id: runtime.id,
                })
        })
        .ok_or_else(|| ApiError::not_found("agent runtime was not found"))?;
    Ok(Json(resolution))
}

async fn billing_overview(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<BillingOverview>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .billing_overview(LinkVerifiedUserInput {
                verified_email: identity.email,
                workos_user_id: identity.workos_user_id,
                now: None,
            })
            .await?,
    ))
}

async fn link_stripe_customer(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<LinkStripeCustomerRequest>,
) -> Result<Json<CustomerBillingAccount>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .link_stripe_customer(LinkStripeCustomerInput {
                verified_email: identity.email,
                workos_user_id: identity.workos_user_id,
                stripe_customer_id: input.stripe_customer_id,
                now: input.now,
            })
            .await?,
    ))
}

async fn sync_stripe_subscription(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<SyncStripeSubscriptionRequest>,
) -> Result<Json<CustomerBillingAccount>, ApiError> {
    require_service_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: input.customer_org_id,
                stripe_customer_id: input.stripe_customer_id,
                stripe_subscription_id: input.stripe_subscription_id,
                stripe_price_id: input.stripe_price_id,
                expected_stripe_price_id: state.standard_stripe_price_id.clone(),
                subscription_status: input.subscription_status,
                current_period_end: input.current_period_end,
                cancel_at_period_end: input.cancel_at_period_end,
                stripe_event_id: input.stripe_event_id,
                stripe_event_created: input.stripe_event_created,
                now: input.now,
            })
            .await?,
    ))
}

async fn import_candidates(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ProjectImportCandidate>>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .claimable_candidates_for_email(Some(&identity.email))
            .await?,
    ))
}

async fn claim_import_candidates(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<ClaimImportsRequest>,
) -> Result<Json<ClaimProjectImportsResult>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    let result = state
        .store
        .claim_project_imports(ClaimProjectImportsInput {
            verified_email: identity.email,
            workos_user_id: identity.workos_user_id,
            selected_candidate_ids: input.selected_candidate_ids,
            now: input.now,
        })
        .await?;
    Ok(Json(result))
}

async fn create_agent_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<CreateAgentRequest>,
) -> Result<Json<RequestAgentCreationResult>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .request_agent_creation_configured(
                RequestAgentCreationInput {
                    verified_email: identity.email,
                    workos_user_id: identity.workos_user_id,
                    display_name: input.display_name,
                    launch_code: input.launch_code,
                    idempotency_key: input.idempotency_key,
                    now: None,
                },
                AgentCreationConfiguration {
                    placement: None,
                    profile_picture_url: input.profile_picture_url,
                },
            )
            .await?,
    ))
}

async fn request_runtime_restart(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<RuntimeControlRequestView>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    let request = state
        .store
        .request_runtime_restart(RequestRuntimeRestartInput {
            verified_email: identity.email,
            workos_user_id: identity.workos_user_id,
            project_id,
            now: input.now,
        })
        .await?;
    Ok(Json(RuntimeControlRequestView::from(request)))
}

async fn request_runtime_recover_known_good_chat(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<RuntimeControlRequestView>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    let request = state
        .store
        .request_runtime_recover_known_good_chat(RequestRuntimeRecoverKnownGoodChatInput {
            verified_email: identity.email,
            workos_user_id: identity.workos_user_id,
            project_id,
            now: input.now,
        })
        .await?;
    Ok(Json(RuntimeControlRequestView::from(request)))
}

async fn request_runtime_stop(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<RuntimeControlRequestView>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    let request = state
        .store
        .request_runtime_stop(crate::RequestRuntimeStopInput {
            verified_email: identity.email,
            workos_user_id: identity.workos_user_id,
            project_id,
            now: input.now,
        })
        .await?;
    Ok(Json(RuntimeControlRequestView::from(request)))
}

async fn request_runtime_destroy(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<RuntimeControlRequestView>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    let request = state
        .store
        .request_runtime_destroy(crate::RequestRuntimeDestroyInput {
            verified_email: identity.email,
            workos_user_id: identity.workos_user_id,
            project_id,
            now: input.now,
        })
        .await?;
    Ok(Json(RuntimeControlRequestView::from(request)))
}

async fn archive_imported_project(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<StatusCode, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    state
        .store
        .archive_imported_project(crate::ArchiveImportedProjectInput {
            verified_email: identity.email,
            workos_user_id: identity.workos_user_id,
            project_id,
            now: input.now,
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn lease_agent_creation_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<LeaseAgentCreationRequest>,
) -> Result<Json<Option<AgentCreationLease>>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    let runner_capacity = authorize_runner_capacity(&credential, input.runner_capacity)?;
    Ok(Json(
        state
            .store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: input.runner_id,
                source_host_id: Some(credential.source_host_id),
                lease_token: input.lease_token,
                lease_seconds: input.lease_seconds,
                runner_capacity: Some(runner_capacity),
                now: input.now,
            })
            .await?,
    ))
}

async fn lease_runtime_control_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<LeaseRuntimeControlRequest>,
) -> Result<Json<Option<crate::RuntimeControlLease>>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    let runner_capacity = authorize_runner_capacity(&credential, input.runner_capacity)?;
    let source_host_id =
        authorize_runner_source_host(&credential, input.source_host_id.as_deref())?;
    Ok(Json(
        state
            .store
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                lease_seconds: input.lease_seconds,
                source_host_id: Some(source_host_id),
                runner_capacity: Some(runner_capacity),
                now: input.now,
            })
            .await?,
    ))
}

async fn complete_runtime_control_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<CompleteRuntimeControlRequest>,
) -> Result<Json<crate::RuntimeControlRequest>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    Ok(Json(
        state
            .store
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                runtime_artifact_id: input.runtime_artifact_id,
                state_schema_version: input.state_schema_version,
                runtime_host: input.runtime_host,
                published_app_urls: input.published_app_urls,
                now: input.now,
            })
            .await?,
    ))
}

async fn fail_runtime_control_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<FailRuntimeControlRequest>,
) -> Result<Json<crate::RuntimeControlRequest>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    Ok(Json(
        state
            .store
            .fail_runtime_control_request(FailRuntimeControlRequestInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                failure_message: input.failure_message,
                now: input.now,
            })
            .await?,
    ))
}

async fn complete_agent_creation_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<CompleteAgentCreationRequest>,
) -> Result<Json<AgentCreationLease>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    let source_host_id = authorize_runner_source_host(&credential, Some(&input.source_host_id))?;
    authorize_provider_runtime_handle(&credential, input.provider_runtime_handle.as_ref())?;
    let runtime_capabilities =
        authorize_runner_runtime_capabilities(&credential, input.runtime_capabilities)?;
    Ok(Json(
        state
            .store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                source_host_id,
                source_machine_id: input.source_machine_id,
                runtime_artifact_id: input.runtime_artifact_id,
                state_schema_version: input.state_schema_version,
                provider_runtime_handle: input.provider_runtime_handle,
                contact_endpoint: input.contact_endpoint,
                runtime_capabilities: Some(runtime_capabilities),
                display_name: input.display_name,
                hostname: input.hostname,
                runtime_host: input.runtime_host,
                runtime_status: input.runtime_status,
                active_inference_profile: input.active_inference_profile,
                hermes_available: input.hermes_available,
                published_app_urls: input.published_app_urls,
                now: input.now,
            })
            .await?,
    ))
}

async fn register_agent_creation_runtime(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<RegisterAgentCreationRuntimeRequest>,
) -> Result<Json<AgentCreationLease>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    let source_host_id = authorize_runner_source_host(&credential, Some(&input.source_host_id))?;
    authorize_provider_runtime_handle(&credential, input.provider_runtime_handle.as_ref())?;
    let runtime_capabilities =
        authorize_runner_runtime_capabilities(&credential, input.runtime_capabilities)?;
    Ok(Json(
        state
            .store
            .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                source_host_id,
                source_machine_id: input.source_machine_id,
                runtime_artifact_id: input.runtime_artifact_id,
                state_schema_version: input.state_schema_version,
                provider_runtime_handle: input.provider_runtime_handle,
                contact_endpoint: input.contact_endpoint,
                runtime_capabilities: Some(runtime_capabilities),
                runtime_relay_token_hash: input.runtime_relay_token_hash,
                display_name: input.display_name,
                hostname: input.hostname,
                runtime_host: input.runtime_host,
                runtime_status: input.runtime_status,
                active_inference_profile: input.active_inference_profile,
                hermes_available: input.hermes_available,
                published_app_urls: input.published_app_urls,
                now: input.now,
            })
            .await?,
    ))
}

async fn provision_finite_private_runtime_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<ProvisionFinitePrivateRuntimeKeyRequest>,
) -> Result<Json<ProvisionFinitePrivateRuntimeKeyResult>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    let source_host_id =
        authorize_runner_source_host(&credential, input.source_host_id.as_deref())?;
    Ok(Json(
        state
            .store
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                source_host_id: Some(source_host_id),
                source_machine_id: input.source_machine_id,
                now: input.now,
            })
            .await?,
    ))
}

async fn fail_agent_creation_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<FailAgentCreationRequest>,
) -> Result<Json<AgentCreationRequest>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    Ok(Json(
        state
            .store
            .fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                failure_message: input.failure_message,
                provisioned_finite_private_api_key_id: input.provisioned_finite_private_api_key_id,
                now: input.now,
            })
            .await?,
    ))
}

async fn cancel_agent_creation_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<CancelAgentCreationRequest>,
) -> Result<Json<AgentCreationRequest>, ApiError> {
    require_service_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .cancel_agent_creation_request(CancelAgentCreationRequestInput {
                request_id,
                now: input.now,
            })
            .await?,
    ))
}

async fn runtime_heartbeat(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<crate::RelayHeartbeat>, ApiError> {
    let token =
        bearer_token(&headers).ok_or_else(|| ApiError::unauthorized("missing runtime token"))?;
    let heartbeat = state.store.record_runtime_heartbeat(&token).await?;
    let _ = state.relay_store.heartbeat(&heartbeat.machine_id)?;
    Ok(Json(heartbeat))
}

async fn runtime_events(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Result<Json<Value>, ApiError> {
    let machine_id = authenticate_runtime_machine(&state, &headers).await?;
    let events =
        state
            .relay_store
            .claim_events(&machine_id, query.after.as_deref(), query.limit)?;
    Ok(Json(serde_json::to_value(events)?))
}

async fn runtime_ack_event(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(event_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let machine_id = authenticate_runtime_machine(&state, &headers).await?;
    let ack = state.relay_store.ack_event(&machine_id, &event_id)?;
    Ok(Json(serde_json::to_value(ack)?))
}

async fn runtime_store_result(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<StoreRelayResultInput>,
) -> Result<Json<Value>, ApiError> {
    let machine_id = authenticate_runtime_machine(&state, &headers).await?;
    let event_id = input.event_id.clone();
    let result = state.relay_store.store_result(&machine_id, &input)?;
    state.result_waiters.notify_result(&machine_id, &event_id);
    Ok(Json(serde_json::to_value(result)?))
}

async fn runtime_chat_inbox(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Query(query): Query<ChatInboxQuery>,
) -> Result<Json<Value>, ApiError> {
    let machine_id = authenticate_runtime_machine(&state, &headers).await?;
    let page = state.relay_store.chat_inbox(
        &machine_id,
        &query.project_agent_id,
        query.after,
        query.limit,
    )?;
    Ok(Json(serde_json::to_value(page)?))
}

async fn runtime_chat_snapshot(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<StoreRelayChatSnapshotInput>,
) -> Result<Json<Value>, ApiError> {
    let machine_id = authenticate_runtime_machine(&state, &headers).await?;
    let snapshot = state.relay_store.store_chat_snapshot(&machine_id, &input)?;
    state.chat_watchers.notify(&machine_id);
    Ok(Json(serde_json::to_value(snapshot)?))
}

async fn runtime_chat_log_messages(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<StoreRelayChatLogInput>,
) -> Result<Json<Value>, ApiError> {
    let machine_id = authenticate_runtime_machine(&state, &headers).await?;
    let ack = state.relay_store.store_chat_log(&machine_id, &input)?;
    if ack.stored > 0 {
        state.chat_watchers.notify(&machine_id);
    }
    Ok(Json(serde_json::to_value(ack)?))
}

async fn runtime_chat_blob(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(sha256): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let machine_id = authenticate_runtime_machine(&state, &headers).await?;
    let ack = state
        .relay_store
        .store_chat_blob(&machine_id, &sha256, &body)?;
    Ok(Json(serde_json::to_value(ack)?))
}

async fn runtime_chat_attachment(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(attachment_id): Path<String>,
) -> Result<Response, ApiError> {
    let machine_id = authenticate_runtime_machine(&state, &headers).await?;
    let bridge = runtime_bridge_device(&machine_id);
    let Some(attachment) =
        state
            .relay_store
            .read_chat_attachment(&machine_id, &attachment_id, &bridge)?
    else {
        return Err(ApiError::not_found("attachment not found"));
    };
    Ok(chat_attachment_response(attachment))
}

async fn runtime_status_snapshot(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<StoreRelayStatusSnapshotInput>,
) -> Result<Json<Value>, ApiError> {
    let machine_id = authenticate_runtime_machine(&state, &headers).await?;
    let snapshot = state
        .relay_store
        .store_status_snapshot(&machine_id, &input)?;
    Ok(Json(serde_json::to_value(snapshot)?))
}

async fn runtime_heartbeat_for_machine(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(machine_id): Path<String>,
) -> Result<Json<crate::RelayHeartbeat>, ApiError> {
    let _credential = require_runner_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .runtime_heartbeat_for_machine(&machine_id)
            .await?,
    ))
}

async fn admin_create_event(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(machine_id): Path<String>,
    Json(input): Json<CreateRelayEventInput>,
) -> Result<Json<Value>, ApiError> {
    require_service_auth(&state, &headers)?;
    if input.scope.is_none() {
        let kind = input.kind.trim();
        let kind = if kind.is_empty() {
            "relay command"
        } else {
            kind
        };
        return Err(ApiError::bad_request(format!(
            "{kind} requires explicit command scope"
        )));
    }
    let event = state.relay_store.create_event(&machine_id, &input)?;
    Ok(Json(serde_json::to_value(event)?))
}

async fn admin_wait_result(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path((machine_id, event_id)): Path<(String, String)>,
    Query(query): Query<ResultQuery>,
) -> Result<Json<Value>, ApiError> {
    require_service_auth(&state, &headers)?;
    let wait_ms = query.wait_ms.unwrap_or(0).min(60_000);
    let result = wait_for_relay_result(&state, machine_id, event_id, wait_ms).await?;
    match result {
        Some(result) => Ok(Json(serde_json::to_value(result)?)),
        None => Err(ApiError::not_found("result not available")),
    }
}

async fn admin_chat_snapshot(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(machine_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    require_service_auth(&state, &headers)?;
    match state.relay_store.read_chat_snapshot(&machine_id)? {
        Some(snapshot) => Ok(Json(serde_json::to_value(snapshot)?)),
        None => Err(ApiError::not_found("chat snapshot not found")),
    }
}

async fn admin_chat_conversations(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(machine_id): Path<String>,
    Query(query): Query<ChatConversationsQuery>,
) -> Result<Json<Value>, ApiError> {
    require_service_auth(&state, &headers)?;
    let bridge = RelayBridgeDevice {
        bridge_account_id: query.bridge_account_id,
        bridge_device_id: query.bridge_device_id,
    };
    let threads = state.relay_store.chat_threads(&machine_id, &bridge)?;
    Ok(Json(serde_json::to_value(threads)?))
}

async fn admin_create_chat_conversation(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(machine_id): Path<String>,
    Json(input): Json<CreateRelayChatConversationInput>,
) -> Result<Json<Value>, ApiError> {
    require_service_auth(&state, &headers)?;
    let thread = state
        .relay_store
        .create_chat_conversation(&machine_id, &input)?;
    state.chat_watchers.notify(&machine_id);
    Ok(Json(serde_json::to_value(thread)?))
}

async fn admin_update_chat_conversation(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path((machine_id, conversation_id)): Path<(String, String)>,
    Json(input): Json<UpdateRelayChatConversationInput>,
) -> Result<Json<Value>, ApiError> {
    require_service_auth(&state, &headers)?;
    let thread =
        state
            .relay_store
            .update_chat_conversation(&machine_id, &conversation_id, &input)?;
    state.chat_watchers.notify(&machine_id);
    Ok(Json(serde_json::to_value(thread)?))
}

async fn admin_chat_messages(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path((machine_id, conversation_id)): Path<(String, String)>,
    Query(query): Query<ChatMessagesQuery>,
) -> Result<Json<Value>, ApiError> {
    require_service_auth(&state, &headers)?;
    let bridge = RelayBridgeDevice {
        bridge_account_id: query.bridge_account_id,
        bridge_device_id: query.bridge_device_id,
    };
    let page = if let Some(project_agent_id) = query.project_agent_id.as_deref() {
        state.relay_store.chat_message_page(
            &machine_id,
            project_agent_id,
            &conversation_id,
            &bridge,
            query.limit,
            query.before.as_deref(),
        )?
    } else {
        state.relay_store.chat_message_page_for_machine(
            &machine_id,
            &conversation_id,
            &bridge,
            query.limit,
            query.before.as_deref(),
        )?
    };
    Ok(Json(serde_json::to_value(page)?))
}

async fn admin_send_chat_message(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path((machine_id, conversation_id)): Path<(String, String)>,
    Json(input): Json<SendRelayChatMessageInput>,
) -> Result<Json<Value>, ApiError> {
    require_service_auth(&state, &headers)?;
    let message = state
        .relay_store
        .send_chat_message(&machine_id, &conversation_id, &input)?;
    state.chat_watchers.notify(&machine_id);
    Ok(Json(serde_json::to_value(message)?))
}

async fn admin_chat_attachment(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path((machine_id, attachment_id)): Path<(String, String)>,
    Query(query): Query<ChatStreamQuery>,
) -> Result<Response, ApiError> {
    require_service_auth(&state, &headers)?;
    let bridge = RelayBridgeDevice {
        bridge_account_id: query.bridge_account_id,
        bridge_device_id: query.bridge_device_id,
    };
    let Some(attachment) =
        state
            .relay_store
            .read_chat_attachment(&machine_id, &attachment_id, &bridge)?
    else {
        return Err(ApiError::not_found("attachment not found"));
    };
    Ok(chat_attachment_response(attachment))
}

async fn admin_status_snapshot(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path((machine_id, state_key)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    require_service_auth(&state, &headers)?;
    match state
        .relay_store
        .read_status_snapshot(&machine_id, &state_key)?
    {
        Some(snapshot) => Ok(Json(serde_json::to_value(snapshot)?)),
        None => Err(ApiError::not_found("status snapshot not found")),
    }
}

async fn admin_chat_stream(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(machine_id): Path<String>,
    Query(query): Query<ChatStreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    require_service_auth(&state, &headers)?;
    let receiver = state.chat_watchers.subscribe(&machine_id);
    let bridge = RelayBridgeDevice {
        bridge_account_id: query.bridge_account_id,
        bridge_device_id: query.bridge_device_id,
    };
    let since = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or(query.since);
    let stream = chat_ledger_stream(state, machine_id, bridge, receiver, since);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn projects(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<PublicVisibleProject>>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    Ok(Json(public_visible_projects(
        state
            .store
            .visible_projects_for_workos_user(&identity.workos_user_id)
            .await?,
    )))
}

async fn authenticate_runtime_machine(
    state: &CoreApiState,
    headers: &HeaderMap,
) -> Result<String, ApiError> {
    let token =
        bearer_token(headers).ok_or_else(|| ApiError::unauthorized("missing runtime token"))?;
    let heartbeat = state.store.record_runtime_heartbeat(&token).await?;
    let _ = state.relay_store.heartbeat(&heartbeat.machine_id)?;
    Ok(heartbeat.machine_id)
}

async fn wait_for_relay_result(
    state: &CoreApiState,
    machine_id: String,
    event_id: String,
    wait_ms: u64,
) -> Result<Option<RelayResult>, ApiError> {
    if let Some(result) = state.relay_store.wait_result(&machine_id, &event_id)? {
        return Ok(Some(result));
    }
    if wait_ms == 0 {
        return Ok(None);
    }

    let deadline = Instant::now() + Duration::from_millis(wait_ms);
    loop {
        let notify = state.result_waiters.notify_for(&machine_id, &event_id);
        if let Some(result) = state.relay_store.wait_result(&machine_id, &event_id)? {
            state.result_waiters.remove(&machine_id, &event_id);
            return Ok(Some(result));
        }

        let now = Instant::now();
        if now >= deadline {
            state.result_waiters.remove(&machine_id, &event_id);
            return Ok(None);
        }

        if timeout(deadline.saturating_duration_since(now), notify.notified())
            .await
            .is_err()
        {
            state.result_waiters.remove(&machine_id, &event_id);
            return Ok(None);
        }
    }
}

fn chat_ledger_stream(
    state: CoreApiState,
    machine_id: String,
    bridge: RelayBridgeDevice,
    receiver: broadcast::Receiver<()>,
    since: Option<String>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    stream::unfold(
        (state, machine_id, bridge, receiver, since, true),
        |(state, machine_id, bridge, mut receiver, mut cursor, mut poll_now)| async move {
            loop {
                if !poll_now {
                    match receiver.recv().await {
                        Ok(()) => {}
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => return None,
                    }
                }

                let event = match state.relay_store.chat_stream_event(
                    &machine_id,
                    &bridge,
                    cursor.as_deref(),
                ) {
                    Ok(Some(update)) => {
                        let event_name = if update.reset {
                            "chat.ledger_snapshot"
                        } else {
                            "chat.ledger_update"
                        };
                        cursor = Some(update.cursor.clone());
                        Event::default()
                            .event(event_name)
                            .id(update.cursor.clone())
                            .data(
                                serde_json::to_string(&update).unwrap_or_else(|_| "{}".to_string()),
                            )
                    }
                    Ok(None) if cursor.is_some() => {
                        poll_now = false;
                        continue;
                    }
                    Ok(None) => Event::default()
                        .event("chat.empty")
                        .data(json!({ "machineId": &machine_id }).to_string()),
                    Err(error) => Event::default()
                        .event("chat.error")
                        .data(json!({ "error": error.to_string() }).to_string()),
                };

                return Some((
                    Ok(event),
                    (state, machine_id, bridge, receiver, cursor, false),
                ));
            }
        },
    )
}

fn runtime_bridge_device(machine_id: &str) -> RelayBridgeDevice {
    assert!(!machine_id.trim().is_empty());
    RelayBridgeDevice {
        bridge_account_id: format!("runtime:{machine_id}"),
        bridge_device_id: "finitec".to_string(),
    }
}

fn chat_attachment_response(attachment: RelayChatAttachmentData) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        HeaderValue::from_str(&attachment.mime_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        "cache-control",
        HeaderValue::from_static("private, max-age=60"),
    );
    headers.insert(
        "content-disposition",
        HeaderValue::from_str(&inline_content_disposition(&attachment.name))
            .unwrap_or_else(|_| HeaderValue::from_static("inline; filename=\"attachment\"")),
    );
    (StatusCode::OK, headers, attachment.bytes).into_response()
}

fn inline_content_disposition(filename: &str) -> String {
    let fallback = ascii_header_filename(filename);
    let encoded = percent_encode_header_value(filename.trim());
    format!("inline; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

fn ascii_header_filename(filename: &str) -> String {
    let sanitized = filename
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | ' ') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed.to_string()
    }
}

fn percent_encode_header_value(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        let keep = byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'.' | b'_' | b'~');
        if keep {
            encoded.push(*byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    assert!(encoded.len() >= value.len());
    encoded
}

impl RelayWaiters {
    fn notify_for(&self, machine_id: &str, event_id: &str) -> Arc<Notify> {
        let key = Self::key(machine_id, event_id);
        let mut inner = self.lock();
        inner
            .entry(key)
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone()
    }

    fn notify_result(&self, machine_id: &str, event_id: &str) {
        let notify = {
            let inner = self.lock();
            inner.get(&Self::key(machine_id, event_id)).cloned()
        };
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
    }

    fn remove(&self, machine_id: &str, event_id: &str) {
        self.lock().remove(&Self::key(machine_id, event_id));
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Arc<Notify>>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn key(machine_id: &str, event_id: &str) -> String {
        format!("{machine_id}:{event_id}")
    }
}

impl ChatWatchers {
    fn subscribe(&self, machine_id: &str) -> broadcast::Receiver<()> {
        let sender = {
            let mut inner = self.lock();
            inner
                .entry(machine_id.to_string())
                .or_insert_with(|| {
                    let (sender, _) = broadcast::channel(128);
                    sender
                })
                .clone()
        };
        sender.subscribe()
    }

    fn notify(&self, machine_id: &str) {
        let sender = {
            let inner = self.lock();
            inner.get(machine_id).cloned()
        };
        if let Some(sender) = sender {
            let _ = sender.send(());
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, broadcast::Sender<()>>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Debug)]
struct VerifiedIdentity {
    email: String,
    workos_user_id: String,
    workos_organization_id: Option<String>,
}

async fn require_verified_identity(
    state: &CoreApiState,
    headers: &HeaderMap,
) -> Result<VerifiedIdentity, ApiError> {
    if [
        WORKOS_USER_ID_HEADER,
        WORKOS_EMAIL_HEADER,
        WORKOS_EMAIL_VERIFIED_HEADER,
    ]
    .into_iter()
    .any(|name| headers.contains_key(name))
    {
        return Err(ApiError::unauthorized(
            "caller-supplied identity headers are not accepted",
        ));
    }

    let access_token =
        bearer_token(headers).ok_or_else(|| ApiError::unauthorized("sign in is required"))?;
    let session = state
        .auth
        .workos()
        .verify_access_token(&access_token)
        .await
        .map_err(|error| workos_api_error_at("access_token", error))?;
    let user = state
        .auth
        .workos()
        .verified_user(&session.subject)
        .await
        .map_err(|error| workos_api_error_at("user_lookup", error))?;
    let email = normalize_owner_email(Some(&user.email))
        .ok_or_else(|| ApiError::unauthorized("invalid account"))?;
    Ok(VerifiedIdentity {
        email,
        workos_user_id: session.subject,
        workos_organization_id: session.organization_id,
    })
}

/// Core-side operator authorization. The WorkOS organization claim is an
/// identity-provider predicate only and is never used as a Core Customer
/// Organization id.
async fn require_admin_identity(
    state: &CoreApiState,
    headers: &HeaderMap,
) -> Result<VerifiedIdentity, ApiError> {
    let identity = require_verified_identity(state, headers).await?;
    if identity.workos_organization_id.as_deref() != Some(state.auth.workos().operator_org_id()) {
        return Err(ApiError::forbidden(
            "admin access is required for this endpoint",
        ));
    }
    Ok(identity)
}

fn workos_api_error_at(stage: &'static str, error: WorkosAuthError) -> ApiError {
    tracing::warn!(stage, error = %error, "WorkOS authentication rejected");
    workos_api_error(error)
}

fn workos_api_error(error: WorkosAuthError) -> ApiError {
    match error {
        WorkosAuthError::UnverifiedUser => ApiError::forbidden("a verified email is required"),
        WorkosAuthError::Unavailable => {
            ApiError::service_unavailable("account authentication is temporarily unavailable")
        }
        WorkosAuthError::InvalidToken
        | WorkosAuthError::UnknownUser
        | WorkosAuthError::InvalidUser => ApiError::unauthorized("invalid account session"),
    }
}

fn require_service_auth(state: &CoreApiState, headers: &HeaderMap) -> Result<(), ApiError> {
    require_route_credential(
        headers,
        state.auth.service_api_token(),
        "invalid service credential",
    )
}

fn require_runner_auth(
    state: &CoreApiState,
    headers: &HeaderMap,
) -> Result<VerifiedRunnerCredential, ApiError> {
    let presented =
        bearer_token(headers).ok_or_else(|| ApiError::unauthorized("invalid Runner credential"))?;
    state
        .auth
        .verify_runner_credential(&presented)
        .ok_or_else(|| ApiError::unauthorized("invalid Runner credential"))
}

fn authorize_runner_id(
    credential: &VerifiedRunnerCredential,
    runner_id: &str,
) -> Result<(), ApiError> {
    if constant_time_token_eq(runner_id.trim(), &credential.runner_id) {
        Ok(())
    } else {
        Err(runner_binding_error())
    }
}

fn authorize_runner_capacity(
    credential: &VerifiedRunnerCredential,
    capacity: Option<RunnerLeaseCapacity>,
) -> Result<RunnerLeaseCapacity, ApiError> {
    let Some(mut capacity) = capacity else {
        if credential.legacy_kata_compatibility {
            return Ok(RunnerLeaseCapacity {
                runner_classes: credential.runner_classes.clone(),
                runtime_capabilities: Some(legacy_kata_runtime_capabilities()),
                ..RunnerLeaseCapacity::default()
            });
        }
        return Err(runner_binding_error());
    };
    if capacity.runner_classes.is_empty()
        || capacity.runner_classes.len() != credential.runner_classes.len()
        || !capacity
            .runner_classes
            .iter()
            .all(|class| credential.runner_classes.contains(class))
    {
        return Err(runner_binding_error());
    }
    if capacity.runtime_capabilities.is_none() {
        if credential.legacy_kata_compatibility {
            capacity.runtime_capabilities = Some(legacy_kata_runtime_capabilities());
        } else {
            return Err(runner_binding_error());
        }
    }
    capacity
        .validate_runtime_capability_policy()
        .map_err(|_| runner_binding_error())?;
    Ok(capacity)
}

fn legacy_kata_runtime_capabilities() -> RuntimeCapabilitiesEnvelope {
    RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
        restart: true,
        recover_known_good_chat: false,
        runtime_upgrade: true,
        stop: true,
        runtime_retirement: false,
    })
}

fn authorize_runner_runtime_capabilities(
    credential: &VerifiedRunnerCredential,
    capabilities: Option<RuntimeCapabilitiesEnvelope>,
) -> Result<RuntimeCapabilitiesEnvelope, ApiError> {
    let capabilities = match capabilities {
        Some(capabilities) => capabilities,
        None if credential.legacy_kata_compatibility => legacy_kata_runtime_capabilities(),
        None => return Err(runner_binding_error()),
    };
    RunnerLeaseCapacity {
        runner_classes: credential.runner_classes.clone(),
        runtime_capabilities: Some(capabilities.clone()),
        ..RunnerLeaseCapacity::default()
    }
    .validate_runtime_capability_policy()
    .map_err(|_| runner_binding_error())?;
    Ok(capabilities)
}

fn authorize_runner_source_host(
    credential: &VerifiedRunnerCredential,
    source_host_id: Option<&str>,
) -> Result<String, ApiError> {
    let source_host_id = match source_host_id {
        Some(source_host_id) => {
            normalize_source_host_id(source_host_id).map_err(|_| runner_binding_error())?
        }
        None if credential.legacy_kata_compatibility => credential.source_host_id.clone(),
        None => return Err(runner_binding_error()),
    };
    if constant_time_token_eq(&source_host_id, &credential.source_host_id) {
        Ok(source_host_id)
    } else {
        Err(runner_binding_error())
    }
}

fn authorize_provider_runtime_handle(
    credential: &VerifiedRunnerCredential,
    handle: Option<&ProviderRuntimeHandleEnvelope>,
) -> Result<(), ApiError> {
    if handle.is_none_or(|handle| credential.runner_classes.contains(&handle.runner_class())) {
        Ok(())
    } else {
        Err(runner_binding_error())
    }
}

fn runner_binding_error() -> ApiError {
    ApiError::forbidden("Runner credential is not authorized for this worker request")
}

fn require_route_credential(
    headers: &HeaderMap,
    expected_token: &str,
    error_message: &'static str,
) -> Result<(), ApiError> {
    let expected = format!("Bearer {expected_token}");
    if header_value(headers, SERVICE_AUTH_HEADER)
        .as_deref()
        .is_some_and(|presented| constant_time_token_eq(presented, &expected))
    {
        return Ok(());
    }

    Err(ApiError::unauthorized(error_message))
}

fn require_finite_private_usage_auth(
    state: &CoreApiState,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    require_route_credential(
        headers,
        state.auth.finite_private_usage_api_token(),
        "invalid finite private usage service token",
    )
}

fn constant_time_token_eq(presented: &str, expected: &str) -> bool {
    let presented_digest = Sha256::digest(presented.as_bytes());
    let expected_digest = Sha256::digest(expected.as_bytes());
    bool::from(presented_digest.ct_eq(&expected_digest))
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = header_value(headers, SERVICE_AUTH_HEADER)?;
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

impl ApiError {
    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
            correlation_id: None,
        }
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
            correlation_id: None,
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
            correlation_id: None,
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            correlation_id: None,
        }
    }

    fn service_unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: message.into(),
            correlation_id: None,
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
            correlation_id: None,
        }
    }

    fn payment_required(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::PAYMENT_REQUIRED,
            message: message.into(),
            correlation_id: None,
        }
    }

    /// A generic 500 that has already been logged server-side under
    /// `correlation_id`. The user never sees the underlying store detail.
    fn internal(correlation_id: String) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "internal error".to_string(),
            correlation_id: Some(correlation_id),
        }
    }
}

/// Monotonic, restart-derived correlation id for internal errors. Avoids random
/// UUIDs (which are constrained in some deploys); a process-start-relative nanos
/// value hashed with a per-process counter is unique enough to grep for and
/// quote, and mirrors the sha256-derived id style used in `lib.rs`.
fn next_correlation_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(nanos.to_le_bytes());
    hasher.update(seq.to_le_bytes());
    let digest = hasher.finalize();
    let hex = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("cid_{hex}")
}

impl From<CoreError> for ApiError {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::MissingSourceHostId
            | CoreError::InvalidSourceHostId
            | CoreError::InvalidSourceHostRelayUrl
            | CoreError::MissingSourceHostRelayAdminToken
            | CoreError::MissingAgentDisplayName
            | CoreError::MissingAgentCreationIdempotencyKey
            | CoreError::MissingLaunchCode
            | CoreError::InvalidLaunchCode
            | CoreError::MissingLaunchCodeBatchName
            | CoreError::InvalidLaunchCodeBatchName
            | CoreError::InvalidLaunchCodeBatchSize
            | CoreError::InvalidLaunchCodeBatchExpiry
            | CoreError::MissingStripeCustomerId
            | CoreError::MissingStripeSubscriptionId
            | CoreError::InvalidBillingSubscriptionStatus
            | CoreError::MissingAgentCreationRunnerId
            | CoreError::MissingAgentCreationLeaseToken
            | CoreError::InvalidAgentCreationLeaseDuration
            | CoreError::MissingSourceMachineId
            | CoreError::MissingRuntimeRelayTokenHash
            | CoreError::MissingRuntimeRelayToken
            | CoreError::MissingRuntimeArtifactId
            | CoreError::MissingRuntimeArtifactReference
            | CoreError::MissingRuntimeArtifactVersionLabel
            | CoreError::MissingRuntimeArtifactStateSchemaVersion
            | CoreError::MissingFinitePrivateApiKey
            | CoreError::MissingStripeStandardPriceId
            | CoreError::StripeSubscriptionPriceMismatch
            | CoreError::InvalidFinitePrivateUsageEstimate
            | CoreError::MissingAgentCreationFailureMessage
            | CoreError::MissingRuntimeControlFailureMessage
            | CoreError::InvalidTimestamp => Self {
                status: StatusCode::BAD_REQUEST,
                message: error.to_string(),
                correlation_id: None,
            },
            CoreError::AgentCreationRequestNotFound
            | CoreError::RuntimeHeartbeatNotFound
            | CoreError::RuntimeArtifactNotFound
            | CoreError::ProjectNotFound
            | CoreError::ProjectRuntimeNotFound
            | CoreError::RuntimeControlRequestNotFound
            | CoreError::BillingAccountNotFound
            | CoreError::FinitePrivateGrantNotFound
            | CoreError::FinitePrivateLimitProfileNotFound
            | CoreError::FinitePrivateReservationNotFound
            | CoreError::LaunchCodeBatchNotFound => Self::not_found(error.to_string()),
            CoreError::InvalidRuntimeRelayToken | CoreError::InvalidFinitePrivateApiKey => {
                Self::unauthorized(error.to_string())
            }
            CoreError::BillingRequired => Self::payment_required(error.to_string()),
            CoreError::AgentCreationEntitlementExhausted
            | CoreError::AgentCreationRequestUnavailable
            | CoreError::AgentCreationRequestLeaseConflict
            | CoreError::AgentCreationRequestNotLaunching
            | CoreError::AgentCreationRequestNotCancellable
            | CoreError::RuntimeArtifactNotPromoted
            | CoreError::RuntimeArtifactRetired
            | CoreError::RuntimeArtifactImmutable
            | CoreError::RuntimeCapabilitiesMismatch
            | CoreError::RuntimeCapabilitiesNotAuthorized
            | CoreError::RuntimeRestartUnsupported
            | CoreError::RuntimeControlUnsupported
            | CoreError::RuntimeUpgradeUnsupported
            | CoreError::RuntimeUpgradeNotEnabled
            | CoreError::RuntimeUpgradeStateSchemaIncompatible
            | CoreError::RuntimeUpgradeTargetConflict
            | CoreError::RuntimeControlOperationConflict
            | CoreError::RuntimeUpgradeCompletionMismatch
            | CoreError::RuntimeControlRequestNotRunning
            | CoreError::RuntimeControlRequestLeaseConflict
            | CoreError::FinitePrivateGrantNotActive
            | CoreError::FinitePrivateReservationAlreadySettled
            | CoreError::StripeCustomerConflict => Self::conflict(error.to_string()),
            // A store/DB failure with structured detail. Log the FULL detail
            // server-side (SQLSTATE code, constraint, table, column, DETAIL)
            // under a correlation id, then hand the user a generic 500 carrying
            // only that id. This is the arm whose absence turned "there is no
            // unique constraint matching the ON CONFLICT" into a bare "db error"
            // in the user's browser.
            CoreError::Database(detail) => {
                let correlation_id = next_correlation_id();
                tracing::error!(
                    correlation_id = %correlation_id,
                    code = detail.code.as_deref().unwrap_or("-"),
                    constraint = detail.constraint.as_deref().unwrap_or("-"),
                    table = detail.table.as_deref().unwrap_or("-"),
                    column = detail.column.as_deref().unwrap_or("-"),
                    detail = detail.detail.as_deref().unwrap_or("-"),
                    message = %detail.message,
                    "core store database error"
                );
                Self::internal(correlation_id)
            }
            // Remaining internal-ish variants (e.g. Store(String) invariants,
            // TimeFormat). Log under a correlation id and stay generic.
            other => {
                let correlation_id = next_correlation_id();
                tracing::error!(
                    correlation_id = %correlation_id,
                    error = %other,
                    "core internal error"
                );
                Self::internal(correlation_id)
            }
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        let correlation_id = next_correlation_id();
        tracing::error!(
            correlation_id = %correlation_id,
            error = %error,
            "core request failed"
        );
        Self::internal(correlation_id)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        let correlation_id = next_correlation_id();
        tracing::error!(
            correlation_id = %correlation_id,
            error = %error,
            "core request body deserialization failed"
        );
        Self::internal(correlation_id)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = match &self.correlation_id {
            Some(correlation_id) => json!({
                "error": self.message,
                "correlation_id": correlation_id,
            }),
            None => json!({
                "error": self.message,
            }),
        };
        (self.status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RunnerClass;
    use crate::auth::test_support::{
        BOUNDARY_RUNNER_TOKEN, FULL_RUNNER_TOKEN, OPERATOR_ORG_ID, SECOND_RUNNER_TOKEN,
        access_token, access_token_with_subject, core_auth, core_auth_with_runner_credentials,
        runner_credential_config, shared_route_core_auth,
    };
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    const TOKEN: &str = "core-token";

    fn test_auth() -> CoreAuth {
        shared_route_core_auth(TOKEN)
    }

    fn scoped_token(scope: &str) -> String {
        let digest = Sha256::digest(format!("{TOKEN}:{scope}").as_bytes());
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn scoped_test_auth() -> CoreAuth {
        core_auth(
            TOKEN,
            scoped_token("runner"),
            scoped_token("finite-private-usage"),
        )
    }

    fn runner_authorization() -> String {
        format!("Bearer {}", scoped_token("runner"))
    }

    fn boundary_runner_authorization() -> String {
        format!("Bearer {BOUNDARY_RUNNER_TOKEN}")
    }

    fn usage_authorization() -> String {
        format!("Bearer {}", scoped_token("finite-private-usage"))
    }

    fn runtime_capabilities_json(runtime_upgrade: bool) -> serde_json::Value {
        serde_json::to_value(RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
            restart: true,
            recover_known_good_chat: false,
            runtime_upgrade,
            stop: true,
            runtime_retirement: false,
        }))
        .unwrap()
    }

    fn runner_capacity_json(runner_class: RunnerClass) -> serde_json::Value {
        serde_json::json!({
            "runnerClasses": [runner_class],
            "runtimeCapabilities": runtime_capabilities_json(runner_class == RunnerClass::Kata),
        })
    }

    fn assert_json_omits_keys(value: &serde_json::Value, forbidden: &[&str]) {
        match value {
            serde_json::Value::Object(object) => {
                for key in forbidden {
                    assert!(
                        !object.contains_key(*key),
                        "public JSON unexpectedly contained `{key}`: {value}"
                    );
                }
                for child in object.values() {
                    assert_json_omits_keys(child, forbidden);
                }
            }
            serde_json::Value::Array(values) => {
                for child in values {
                    assert_json_omits_keys(child, forbidden);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn user_agent_creation_json_cannot_select_provider_placement() {
        let error = serde_json::from_value::<CreateAgentRequest>(serde_json::json!({
            "displayName": "Provider injection",
            "launchCode": "finite_test",
            "idempotencyKey": "provider-injection",
            "runnerClass": "phala"
        }))
        .expect_err("runnerClass must remain outside the user boundary");
        assert!(error.to_string().contains("unknown field `runnerClass`"));
    }

    #[test]
    fn runner_capability_authorization_is_explicit_and_legacy_kata_is_narrow() {
        let current_kata = VerifiedRunnerCredential {
            credential_id: "kata-current".to_string(),
            runner_id: "kata-worker".to_string(),
            runner_classes: vec![RunnerClass::Kata],
            source_host_id: "kata-host".to_string(),
            legacy_kata_compatibility: false,
        };
        assert!(authorize_runner_capacity(&current_kata, None).is_err());
        assert!(
            authorize_runner_runtime_capabilities(&current_kata, None).is_err(),
            "current credentials must advertise on registration and completion"
        );
        assert!(
            authorize_runner_capacity(
                &current_kata,
                Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                })
            )
            .is_err()
        );

        let legacy_kata = VerifiedRunnerCredential {
            legacy_kata_compatibility: true,
            ..current_kata.clone()
        };
        let compatibility = authorize_runner_capacity(&legacy_kata, None).unwrap();
        let RuntimeCapabilitiesEnvelope::V1(capabilities) =
            compatibility.runtime_capabilities.unwrap();
        assert_eq!(
            capabilities,
            RuntimeCapabilitiesV1 {
                restart: true,
                recover_known_good_chat: false,
                runtime_upgrade: true,
                stop: true,
                runtime_retirement: false,
            }
        );
        assert_eq!(
            authorize_runner_runtime_capabilities(&legacy_kata, None).unwrap(),
            legacy_kata_runtime_capabilities()
        );

        for overclaim in [
            RuntimeCapabilitiesV1 {
                recover_known_good_chat: true,
                ..RuntimeCapabilitiesV1::default()
            },
            RuntimeCapabilitiesV1 {
                runtime_retirement: true,
                ..RuntimeCapabilitiesV1::default()
            },
        ] {
            assert!(
                authorize_runner_capacity(
                    &current_kata,
                    Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(overclaim)),
                        ..RunnerLeaseCapacity::default()
                    })
                )
                .is_err()
            );
        }

        let current_phala = VerifiedRunnerCredential {
            credential_id: "phala-current".to_string(),
            runner_id: "phala-worker".to_string(),
            runner_classes: vec![RunnerClass::Phala],
            source_host_id: "phala-host".to_string(),
            legacy_kata_compatibility: false,
        };
        assert!(
            authorize_runner_capacity(
                &current_phala,
                Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Phala],
                    runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                        RuntimeCapabilitiesV1 {
                            runtime_upgrade: true,
                            ..RuntimeCapabilitiesV1::default()
                        }
                    )),
                    ..RunnerLeaseCapacity::default()
                })
            )
            .is_err()
        );
        assert!(
            authorize_runner_capacity(
                &current_phala,
                Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Phala],
                    runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                        RuntimeCapabilitiesV1 {
                            restart: true,
                            stop: true,
                            ..RuntimeCapabilitiesV1::default()
                        }
                    )),
                    ..RunnerLeaseCapacity::default()
                })
            )
            .is_ok()
        );
    }

    #[test]
    fn public_runtime_contact_prefers_explicit_normalized_endpoint_over_n_minus_one_urls() {
        let runtime = AgentRuntime {
            id: "runtime-public-contact".to_string(),
            project_id: "project-public-contact".to_string(),
            source_host_id: "internal-host".to_string(),
            source_machine_id: "internal-machine".to_string(),
            source_import_key: "internal-host:internal-machine".to_string(),
            runtime_artifact_id: None,
            state_schema_version: None,
            placement: None,
            provider_runtime_handle: None,
            provider_runtime_handle_history: Vec::new(),
            contact_endpoint: Some("https://contact.example.test/".to_string()),
            runtime_capabilities: None,
            host_facts: crate::HostOwnedRuntimeFacts {
                display_name: "Contact test".to_string(),
                hostname: None,
                runtime_host: "internal-host".to_string(),
                runtime_status: RuntimeSummaryStatus::Online,
                active_inference_profile: None,
                hermes_available: Some(true),
                published_app_urls: vec!["https://legacy.example.test/wrong".to_string()],
            },
            created_at: "2026-07-11T12:00:00Z".to_string(),
            updated_at: "2026-07-11T12:00:00Z".to_string(),
        };

        assert_eq!(
            public_runtime_contact_endpoint(&runtime).as_deref(),
            Some("https://contact.example.test")
        );
        let legacy_runtime = AgentRuntime {
            contact_endpoint: None,
            host_facts: crate::HostOwnedRuntimeFacts {
                published_app_urls: vec![
                    "not-a-contact".to_string(),
                    "https://legacy.example.test/contact/".to_string(),
                ],
                ..runtime.host_facts.clone()
            },
            ..runtime
        };
        assert_eq!(
            public_runtime_contact_endpoint(&legacy_runtime).as_deref(),
            Some("https://legacy.example.test/contact")
        );

        let public_legacy = PublicAgentRuntime::from(legacy_runtime.clone());
        assert!(public_legacy.runtime_capabilities.is_none());
        assert!(
            serde_json::to_value(public_legacy)
                .unwrap()
                .get("runtime_capabilities")
                .is_none()
        );

        let public_current = PublicAgentRuntime::from(AgentRuntime {
            runtime_capabilities: Some(legacy_kata_runtime_capabilities()),
            ..legacy_runtime
        });
        assert_eq!(
            public_current.runtime_capabilities,
            Some(PublicRuntimeCapabilities {
                restart: true,
                recover_known_good_chat: false,
                runtime_upgrade: true,
                stop: true,
                runtime_retirement: false,
            })
        );
    }

    async fn issue_test_launch_code(store: &CoreStore) -> String {
        store
            .issue_launch_code_batch(crate::launch_codes::IssueLaunchCodeBatchInput {
                name: "Core API test batch".to_string(),
                code_count: 1,
                expires_in_hours: Some(crate::launch_codes::MAX_LAUNCH_CODE_BATCH_HOURS),
                hosting_tier: None,
                created_by_workos_user_id: "workos-test-operator".to_string(),
                now: None,
            })
            .await
            .expect("test Launch Code batch should issue")
            .codes
            .into_iter()
            .next()
            .expect("one test Launch Code should be returned")
            .code
    }

    #[test]
    fn runtime_control_request_view_redacts_runner_lease_fields() {
        let view = RuntimeControlRequestView::from(crate::RuntimeControlRequest {
            id: "runtime_ctl_123".to_string(),
            project_id: "project_123".to_string(),
            agent_runtime_id: "runtime_123".to_string(),
            source_host_id: "oslo-host-1".to_string(),
            source_machine_id: "oslo-agent-001".to_string(),
            requested_by_user_id: "user_123".to_string(),
            kind: crate::RuntimeControlKind::Destroy,
            target_runtime_artifact_id: None,
            status: crate::RuntimeControlRequestStatus::Running,
            runner_id: Some("runner-1".to_string()),
            lease_token: Some("secret-lease-token".to_string()),
            lease_expires_at: Some("2026-05-25T13:10:00Z".to_string()),
            failure_message: None,
            created_at: "2026-05-25T13:00:00Z".to_string(),
            updated_at: "2026-05-25T13:00:00Z".to_string(),
            completed_at: None,
        });
        let json = serde_json::to_value(view).unwrap();

        assert!(json.get("runner_id").is_none());
        assert!(json.get("lease_token").is_none());
        assert!(json.get("lease_expires_at").is_none());
        assert_eq!(json["source_machine_id"], "oslo-agent-001");
    }

    #[tokio::test]
    async fn core_api_reconciles_claims_and_lists_visible_projects() {
        let app = router(CoreStore::memory(), test_auth());
        let reconcile = serde_json::to_vec(&ReconcileImportsRequest {
            records: vec![ExistingHostProjectImport {
                source_host_id: "smoke".to_string(),
                source_machine_id: "test-smoke".to_string(),
                owner_email: Some("test@finite.vip".to_string()),
                display_name: "Smoke".to_string(),
                hostname: Some("smoke.example.com".to_string()),
                runtime_host: Some("smoke".to_string()),
                runtime_status: crate::RuntimeSummaryStatus::Online,
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: vec![
                    "not-a-contact-url".to_string(),
                    "https://smoke.example.com/contact/".to_string(),
                ],
                known_external_channel_participants: Vec::new(),
                admin_visible_to_emails: Vec::new(),
            }],
            allowlisted_owner_emails: vec!["test@finite.vip".to_string()],
            now: Some("2026-05-25T12:00:00Z".to_string()),
        })
        .unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/import-candidates/reconcile")
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(reconcile))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_test",
                                "test@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let me_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(me_json.get("workos_user_id").is_some());
        assert!(me_json.get("claimable_candidates").is_some());
        assert!(me_json.get("agent_creation_requests").is_some());
        assert!(me_json.get("workosUserId").is_none());
        assert!(me_json.get("claimableCandidates").is_none());
        assert!(me_json.get("agentCreationRequests").is_none());
        assert_json_omits_keys(
            &me_json,
            &[
                "runner_class",
                "placement",
                "source_host_id",
                "source_machine_id",
                "source_import_key",
                "provider_runtime_handle",
                "provider_runtime_handle_history",
                "published_app_urls",
                "host_facts",
            ],
        );
        let me: MeResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(me.claimable_candidates.len(), 1);
        assert_eq!(me.claimable_candidates[0].display_name, "Smoke");
        assert!(me.projects.is_empty());
        assert!(me.agent_creation_requests.is_empty());

        let claim = serde_json::to_vec(&ClaimImportsRequest {
            selected_candidate_ids: vec![me.claimable_candidates[0].id.clone()],
            now: Some("2026-05-25T13:00:00Z".to_string()),
        })
        .unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/import-candidates/claim")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_test",
                                "test@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(claim))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let claim_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(claim_json.get("claimed_project_ids").is_some());
        assert!(claim_json.get("claimedProjectIds").is_none());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me/projects")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_test",
                                "test@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let projects: Vec<PublicVisibleProject> = serde_json::from_slice(&body).unwrap();
        assert!(
            projects.is_empty(),
            "legacy imports stay out of the product DTO"
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me/runtime-routes/test-smoke")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_test",
                                "test@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_prod_google",
                                "test@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let relinked_me: MeResponse = serde_json::from_slice(&body).unwrap();
        assert!(relinked_me.projects.is_empty());
        assert_eq!(relinked_me.workos_user_id, "user_workos_prod_google");
    }

    #[tokio::test]
    async fn core_api_stores_source_host_relay_endpoints_behind_service_auth() {
        let app = router(CoreStore::memory(), test_auth());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/core/v1/source-host-relays/Smoke")
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "url": "https://relay.smoke.finite.computer/",
                            "adminToken": "smoke-token",
                            "now": "2026-05-25T12:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let endpoint: SourceHostRelayEndpoint = serde_json::from_slice(&body).unwrap();
        assert_eq!(endpoint.source_host_id, "smoke");
        assert_eq!(endpoint.url, "https://relay.smoke.finite.computer");
        assert_eq!(endpoint.admin_token, "smoke-token");

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/source-host-relays/smoke")
                    .header("authorization", "Bearer core-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/source-host-relays/smoke")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn core_api_serves_finite_private_operator_grant_and_key_lifecycle() {
        let app = router(CoreStore::memory(), test_auth());
        let operator_authorization = format!(
            "Bearer {}",
            access_token_with_subject(
                "operator_finite_private",
                "admin@finite.vip",
                true,
                Some(OPERATOR_ORG_ID),
            )
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/finite-private/grants")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "verifiedEmail": "private@finite.vip",
                            "workosUserId": "user_workos_private",
                            "now": "2026-05-26T12:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/finite-private/grants")
                    .header("authorization", &operator_authorization)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "verifiedEmail": "private@finite.vip",
                            "workosUserId": "user_workos_private",
                            "now": "2026-05-26T12:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let grant: FinitePrivateGrant = serde_json::from_slice(&body).unwrap();
        assert_eq!(grant.status, crate::FinitePrivateGrantStatus::Active);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/finite-private/grants/{}/api-keys",
                        grant.id
                    ))
                    .header("authorization", &operator_authorization)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "rawKey": "fpk_live_old",
                            "projectId": "proj_private",
                            "agentRuntimeId": "runtime_private",
                            "now": "2026-05-26T12:01:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let old_key: FinitePrivateApiKey = serde_json::from_slice(&body).unwrap();
        assert_eq!(old_key.status, crate::FinitePrivateApiKeyStatus::Active);
        assert_eq!(old_key.grant_id, grant.id);
        assert_ne!(old_key.key_hash, "fpk_live_old");

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/finite-private/v1/reservations")
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "requestId": "req-private-admin-before-reset",
                            "presentedApiKey": "fpk_live_old",
                            "endpoint": "/v1/chat/completions",
                            "model": "kimi-k2-6",
                            "estimatedPromptTokens": 50,
                            "estimatedCompletionTokens": 100,
                            "estimatedUsageUnits": 350,
                            "usageFormulaVersion": "2026-05-26.v1",
                            "dashboardUrl": "https://finite.computer/dashboard",
                            "now": "2026-05-26T12:02:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let decision: FinitePrivateUsageDecision = serde_json::from_slice(&body).unwrap();
        assert_eq!(decision.decision, "allow");

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/finite-private/grants/{}/reset",
                        grant.id
                    ))
                    .header("authorization", &operator_authorization)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "now": "2026-05-26T12:03:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let reset_grant: FinitePrivateGrant = serde_json::from_slice(&body).unwrap();
        assert_eq!(reset_grant.current_window_used_units, 0);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/finite-private/api-keys/{}/rotate",
                        old_key.id
                    ))
                    .header("authorization", &operator_authorization)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "rawKey": "fpk_live_new",
                            "now": "2026-05-26T12:04:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let new_key: FinitePrivateApiKey = serde_json::from_slice(&body).unwrap();
        assert_ne!(new_key.id, old_key.id);
        assert_eq!(new_key.status, crate::FinitePrivateApiKeyStatus::Active);
        assert_eq!(new_key.grant_id, grant.id);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/finite-private/v1/reservations")
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "requestId": "req-private-admin-old-key-denied",
                            "presentedApiKey": "fpk_live_old",
                            "endpoint": "/v1/chat/completions",
                            "model": "kimi-k2-6",
                            "estimatedPromptTokens": 50,
                            "estimatedCompletionTokens": 100,
                            "estimatedUsageUnits": 350,
                            "usageFormulaVersion": "2026-05-26.v1",
                            "dashboardUrl": "https://finite.computer/dashboard",
                            "now": "2026-05-26T12:05:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let decision: FinitePrivateUsageDecision = serde_json::from_slice(&body).unwrap();
        assert_eq!(decision.decision, "deny");

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/finite-private/api-keys/{}/revoke",
                        new_key.id
                    ))
                    .header("authorization", &operator_authorization)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "now": "2026-05-26T12:06:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let revoked_key: FinitePrivateApiKey = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            revoked_key.status,
            crate::FinitePrivateApiKeyStatus::Revoked
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/core/v1/finite-private/admin-audit-events")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token("admin@finite.vip", true, Some(OPERATOR_ORG_ID),)
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let audit_events: Vec<FinitePrivateAdminAuditEvent> =
            serde_json::from_slice(&body).unwrap();
        assert!(
            audit_events
                .iter()
                .any(|event| event.action == "finite_private.api_key.rotate")
        );
        let audit_json = serde_json::to_string(&audit_events).unwrap();
        assert!(!audit_json.contains("fpk_live_old"));
        assert!(!audit_json.contains("fpk_live_new"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/core/v1/finite-private/admin-state")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token("admin@finite.vip", true, Some(OPERATOR_ORG_ID),)
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let admin_state: crate::FinitePrivateAdminState = serde_json::from_slice(&body).unwrap();
        assert_eq!(admin_state.grants.len(), 1);
        assert_eq!(admin_state.api_keys.len(), 2);
        assert!(
            admin_state.api_keys.iter().any(|key| key.id == old_key.id
                && key.status == crate::FinitePrivateApiKeyStatus::Revoked)
        );
        let admin_state_json = serde_json::to_string(&admin_state).unwrap();
        assert!(!admin_state_json.contains("fpk_live_old"));
        assert!(!admin_state_json.contains("fpk_live_new"));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/finite-private/grants/{}/revoke",
                        grant.id
                    ))
                    .header("authorization", &operator_authorization)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "now": "2026-05-26T12:07:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let revoked_grant: FinitePrivateGrant = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            revoked_grant.status,
            crate::FinitePrivateGrantStatus::Revoked
        );
    }

    #[tokio::test]
    async fn core_api_serves_finite_private_usage_reserve_and_settle() {
        let store = CoreStore::memory();
        let grant = store
            .approve_finite_private_grant(crate::ApproveFinitePrivateGrantInput {
                verified_email: "private@finite.vip".to_string(),
                workos_user_id: Some("user_workos_private".to_string()),
                limit_profile_id: None,
                now: Some("2026-05-25T12:00:00Z".to_string()),
            })
            .await
            .unwrap();
        store
            .issue_finite_private_api_key(crate::IssueFinitePrivateApiKeyInput {
                grant_id: grant.id,
                raw_key: "fpk_live_secret".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: Some("2026-05-25T12:00:00Z".to_string()),
            })
            .await
            .unwrap();
        let app = router(store, test_auth());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/internal/finite-private/v1/health")
                    .header("authorization", "Bearer core-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/finite-private/v1/reservations")
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "requestId": "req-private-api-1",
                            "presentedApiKey": "fpk_live_secret",
                            "endpoint": "/v1/chat/completions",
                            "model": "kimi-k2-6",
                            "estimatedPromptTokens": 120000,
                            "estimatedCompletionTokens": 4096,
                            "estimatedUsageUnits": 250000,
                            "usageFormulaVersion": "2026-05-26.v1",
                            "dashboardUrl": "https://finite.computer/dashboard",
                            "now": "2026-05-25T13:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let decision: FinitePrivateUsageDecision = serde_json::from_slice(&body).unwrap();
        assert_eq!(decision.decision, "allow");
        let reservation_id = decision.reservation_id.unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/internal/finite-private/v1/reservations/{reservation_id}/settle"
                    ))
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "requestId": "req-private-api-1",
                            "settlement": "actual",
                            "promptTokens": 120000,
                            "completionTokens": 1200,
                            "usageUnits": 160000,
                            "usageFormulaVersion": "2026-05-26.v1",
                            "upstreamStatus": 200,
                            "upstreamErrorClass": null,
                            "now": "2026-05-25T13:05:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let settled: SettleFinitePrivateReservationResult = serde_json::from_slice(&body).unwrap();
        assert!(settled.settled);
        assert_eq!(settled.reservation_id, reservation_id);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/finite-private/v1/reservations")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "requestId": "req-private-api-unauth",
                            "presentedApiKey": "fpk_live_secret",
                            "endpoint": "/v1/chat/completions",
                            "model": "kimi-k2-6",
                            "estimatedPromptTokens": 100,
                            "estimatedCompletionTokens": 100,
                            "estimatedUsageUnits": 200,
                            "usageFormulaVersion": "2026-05-26.v1",
                            "dashboardUrl": "https://finite.computer/dashboard"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn core_api_creates_self_serve_agent_request_with_launch_code() {
        let store = CoreStore::memory();
        let launch_code = issue_test_launch_code(&store).await;
        let app = router(store, test_auth());
        let create = serde_json::to_vec(&CreateAgentRequest {
            display_name: "Oslo Agent".to_string(),
            launch_code: launch_code.clone(),
            idempotency_key: "browser-submit-1".to_string(),
            profile_picture_url: Some("https://chat.finite.computer/v1/blobs/profile".to_string()),
        })
        .unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/agent-creation-requests")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(create.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let result: RequestAgentCreationResult = serde_json::from_slice(&body).unwrap();
        assert_eq!(result.project.display_name, "Oslo Agent");
        assert!(result.project.import_candidate_id.is_none());
        assert!(result.request.agent_runtime_id.is_none());
        assert_eq!(result.request.runner_class, RunnerClass::Kata);
        assert_eq!(
            result.request.profile_picture_url.as_deref(),
            Some("https://chat.finite.computer/v1/blobs/profile")
        );
        assert!(!result.reused);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let me: MeResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(me.projects.len(), 1);
        assert_eq!(me.projects[0].project.id, result.project.id);
        assert!(me.projects[0].runtime.is_none());
        assert_eq!(me.agent_creation_requests.len(), 1);
        assert_eq!(me.agent_creation_requests[0].project_id, result.project.id);
        assert_eq!(
            me.agent_creation_requests[0].status,
            crate::AgentCreationRequestStatus::Requested
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/agent-creation-requests")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(create))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let retry: RequestAgentCreationResult = serde_json::from_slice(&body).unwrap();
        assert!(retry.reused);
        assert_eq!(retry.project.id, result.project.id);

        let second = serde_json::to_vec(&CreateAgentRequest {
            display_name: "Second Agent".to_string(),
            launch_code: launch_code.clone(),
            idempotency_key: "browser-submit-2".to_string(),
            profile_picture_url: None,
        })
        .unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/agent-creation-requests")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(second))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn core_api_uses_server_time_for_launch_code_expiry() {
        let store = CoreStore::memory();
        let issued = store
            .issue_launch_code_batch(crate::launch_codes::IssueLaunchCodeBatchInput {
                name: "Expired browser code".to_string(),
                code_count: 1,
                expires_in_hours: Some(1),
                hosting_tier: None,
                created_by_workos_user_id: "workos-test-operator".to_string(),
                now: Some("2020-01-01T00:00:00Z".to_string()),
            })
            .await
            .expect("expired test batch should issue");
        let plaintext = issued.codes[0].code.clone();
        let app = router(store.clone(), test_auth());
        let user = identity_headers("expired-code@finite.vip", "true");
        let request = serde_json::json!({
            "displayName": "Expired Agent",
            "launchCode": plaintext,
            "idempotencyKey": "expired-browser-submit"
        });

        let mut forged = request.clone();
        forged["now"] = serde_json::json!("2020-01-01T00:30:00Z");
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/me/agent-creation-requests",
            &user,
            Some(forged),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/me/agent-creation-requests",
            &user,
            Some(request),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let batches = store.list_launch_code_batches().await.unwrap();
        assert_eq!(batches.len(), 1);
        assert!(batches[0].codes[0].redeemed_at.is_none());
        assert!(batches[0].codes[0].redeemed_customer_org_id.is_none());
    }

    #[tokio::test]
    async fn core_api_lets_runner_lease_and_complete_agent_creation_request() {
        let store = CoreStore::memory();
        let launch_code = issue_test_launch_code(&store).await;
        let app = router(store, scoped_test_auth());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/core/v1/runtime-artifacts/artifact-v1")
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "kind": "oci_image",
                            "reference": format!(
                                "ghcr.io/finitecomputer/agent-runtime:v1@sha256:{}",
                                "a".repeat(64)
                            ),
                            "versionLabel": "v1",
                            "stateSchemaVersion": "state-v1",
                            "baseImage": "python:3.11-trixie",
                            "promoted": true,
                            "now": "2026-05-25T12:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let create = serde_json::to_vec(&CreateAgentRequest {
            display_name: "Oslo Agent".to_string(),
            launch_code: launch_code.clone(),
            idempotency_key: "browser-submit-1".to_string(),
            profile_picture_url: None,
        })
        .unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/agent-creation-requests")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(create))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/agent-creation-requests/lease")
                    .header("authorization", runner_authorization())
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "runnerId": "runner-oslo-1",
                            "leaseToken": "lease-token-1",
                            "leaseSeconds": 300,
                            "runnerCapacity": runner_capacity_json(RunnerClass::Kata),
                            "now": "2026-05-25T13:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let lease: Option<AgentCreationLease> = serde_json::from_slice(&body).unwrap();
        let lease = lease.unwrap();
        assert_eq!(
            lease.request.status,
            crate::AgentCreationRequestStatus::Launching
        );
        assert_eq!(lease.request.runner_id.as_deref(), Some("runner-oslo-1"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let me: MeResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(me.agent_creation_requests.len(), 1);
        assert_eq!(
            me.agent_creation_requests[0].status,
            crate::AgentCreationRequestStatus::Launching
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/agent-creation-requests/{}/complete",
                        lease.request.id
                    ))
                    .header("authorization", runner_authorization())
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "runnerId": "runner-oslo-1",
                            "leaseToken": "lease-token-1",
                            "sourceHostId": "oslo-host-1",
                            "sourceMachineId": "oslo-agent-001",
                            "runtimeArtifactId": "artifact-v1",
                            "contactEndpoint": "https://oslo-agent.example.test/contact/",
                            "hostname": "oslo-agent-001.finite.computer",
                            "runtimeHost": "oslo-host-1",
                            "runtimeStatus": "online",
                            "activeInferenceProfile": "finite-private",
                            "hermesAvailable": true,
                            "publishedAppUrls": [],
                            "runtimeCapabilities": runtime_capabilities_json(true),
                            "now": "2026-05-25T13:01:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let completed: AgentCreationLease = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            completed.request.status,
            crate::AgentCreationRequestStatus::Running
        );
        assert!(completed.request.agent_runtime_id.is_some());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me/projects")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let projects_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_json_omits_keys(
            &projects_json,
            &[
                "runner_class",
                "placement",
                "source_host_id",
                "source_machine_id",
                "source_import_key",
                "provider_runtime_handle",
                "provider_runtime_handle_history",
                "published_app_urls",
                "host_facts",
            ],
        );
        let projects: Vec<PublicVisibleProject> = serde_json::from_slice(&body).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(
            projects[0].runtime.as_ref().unwrap().runtime_status,
            RuntimeSummaryStatus::Online
        );
        assert_eq!(
            projects[0]
                .runtime
                .as_ref()
                .unwrap()
                .contact_endpoint
                .as_deref(),
            Some("https://oslo-agent.example.test/contact")
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me/runtime-routes/oslo-agent-001")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resolution: RuntimeRouteResolution = serde_json::from_slice(&body).unwrap();
        assert_eq!(resolution.project_id, projects[0].project.id);
        assert_eq!(
            resolution.runtime_id,
            projects[0].runtime.as_ref().unwrap().id
        );
    }

    #[tokio::test]
    async fn core_api_skips_full_or_draining_runner_without_blocking_other_runner() {
        let store = CoreStore::memory();
        let launch_code = issue_test_launch_code(&store).await;
        let app = router(store, test_auth());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/core/v1/runtime-artifacts/artifact-v1")
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "kind": "oci_image",
                            "reference": format!(
                                "ghcr.io/finitecomputer/agent-runtime:v1@sha256:{}",
                                "a".repeat(64)
                            ),
                            "versionLabel": "v1",
                            "stateSchemaVersion": "state-v1",
                            "baseImage": "python:3.11-trixie",
                            "promoted": true,
                            "now": "2026-05-25T12:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/agent-creation-requests")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "displayName": "Oslo Agent",
                            "launchCode": launch_code.clone(),
                            "idempotencyKey": "browser-submit-1"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        for runner_capacity in [
            serde_json::json!({
                "draining": true,
                "maxSandboxCount": 4,
                "activeSandboxCount": 1,
                "availableMemoryBytes": 8589934592_u64,
                "runnerClasses": ["kata"],
                "runtimeCapabilities": runtime_capabilities_json(true)
            }),
            serde_json::json!({
                "draining": false,
                "maxSandboxCount": 1,
                "activeSandboxCount": 1,
                "availableMemoryBytes": 1073741824_u64,
                "runnerClasses": ["kata"],
                "runtimeCapabilities": runtime_capabilities_json(true)
            }),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/core/v1/agent-creation-requests/lease")
                        .header("authorization", format!("Bearer {FULL_RUNNER_TOKEN}"))
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::json!({
                                "runnerId": "runner-oslo-full",
                                "leaseToken": "lease-token-full",
                                "leaseSeconds": 300,
                                "runnerCapacity": runner_capacity,
                                "now": "2026-05-25T13:00:00Z"
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let lease: Option<AgentCreationLease> = serde_json::from_slice(&body).unwrap();
            assert!(lease.is_none());
        }

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/agent-creation-requests/lease")
                    .header("authorization", format!("Bearer {SECOND_RUNNER_TOKEN}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "runnerId": "runner-oslo-2",
                            "leaseToken": "lease-token-2",
                            "leaseSeconds": 300,
                            "runnerCapacity": {
                                "draining": false,
                                "maxSandboxCount": 4,
                                "activeSandboxCount": 1,
                                "availableMemoryBytes": 8589934592_u64,
                                "runnerClasses": ["kata"],
                                "runtimeCapabilities": runtime_capabilities_json(true)
                            },
                            "now": "2026-05-25T13:00:01Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let lease: Option<AgentCreationLease> = serde_json::from_slice(&body).unwrap();
        let lease = lease.expect("available runner should lease queued work");
        assert_eq!(lease.request.runner_id.as_deref(), Some("runner-oslo-2"));
    }

    #[tokio::test]
    async fn core_api_lets_operator_cancel_failed_agent_creation_request() {
        let store = CoreStore::memory();
        let launch_code = issue_test_launch_code(&store).await;
        let app = router(store, test_auth());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/core/v1/runtime-artifacts/artifact-v1")
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "kind": "oci_image",
                            "reference": format!(
                                "ghcr.io/finitecomputer/agent-runtime:v1@sha256:{}",
                                "a".repeat(64)
                            ),
                            "versionLabel": "v1",
                            "stateSchemaVersion": "state-v1",
                            "promoted": true,
                            "now": "2026-05-25T12:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let create = serde_json::to_vec(&CreateAgentRequest {
            display_name: "Oslo Agent".to_string(),
            launch_code: launch_code.clone(),
            idempotency_key: "browser-submit-1".to_string(),
            profile_picture_url: None,
        })
        .unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/agent-creation-requests")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(create))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: RequestAgentCreationResult = serde_json::from_slice(&body).unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/agent-creation-requests/lease")
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "runnerId": "runner-oslo-1",
                            "leaseToken": "lease-token-1",
                            "leaseSeconds": 300,
                            "now": "2026-05-25T13:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/agent-creation-requests/{}/fail",
                        created.request.id
                    ))
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "runnerId": "runner-oslo-1",
                            "leaseToken": "lease-token-1",
                            "failureMessage": "runtime did not publish a relay heartbeat",
                            "now": "2026-05-25T13:01:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/agent-creation-requests/{}/cancel",
                        created.request.id
                    ))
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let cancelled: AgentCreationRequest = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            cancelled.status,
            crate::AgentCreationRequestStatus::Cancelled
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me/projects")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_new",
                                "new@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let projects: Vec<PublicVisibleProject> = serde_json::from_slice(&body).unwrap();
        assert!(projects.is_empty());
    }

    #[tokio::test]
    async fn core_api_serves_runtime_chat_relay_endpoints() {
        let relay_dir = tempfile::tempdir().unwrap();
        let store = CoreStore::memory();
        let launch_code = issue_test_launch_code(&store).await;
        let app = router_with_relay_state_dir(store, test_auth(), relay_dir.path());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/core/v1/runtime-artifacts/artifact-v1")
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "kind": "oci_image",
                            "reference": format!(
                                "ghcr.io/finitecomputer/agent-runtime:v1@sha256:{}",
                                "a".repeat(64)
                            ),
                            "versionLabel": "v1",
                            "stateSchemaVersion": "state-v1",
                            "baseImage": "python:3.11-trixie",
                            "promoted": true,
                            "now": "2026-05-25T12:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/agent-creation-requests")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "user_workos_chat",
                                "chat@finite.vip",
                                true,
                                None,
                            )
                        ),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "displayName": "Chat Agent",
                            "launchCode": launch_code.clone(),
                            "idempotencyKey": "chat-submit-1"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: RequestAgentCreationResult = serde_json::from_slice(&body).unwrap();
        let project_agent_id = format!("agent_{}", created.project.id);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/agent-creation-requests/lease")
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "runnerId": "runner-oslo-1",
                            "leaseToken": "lease-token-1",
                            "leaseSeconds": 300,
                            "now": "2026-05-25T13:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let lease: Option<AgentCreationLease> = serde_json::from_slice(&body).unwrap();
        let lease = lease.unwrap();

        let runtime_token = "runtime-token-1";
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/core/v1/agent-creation-requests/{}/runtime",
                        lease.request.id
                    ))
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "runnerId": "runner-oslo-1",
                            "leaseToken": "lease-token-1",
                            "sourceHostId": "oslo-host-1",
                            "sourceMachineId": "oslo-agent-001",
                            "runtimeArtifactId": "artifact-v1",
                            "stateSchemaVersion": "state-v1",
                            "runtimeRelayTokenHash": crate::runtime_relay_token_hash(runtime_token).unwrap(),
                            "displayName": "Chat Agent",
                            "runtimeHost": "oslo-host-1",
                            "runtimeStatus": "unknown",
                            "hermesAvailable": true,
                            "publishedAppUrls": [],
                            "now": "2026-05-25T13:00:30Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/finite/v1/chat/inbox?projectAgentId={project_agent_id}&after=0&limit=10"
                    ))
                    .header("authorization", "Bearer runtime-token-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let inbox: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(inbox["machineId"], "oslo-agent-001");
        assert_eq!(inbox["events"].as_array().unwrap().len(), 0);

        let snapshot = serde_json::json!({
            "users": [],
            "machines": [{ "id": "oslo-agent-001" }],
            "project_agents": [{ "id": project_agent_id }],
            "sites": [],
            "skills": [],
            "capabilities": []
        });
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/finite/v1/chat/snapshot")
                    .header("authorization", "Bearer runtime-token-1")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "snapshot": snapshot }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/finite/v1/chat/log/messages")
                    .header("authorization", "Bearer runtime-token-1")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "projectAgentId": project_agent_id,
                            "threads": [{
                                "id": "topic-1",
                                "project_agent_id": project_agent_id,
                                "created_by": "runtime",
                                "title": "Smoke",
                                "created_at": "2026-05-25T13:00:00Z",
                                "last_activity_at": "2026-05-25T13:00:00Z",
                                "message_count": 0
                            }],
                            "messages": []
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/finite/v1/machines/oslo-agent-001/chat/snapshot")
                    .header("authorization", "Bearer core-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn core_api_rejects_spoofed_legacy_identity_headers() {
        let app = router(CoreStore::memory(), test_auth());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_test")
                    .header(WORKOS_EMAIL_HEADER, "test@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn core_api_rejects_mismatched_identity_headers_even_with_valid_jwt() {
        let app = router(CoreStore::memory(), test_auth());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject("user_real", "real@finite.vip", true, None,)
                        ),
                    )
                    .header(WORKOS_USER_ID_HEADER, "user_spoofed")
                    .header(WORKOS_EMAIL_HEADER, "spoofed@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn runner_keyring_enforces_worker_class_source_and_revocation_bindings() {
        let auth = core_auth_with_runner_credentials(
            "service-token",
            vec![
                runner_credential_config(
                    "kata-current",
                    "kata-current-token",
                    "kata-worker-1",
                    &[RunnerClass::Kata],
                    "kata-host-1",
                    false,
                ),
                runner_credential_config(
                    "kata-next",
                    "kata-next-token",
                    "kata-worker-1",
                    &[RunnerClass::Kata],
                    "kata-host-1",
                    false,
                ),
                runner_credential_config(
                    "kata-revoked",
                    "kata-revoked-token",
                    "kata-worker-1",
                    &[RunnerClass::Kata],
                    "kata-host-1",
                    true,
                ),
                runner_credential_config(
                    "phala-current",
                    "phala-current-token",
                    "phala-worker-1",
                    &[RunnerClass::Phala],
                    "phala-host-1",
                    false,
                ),
            ],
            "usage-token",
        );
        let app = router(CoreStore::memory(), auth);

        for token in ["kata-current-token", "kata-next-token"] {
            let headers = vec![("authorization".to_string(), format!("Bearer {token}"))];
            let (status, body) = send_json(
                &app,
                "POST",
                "/api/core/v1/agent-creation-requests/lease",
                &headers,
                Some(serde_json::json!({
                    "runnerId": "kata-worker-1",
                    "leaseToken": "lease-token",
                    "runnerCapacity": runner_capacity_json(RunnerClass::Kata)
                })),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert!(body.is_null(), "empty queue must return no lease");
        }

        let kata = vec![(
            "authorization".to_string(),
            "Bearer kata-current-token".to_string(),
        )];
        for body in [
            serde_json::json!({
                "runnerId": "phala-worker-1",
                "leaseToken": "lease-token",
                "runnerCapacity": { "runnerClasses": ["kata"] }
            }),
            serde_json::json!({
                "runnerId": "kata-worker-1",
                "leaseToken": "lease-token",
                "runnerCapacity": { "runnerClasses": [] }
            }),
            serde_json::json!({
                "runnerId": "kata-worker-1",
                "leaseToken": "lease-token",
                "runnerCapacity": runner_capacity_json(RunnerClass::Phala)
            }),
        ] {
            let (status, _) = send_json(
                &app,
                "POST",
                "/api/core/v1/agent-creation-requests/lease",
                &kata,
                Some(body),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN);
        }

        let phala = vec![(
            "authorization".to_string(),
            "Bearer phala-current-token".to_string(),
        )];
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/agent-creation-requests/lease",
            &phala,
            Some(serde_json::json!({
                "runnerId": "phala-worker-1",
                "leaseToken": "lease-token",
                "runnerCapacity": { "runnerClasses": ["kata"] }
            })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, body) = send_json(
            &app,
            "POST",
            "/api/core/v1/runtime-control-requests/lease",
            &phala,
            Some(serde_json::json!({
                "runnerId": "phala-worker-1",
                "leaseToken": "lease-token",
                "sourceHostId": "phala-host-1",
                "runnerCapacity": runner_capacity_json(RunnerClass::Phala)
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.is_null());

        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/runtime-control-requests/lease",
            &phala,
            Some(serde_json::json!({
                "runnerId": "phala-worker-1",
                "leaseToken": "lease-token",
                "sourceHostId": "kata-host-1",
                "runnerCapacity": { "runnerClasses": ["phala"] }
            })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let revoked = vec![(
            "authorization".to_string(),
            "Bearer kata-revoked-token".to_string(),
        )];
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/agent-creation-requests/lease",
            &revoked,
            Some(serde_json::json!({
                "runnerId": "kata-worker-1",
                "leaseToken": "lease-token",
                "runnerCapacity": { "runnerClasses": ["kata"] }
            })),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn route_scoped_credentials_cannot_cross_user_admin_or_runner_boundaries() {
        let app = router(CoreStore::memory(), scoped_test_auth());
        let runner = vec![("authorization".to_string(), boundary_runner_authorization())];
        let service = vec![("authorization".to_string(), "Bearer core-token".to_string())];
        let usage = vec![("authorization".to_string(), usage_authorization())];

        for uri in ["/api/core/v1/me", "/api/core/v1/admin/runtimes"] {
            for (credential, headers) in [
                ("service", &service),
                ("Runner", &runner),
                ("usage", &usage),
            ] {
                let (status, _) = send_json(&app, "GET", uri, headers, None).await;
                assert_eq!(
                    status,
                    StatusCode::UNAUTHORIZED,
                    "{credential} credential entered {uri}"
                );
            }
        }

        let runner_routes = [
            (
                "/api/core/v1/agent-creation-requests/lease",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary",
                    "leaseSeconds": 60,
                    "runnerCapacity": runner_capacity_json(RunnerClass::Kata)
                }),
            ),
            (
                "/api/core/v1/runtime-control-requests/lease",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary",
                    "leaseSeconds": 60,
                    "sourceHostId": "source-auth-boundary",
                    "runnerCapacity": runner_capacity_json(RunnerClass::Kata)
                }),
            ),
            (
                "/api/core/v1/runtime-control-requests/missing/complete",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary"
                }),
            ),
            (
                "/api/core/v1/runtime-control-requests/missing/fail",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary",
                    "failureMessage": "boundary test"
                }),
            ),
            (
                "/api/core/v1/agent-creation-requests/missing/complete",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary",
                    "sourceHostId": "source-auth-boundary",
                    "sourceMachineId": "machine-auth-boundary",
                    "publishedAppUrls": []
                }),
            ),
            (
                "/api/core/v1/agent-creation-requests/missing/runtime",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary",
                    "sourceHostId": "source-auth-boundary",
                    "sourceMachineId": "machine-auth-boundary",
                    "runtimeRelayTokenHash": "hash-auth-boundary",
                    "publishedAppUrls": []
                }),
            ),
            (
                "/api/core/v1/agent-creation-requests/missing/finite-private-key",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary"
                }),
            ),
            (
                "/api/core/v1/agent-creation-requests/missing/fail",
                serde_json::json!({
                    "runnerId": "runner-auth-boundary",
                    "leaseToken": "lease-auth-boundary",
                    "failureMessage": "boundary test"
                }),
            ),
        ];
        for (uri, body) in &runner_routes {
            for (credential, headers) in [("service", &service), ("usage", &usage)] {
                let (status, _) = send_json(&app, "POST", uri, headers, Some(body.clone())).await;
                assert_eq!(
                    status,
                    StatusCode::UNAUTHORIZED,
                    "{credential} credential entered {uri}"
                );
            }
        }

        for headers in [&runner, &usage] {
            let (status, _) = send_json(
                &app,
                "GET",
                "/api/core/v1/source-host-relays/missing",
                headers,
                None,
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let (status, _) = send_json(
            &app,
            "GET",
            "/api/core/v1/source-host-relays/missing",
            &service,
            None,
        )
        .await;
        assert_ne!(status, StatusCode::UNAUTHORIZED);

        let (status, _) = send_json(
            &app,
            "PUT",
            "/api/core/v1/runtime-artifacts/artifact-auth-boundary",
            &service,
            Some(serde_json::json!({
                "kind": "oci_image",
                "reference": "ghcr.io/finitecomputer/agent-runtime@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "versionLabel": "auth-boundary",
                "stateSchemaVersion": "state-v1",
                "baseImage": "python:3.13-trixie",
                "promoted": true
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        for headers in [&service, &usage] {
            let (status, _) = send_json(
                &app,
                "GET",
                "/api/core/v1/runtime-artifacts/artifact-auth-boundary",
                headers,
                None,
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let (status, artifact) = send_json(
            &app,
            "GET",
            "/api/core/v1/runtime-artifacts/artifact-auth-boundary",
            &runner,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(artifact["id"], "artifact-auth-boundary");

        for headers in [&runner, &usage] {
            let (status, _) = send_json(
                &app,
                "PUT",
                "/api/core/v1/runtime-artifacts/forbidden-artifact",
                headers,
                Some(serde_json::json!({
                    "kind": "oci_image",
                    "reference": "ghcr.io/finitecomputer/agent-runtime@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "versionLabel": "forbidden",
                    "stateSchemaVersion": "state-v1",
                    "baseImage": "python:3.13-trixie",
                    "promoted": true
                })),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }

        for headers in [&service, &usage] {
            let (status, _) = send_json(
                &app,
                "GET",
                "/api/finite/v1/machines/missing/heartbeat",
                headers,
                None,
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let (status, _) = send_json(
            &app,
            "GET",
            "/api/finite/v1/machines/missing/heartbeat",
            &runner,
            None,
        )
        .await;
        assert_ne!(status, StatusCode::UNAUTHORIZED);

        for headers in [&service, &runner] {
            let (status, _) = send_json(
                &app,
                "GET",
                "/internal/finite-private/v1/health",
                headers,
                None,
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let (status, _) = send_json(
            &app,
            "GET",
            "/internal/finite-private/v1/health",
            &usage,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send_json(
            &app,
            "POST",
            "/api/core/v1/agent-creation-requests/lease",
            &runner,
            Some(runner_routes[0].1.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.is_null(), "empty Runner queue should return null");
    }

    fn admin_router(store: CoreStore) -> Router {
        router_with_runtime_upgrades(store, test_auth(), default_relay_state_dir(), true)
    }

    fn identity_headers(email: &str, verified: &str) -> Vec<(String, String)> {
        workos_headers(email, matches!(verified, "1" | "true" | "yes"), None)
    }

    fn operator_identity_headers(email: &str) -> Vec<(String, String)> {
        workos_headers(email, true, Some(OPERATOR_ORG_ID))
    }

    fn workos_headers(
        email: &str,
        verified: bool,
        organization_id: Option<&str>,
    ) -> Vec<(String, String)> {
        vec![(
            "authorization".to_string(),
            format!("Bearer {}", access_token(email, verified, organization_id)),
        )]
    }

    async fn send_json(
        app: &Router,
        method: &str,
        uri: &str,
        headers: &[(String, String)],
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        for (name, value) in headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let request = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = if bytes.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| serde_json::json!({ "raw": String::from_utf8_lossy(&bytes) }))
        };
        (status, json)
    }

    #[tokio::test]
    async fn runtime_upgrade_first_use_gate_is_fail_closed_without_blocking_restart() {
        let app = router_with_runtime_upgrades(
            CoreStore::memory(),
            test_auth(),
            default_relay_state_dir(),
            false,
        );
        let admin = operator_identity_headers("admin@finite.vip");
        let (upgrade_status, upgrade_body) = send_json(
            &app,
            "POST",
            "/api/core/v1/admin/projects/missing/runtime/upgrade",
            &admin,
            Some(serde_json::json!({ "targetRuntimeArtifactId": "artifact-v2" })),
        )
        .await;
        assert_eq!(upgrade_status, StatusCode::CONFLICT);
        assert!(
            upgrade_body["error"]
                .as_str()
                .unwrap_or_default()
                .contains("not enabled")
        );

        let (restart_status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/admin/projects/missing/runtime/restart",
            &admin,
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(restart_status, StatusCode::NOT_FOUND);
    }

    /// Provision one hosted agent through the same HTTP flow the dashboard and
    /// runner use, returning (project_id, agent_runtime_id).
    async fn provision_hosted_agent(app: &Router, launch_code: &str) -> (String, String) {
        let service = [("authorization".to_string(), "Bearer core-token".to_string())];
        let (status, _) = send_json(
            app,
            "PUT",
            "/api/core/v1/runtime-artifacts/artifact-v1",
            &service,
            Some(serde_json::json!({
                "kind": "oci_image",
                "reference": format!(
                    "ghcr.io/finitecomputer/agent-runtime:v1@sha256:{}",
                    "a".repeat(64)
                ),
                "versionLabel": "v1",
                "stateSchemaVersion": "state-v1",
                "baseImage": "python:3.11-trixie",
                "promoted": true,
                "now": "2026-05-25T12:00:00Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let owner = identity_headers("owner@finite.vip", "true");
        let (status, _) = send_json(
            app,
            "POST",
            "/api/core/v1/me/agent-creation-requests",
            &owner,
            Some(serde_json::json!({
                "displayName": "Oslo Agent",
                "launchCode": launch_code,
                "idempotencyKey": "browser-submit-1"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, lease) = send_json(
            app,
            "POST",
            "/api/core/v1/agent-creation-requests/lease",
            &service,
            Some(serde_json::json!({
                "runnerId": "runner-oslo-1",
                "leaseToken": "lease-token-1",
                "leaseSeconds": 300,
                "now": "2026-05-25T13:00:00Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let request_id = lease["request"]["id"].as_str().unwrap().to_string();

        let (status, completed) = send_json(
            app,
            "POST",
            &format!("/api/core/v1/agent-creation-requests/{request_id}/complete"),
            &service,
            Some(serde_json::json!({
                "runnerId": "runner-oslo-1",
                "leaseToken": "lease-token-1",
                "sourceHostId": "oslo-host-1",
                "sourceMachineId": "oslo-agent-001",
                "runtimeArtifactId": "artifact-v1",
                "runtimeHost": "oslo-host-1",
                "runtimeStatus": "online",
                "activeInferenceProfile": "finite-private",
                "hermesAvailable": true,
                "publishedAppUrls": [],
                "runtimeCapabilities": runtime_capabilities_json(true),
                "now": "2026-05-25T13:01:00Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        (
            completed["project"]["id"].as_str().unwrap().to_string(),
            completed["request"]["agent_runtime_id"]
                .as_str()
                .unwrap()
                .to_string(),
        )
    }

    #[tokio::test]
    async fn launch_code_admin_api_derives_operator_and_returns_plaintext_once() {
        let app = admin_router(CoreStore::memory());
        let operator_subject = "workos_operator_subject";
        let operator = vec![(
            "authorization".to_string(),
            format!(
                "Bearer {}",
                access_token_with_subject(
                    operator_subject,
                    "admin@finite.vip",
                    true,
                    Some(OPERATOR_ORG_ID),
                )
            ),
        )];
        let (status, issued) = send_json(
            &app,
            "POST",
            "/api/core/v1/admin/launch-code-batches",
            &operator,
            Some(serde_json::json!({
                "name": "Twelve-person training",
                "codeCount": 12,
                "expiresInHours": 24
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(issued["batch"]["code_count"], 12);
        assert_eq!(
            issued["batch"]["created_by_workos_user_id"],
            operator_subject
        );
        let codes = issued["codes"].as_array().unwrap();
        assert_eq!(codes.len(), 12);
        let plaintext = codes[0]["code"].as_str().unwrap().to_string();
        let batch_id = issued["batch"]["id"].as_str().unwrap().to_string();

        let (status, listed) = send_json(
            &app,
            "GET",
            "/api/core/v1/admin/launch-code-batches",
            &operator,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert!(!listed.to_string().contains(&plaintext));
        assert!(listed[0]["codes"][0].get("code").is_none());

        let (status, revoked) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/launch-code-batches/{batch_id}/revoke"),
            &operator,
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            revoked["batch"]["revoked_by_workos_user_id"],
            operator_subject
        );
        assert!(!revoked.to_string().contains(&plaintext));

        let ordinary_user = identity_headers("member@finite.vip", "true");
        for (method, uri, body) in [
            (
                "GET",
                "/api/core/v1/admin/launch-code-batches".to_string(),
                None,
            ),
            (
                "POST",
                "/api/core/v1/admin/launch-code-batches".to_string(),
                Some(serde_json::json!({
                    "name": "Forbidden",
                    "codeCount": 1,
                    "expiresInHours": 24
                })),
            ),
        ] {
            let (status, _) = send_json(&app, method, &uri, &ordinary_user, body).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
        }
    }

    #[tokio::test]
    async fn launch_code_plaintext_issuance_response_is_not_cacheable() {
        let app = admin_router(CoreStore::memory());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/admin/launch-code-batches")
                    .header(
                        "authorization",
                        format!(
                            "Bearer {}",
                            access_token_with_subject(
                                "workos_operator_no_store",
                                "admin@finite.vip",
                                true,
                                Some(OPERATOR_ORG_ID),
                            )
                        ),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "name": "One-time response",
                            "codeCount": 1,
                            "expiresInHours": 24
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("cache-control")
                .and_then(|value| value.to_str().ok()),
            Some("no-store, private")
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let issued: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(issued["codes"][0]["code"].as_str().is_some());
    }

    #[tokio::test]
    async fn core_api_admin_endpoints_require_configured_operator_organization() {
        let app = admin_router(CoreStore::memory());

        // Missing WorkOS access token entirely.
        let (status, _) = send_json(&app, "GET", "/api/core/v1/admin/runtimes", &[], None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // A service credential cannot enter the operator boundary.
        let service = [("authorization".to_string(), "Bearer core-token".to_string())];
        let (status, _) =
            send_json(&app, "GET", "/api/core/v1/admin/runtimes", &service, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // A valid user without an organization is not an operator.
        let (status, body) = send_json(
            &app,
            "GET",
            "/api/core/v1/admin/runtimes",
            &identity_headers("stranger@finite.vip", "true"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body["error"].as_str().unwrap().contains("admin access"));

        // Operator organization membership cannot bypass email verification.
        let (status, _) = send_json(
            &app,
            "GET",
            "/api/core/v1/admin/runtimes",
            &workos_headers("admin@finite.vip", false, Some(OPERATOR_ORG_ID)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // The configured operator organization is accepted.
        let (status, body) = send_json(
            &app,
            "GET",
            "/api/core/v1/admin/runtimes",
            &operator_identity_headers("admin@finite.vip"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.as_array().unwrap().is_empty());

        // Account Auth's operator organization is never reused as a Core
        // Customer Organization, even when an operator uses a user route.
        let operator = operator_identity_headers("admin@finite.vip");
        let (status, _) = send_json(&app, "GET", "/api/core/v1/me", &operator, None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, billing) =
            send_json(&app, "GET", "/api/core/v1/me/billing", &operator, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_ne!(billing["customer_org"]["id"], OPERATOR_ORG_ID);

        // Every mutating admin endpoint rejects a valid user without the
        // configured operator organization.
        for (method, uri, body) in [
            (
                "GET",
                "/api/core/v1/finite-private/admin-audit-events".to_string(),
                serde_json::json!({}),
            ),
            (
                "GET",
                "/api/core/v1/finite-private/admin-state".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/finite-private/grants".to_string(),
                serde_json::json!({ "verifiedEmail": "friend@finite.vip" }),
            ),
            (
                "POST",
                "/api/core/v1/finite-private/grants/grant_x/api-keys".to_string(),
                serde_json::json!({ "rawKey": "test-key-never-stored" }),
            ),
            (
                "POST",
                "/api/core/v1/finite-private/grants/grant_x/revoke".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/finite-private/grants/grant_x/reset".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/finite-private/api-keys/key_x/revoke".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/finite-private/api-keys/key_x/rotate".to_string(),
                serde_json::json!({ "rawKey": "replacement-test-key-never-stored" }),
            ),
            (
                "GET",
                "/api/core/v1/admin/launch-code-batches".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/admin/launch-code-batches".to_string(),
                serde_json::json!({
                    "name": "Forbidden",
                    "codeCount": 1,
                    "expiresInHours": 24
                }),
            ),
            (
                "POST",
                "/api/core/v1/admin/launch-code-batches/batch_x/revoke".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/admin/projects/project_x/runtime/restart".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/admin/projects/project_x/runtime/recover-known-good-chat".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/admin/projects/project_x/runtime/upgrade".to_string(),
                serde_json::json!({ "targetRuntimeArtifactId": "artifact-v2" }),
            ),
            (
                "POST",
                "/api/core/v1/admin/finite-private/friend-keys".to_string(),
                serde_json::json!({ "email": "friend@finite.vip" }),
            ),
            (
                "POST",
                "/api/core/v1/admin/finite-private/keys/key_x/rotate".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/admin/finite-private/keys/key_x/revoke".to_string(),
                serde_json::json!({}),
            ),
            (
                "POST",
                "/api/core/v1/admin/finite-private/grants/grant_x/window-reset".to_string(),
                serde_json::json!({}),
            ),
        ] {
            for headers in [
                identity_headers("stranger@finite.vip", "true"),
                workos_headers("member@finite.vip", true, Some("workos_org_not_operator")),
            ] {
                let (status, _) = send_json(&app, method, &uri, &headers, Some(body.clone())).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{uri} must be admin-gated");
            }
        }

        // A different WorkOS organization fails closed as well.
        let (status, _) = send_json(
            &app,
            "GET",
            "/api/core/v1/admin/runtimes",
            &workos_headers("admin@finite.vip", true, Some("workos_org_customer")),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn core_api_admin_runtimes_and_runtime_control_feed_the_runner_queue() {
        let store = CoreStore::memory();
        let launch_code = issue_test_launch_code(&store).await;
        let app = admin_router(store);
        let (project_id, runtime_id) = provision_hosted_agent(&app, &launch_code).await;
        let admin = operator_identity_headers("admin@finite.vip");
        let service = [("authorization".to_string(), "Bearer core-token".to_string())];

        let (status, runtimes) =
            send_json(&app, "GET", "/api/core/v1/admin/runtimes", &admin, None).await;
        assert_eq!(status, StatusCode::OK);
        let runtimes = runtimes.as_array().unwrap().clone();
        assert_eq!(runtimes.len(), 1);
        let overview = &runtimes[0];
        assert_eq!(overview["project_id"], project_id.as_str());
        assert_eq!(overview["agent_runtime_id"], runtime_id.as_str());
        assert_eq!(overview["owner_email"], "owner@finite.vip");
        assert_eq!(overview["source_host_id"], "oslo-host-1");
        assert_eq!(overview["runtime_artifact_version_label"], "v1");
        assert_eq!(overview["runtime_status"], "online");
        assert_eq!(overview["runtime_capabilities"]["restart"], true);
        assert_eq!(
            overview["runtime_capabilities"]["recover_known_good_chat"],
            false
        );
        assert_eq!(overview["runtime_capabilities"]["runtime_upgrade"], true);
        assert_eq!(overview["runtime_capabilities"]["stop"], true);
        assert_eq!(
            overview["runtime_capabilities"]["runtime_retirement"],
            false
        );

        // Admin restart succeeds even though the admin does not own the project.
        let (status, restart) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/projects/{project_id}/runtime/restart"),
            &admin,
            Some(serde_json::json!({ "now": "2026-05-25T13:03:00Z" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(restart["kind"], "restart");
        assert_eq!(restart["status"], "requested");
        assert_eq!(restart["agent_runtime_id"], runtime_id.as_str());
        assert!(restart.get("lease_token").is_none());
        let restart_id = restart["id"].as_str().unwrap().to_string();

        // The runner consumes the admin-created request through the same lease
        // endpoint and shape as owner-created requests.
        let (status, lease) = send_json(
            &app,
            "POST",
            "/api/core/v1/runtime-control-requests/lease",
            &service,
            Some(serde_json::json!({
                "runnerId": "runner-oslo-1",
                "leaseToken": "restart-lease-1",
                "leaseSeconds": 60,
                "sourceHostId": "oslo-host-1",
                "now": "2026-05-25T13:04:00Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(lease["request"]["id"], restart_id.as_str());
        assert_eq!(lease["request"]["status"], "running");
        assert_eq!(lease["runtime"]["source_machine_id"], "oslo-agent-001");

        let (status, completed) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/runtime-control-requests/{restart_id}/complete"),
            &service,
            Some(serde_json::json!({
                "runnerId": "runner-oslo-1",
                "leaseToken": "restart-lease-1",
                "now": "2026-05-25T13:05:00Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(completed["status"], "succeeded");

        let target_reference = format!(
            "ghcr.io/finitecomputer/agent-runtime:v2@sha256:{}",
            "b".repeat(64)
        );
        let (status, _) = send_json(
            &app,
            "PUT",
            "/api/core/v1/runtime-artifacts/artifact-v2",
            &service,
            Some(serde_json::json!({
                "kind": "oci_image",
                "reference": target_reference,
                "versionLabel": "v2",
                "stateSchemaVersion": "state-v1",
                "promoted": true,
                "now": "2026-05-25T13:05:10Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, upgrade) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/projects/{project_id}/runtime/upgrade"),
            &admin,
            Some(serde_json::json!({
                "targetRuntimeArtifactId": "artifact-v2",
                "now": "2026-05-25T13:05:20Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(upgrade["kind"], "upgrade");
        assert_eq!(upgrade["target_runtime_artifact_id"], "artifact-v2");
        let upgrade_id = upgrade["id"].as_str().unwrap();
        let (status, lease) = send_json(
            &app,
            "POST",
            "/api/core/v1/runtime-control-requests/lease",
            &service,
            Some(serde_json::json!({
                "runnerId": "runner-oslo-1",
                "leaseToken": "upgrade-lease-1",
                "leaseSeconds": 60,
                "sourceHostId": "oslo-host-1",
                "runnerCapacity": { "runnerClasses": ["kata"] },
                "now": "2026-05-25T13:05:30Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(lease["request"]["id"], upgrade_id);
        assert_eq!(lease["target_runtime_artifact"]["id"], "artifact-v2");
        let (status, upgraded) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/runtime-control-requests/{upgrade_id}/complete"),
            &service,
            Some(serde_json::json!({
                "runnerId": "runner-oslo-1",
                "leaseToken": "upgrade-lease-1",
                "runtimeArtifactId": "artifact-v2",
                "stateSchemaVersion": "state-v1",
                "runtimeHost": "http://127.0.0.1:41002",
                "publishedAppUrls": ["http://127.0.0.1:41002/contact"],
                "now": "2026-05-25T13:05:40Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(upgraded["status"], "succeeded");

        // Recovery remains disabled until it is more than a restart alias.
        let (status, _) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/projects/{project_id}/runtime/recover-known-good-chat"),
            &admin,
            Some(serde_json::json!({ "now": "2026-05-25T13:06:00Z" })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        // Both admin actions are audited with the admin's email as actor.
        let (status, events) = send_json(
            &app,
            "GET",
            "/api/core/v1/finite-private/admin-audit-events",
            &admin,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let events = events.as_array().unwrap().clone();
        let admin_actions = events
            .iter()
            .filter(|event| event["actor"] == "admin@finite.vip")
            .map(|event| event["action"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(admin_actions.contains(&"runtime.admin_restart".to_string()));
        assert!(admin_actions.contains(&"runtime.admin_upgrade".to_string()));
        assert!(!admin_actions.contains(&"runtime.admin_recover_known_good_chat".to_string()));
    }

    #[tokio::test]
    async fn core_api_admin_friend_key_lifecycle_returns_raw_key_exactly_once() {
        let app = admin_router(CoreStore::memory());
        let admin = operator_identity_headers("admin@finite.vip");

        let (status, issued) = send_json(
            &app,
            "POST",
            "/api/core/v1/admin/finite-private/friend-keys",
            &admin,
            Some(serde_json::json!({
                "email": "Friend@Finite.VIP",
                "now": "2026-05-25T12:00:00Z"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let raw_key = issued["raw_api_key"].as_str().unwrap().to_string();
        assert!(raw_key.starts_with("fpk_live_"));
        assert_eq!(issued["grant"]["status"], "active");
        assert_eq!(issued["api_key"]["status"], "active");
        assert_ne!(issued["api_key"]["key_hash"], raw_key.as_str());
        assert!(
            issued["raw_api_key_note"]
                .as_str()
                .unwrap()
                .contains("shown once")
        );
        let key_id = issued["api_key"]["id"].as_str().unwrap().to_string();
        let grant_id = issued["grant"]["id"].as_str().unwrap().to_string();

        // Core never stores or re-serves the raw key: the whole admin state
        // must not contain it anywhere.
        let (status, admin_state) = send_json(
            &app,
            "GET",
            "/api/core/v1/finite-private/admin-state",
            &admin,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!admin_state.to_string().contains(&raw_key));

        // Rotate returns a brand-new one-time raw key and revokes the old key.
        let (status, rotated) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/finite-private/keys/{key_id}/rotate"),
            &admin,
            Some(serde_json::json!({ "now": "2026-05-25T13:00:00Z" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let rotated_raw = rotated["raw_api_key"].as_str().unwrap().to_string();
        assert!(rotated_raw.starts_with("fpk_live_"));
        assert_ne!(rotated_raw, raw_key);
        assert!(rotated.get("grant").is_none());
        let rotated_key_id = rotated["api_key"]["id"].as_str().unwrap().to_string();
        assert_ne!(rotated_key_id, key_id);

        let (_, admin_state) = send_json(
            &app,
            "GET",
            "/api/core/v1/finite-private/admin-state",
            &admin,
            None,
        )
        .await;
        let keys = admin_state["apiKeys"].as_array().unwrap().clone();
        let old_key = keys
            .iter()
            .find(|key| key["id"] == key_id.as_str())
            .unwrap();
        assert_eq!(old_key["status"], "revoked");
        let new_key = keys
            .iter()
            .find(|key| key["id"] == rotated_key_id.as_str())
            .unwrap();
        assert_eq!(new_key["status"], "active");
        assert!(!admin_state.to_string().contains(&rotated_raw));

        // Revoke the rotated key.
        let (status, revoked) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/finite-private/keys/{rotated_key_id}/revoke"),
            &admin,
            Some(serde_json::json!({ "now": "2026-05-25T14:00:00Z" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(revoked["status"], "revoked");

        // Burst window reset mirrors the CLI window-reset semantics.
        let (status, reset) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/finite-private/grants/{grant_id}/window-reset"),
            &admin,
            Some(serde_json::json!({ "now": "2026-05-25T15:00:00Z" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reset["current_window_used_units"], 0);
        assert!(reset["current_window_started_at"].is_null());

        // Unknown ids surface as 404s, and every admin action was audited
        // with the admin as actor.
        let (status, _) = send_json(
            &app,
            "POST",
            "/api/core/v1/admin/finite-private/grants/missing/window-reset",
            &admin,
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (_, events) = send_json(
            &app,
            "GET",
            "/api/core/v1/finite-private/admin-audit-events",
            &admin,
            None,
        )
        .await;
        let admin_actions = events
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["actor"] == "admin@finite.vip")
            .map(|event| event["action"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        for expected in [
            "finite_private.friend_key.admin_issue",
            "finite_private.api_key.admin_rotate",
            "finite_private.api_key.admin_revoke",
            "finite_private.grant.admin_window_reset",
        ] {
            assert!(
                admin_actions.contains(&expected.to_string()),
                "missing audit action {expected}"
            );
        }
    }
}
