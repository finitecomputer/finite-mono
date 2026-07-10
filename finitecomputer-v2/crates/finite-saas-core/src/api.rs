use crate::store::{CoreStore, VisibleProject};
use crate::{
    AdminIssueFinitePrivateFriendKeyInput, AdminIssuedFinitePrivateKey,
    AdminResetFinitePrivateUsageWindowInput, AdminRevokeFinitePrivateApiKeyInput,
    AdminRotateFinitePrivateApiKeyInput, AdminRuntimeControlInput, AdminRuntimeOverview,
    AgentCreationConfiguration, AgentCreationLease, AgentCreationRequest, BillingOverview,
    BillingSubscriptionStatus, CancelAgentCreationRequestInput, ClaimProjectImportsInput,
    ClaimProjectImportsResult, CompleteAgentCreationRequestInput,
    CompleteRuntimeControlRequestInput, CoreError, CustomerBillingAccount,
    ExistingHostProjectImport, FailAgentCreationRequestInput, FailRuntimeControlRequestInput,
    FinitePrivateAdminAuditEvent, FinitePrivateAdminState, FinitePrivateApiKey, FinitePrivateGrant,
    FinitePrivateSettlementKind, FinitePrivateUsageDecision, IssueFinitePrivateApiKeyInput,
    LeaseAgentCreationRequestInput, LeaseRuntimeControlRequestInput, LinkStripeCustomerInput,
    LinkVerifiedUserInput, ProjectImportCandidate, ProvisionFinitePrivateRuntimeKeyInput,
    ProvisionFinitePrivateRuntimeKeyResult, ReconcileExistingHostImportsOptions,
    ReconcileExistingHostImportsReport, RegisterAgentCreationRuntimeInput,
    RequestAgentCreationInput, RequestAgentCreationResult, RequestRuntimeRecoverKnownGoodChatInput,
    RequestRuntimeRestartInput, ReserveFinitePrivateUsageInput, ResetFinitePrivateUsageWindowInput,
    RevokeFinitePrivateApiKeyInput, RevokeFinitePrivateGrantInput, RotateFinitePrivateApiKeyInput,
    RunnerClass, RunnerLeaseCapacity, RuntimeArtifact, RuntimeArtifactKind, RuntimeSummaryStatus,
    SettleFinitePrivateReservationInput, SettleFinitePrivateReservationResult,
    SourceHostRelayEndpoint, SyncStripeSubscriptionInput, UpsertRuntimeArtifactInput,
    UpsertSourceHostRelayEndpointInput, normalize_owner_email,
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
use std::collections::{BTreeSet, HashMap};
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
    api_token: String,
    finite_private_usage_api_token: String,
    standard_stripe_price_id: Option<String>,
    /// Normalized emails allowed to call `/api/core/v1/admin/*`. Empty means
    /// no admins: every admin endpoint fails closed with 403.
    admin_emails: Arc<BTreeSet<String>>,
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
#[serde(rename_all = "camelCase")]
pub struct CreateAgentRequest {
    pub display_name: String,
    pub launch_code: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub runner_class: RunnerClass,
    #[serde(default)]
    pub profile_picture_url: Option<String>,
    pub now: Option<String>,
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
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteRuntimeControlRequest {
    pub runner_id: String,
    pub lease_token: String,
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
    pub claimable_candidates: Vec<ProjectImportCandidate>,
    pub projects: Vec<VisibleProject>,
    pub agent_creation_requests: Vec<AgentCreationRequestSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCreationRequestSummary {
    pub id: String,
    pub project_id: String,
    pub display_name: String,
    pub runner_class: RunnerClass,
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
            runner_class: request.runner_class,
            profile_picture_url: request.profile_picture_url,
            status: request.status,
            agent_runtime_id: request.agent_runtime_id,
            failure_message: request.failure_message,
            created_at: request.created_at,
            updated_at: request.updated_at,
        }
    }
}

pub fn router(store: CoreStore, api_token: impl Into<String>) -> Router {
    router_with_relay_state_dir(store, api_token, default_relay_state_dir())
}

pub fn router_with_relay_state_dir(
    store: CoreStore,
    api_token: impl Into<String>,
    relay_state_dir: impl Into<PathBuf>,
) -> Router {
    router_with_admin_emails(
        store,
        api_token,
        relay_state_dir,
        env::var("FC_CORE_ADMIN_EMAILS").unwrap_or_default(),
    )
}

pub fn router_with_admin_emails(
    store: CoreStore,
    api_token: impl Into<String>,
    relay_state_dir: impl Into<PathBuf>,
    admin_emails: impl AsRef<str>,
) -> Router {
    let api_token = api_token.into();
    let admin_emails = parse_admin_email_allowlist(admin_emails.as_ref());
    let finite_private_usage_api_token = env::var("FC_FINITE_PRIVATE_USAGE_API_TOKEN")
        .ok()
        .and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .unwrap_or_else(|| api_token.clone());
    let standard_stripe_price_id = optional_env_value("FC_CORE_STANDARD_STRIPE_PRICE_ID")
        .or_else(|| optional_env_value("STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID"));
    let state = CoreApiState {
        store,
        api_token,
        finite_private_usage_api_token,
        standard_stripe_price_id,
        admin_emails: Arc::new(admin_emails),
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
            "/api/core/v1/admin/projects/{project_id}/runtime/restart",
            post(admin_request_runtime_restart),
        )
        .route(
            "/api/core/v1/admin/projects/{project_id}/runtime/recover-known-good-chat",
            post(admin_request_runtime_recover_known_good_chat),
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
    require_service_auth(&state, &headers)?;
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
    require_service_auth(&state, &headers)?;
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
    require_service_auth(&state, &headers)?;
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
    require_service_auth(&state, &headers)?;
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
    require_service_auth(&state, &headers)?;
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
    require_service_auth(&state, &headers)?;
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
    require_service_auth(&state, &headers)?;
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
    require_service_auth(&state, &headers)?;
    Ok(Json(state.store.finite_private_admin_audit_events().await?))
}

async fn finite_private_admin_state(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<FinitePrivateAdminState>, ApiError> {
    require_service_auth(&state, &headers)?;
    Ok(Json(state.store.finite_private_admin_state().await?))
}

async fn admin_runtimes(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AdminRuntimeOverview>>, ApiError> {
    require_admin_identity(&state, &headers)?;
    Ok(Json(state.store.admin_runtime_overviews().await?))
}

async fn admin_request_runtime_restart(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<RuntimeControlRequestView>, ApiError> {
    let identity = require_admin_identity(&state, &headers)?;
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
    let identity = require_admin_identity(&state, &headers)?;
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

async fn admin_issue_finite_private_friend_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<AdminIssueFinitePrivateFriendKeyRequest>,
) -> Result<Json<AdminIssuedFinitePrivateKeyResponse>, ApiError> {
    let identity = require_admin_identity(&state, &headers)?;
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
    let identity = require_admin_identity(&state, &headers)?;
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
    let identity = require_admin_identity(&state, &headers)?;
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
    let identity = require_admin_identity(&state, &headers)?;
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
    let identity = require_verified_identity(&state, &headers)?;
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
        claimable_candidates,
        projects,
        agent_creation_requests: agent_creation_requests
            .into_iter()
            .map(AgentCreationRequestSummary::from)
            .collect(),
    }))
}

async fn billing_overview(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<BillingOverview>, ApiError> {
    let identity = require_verified_identity(&state, &headers)?;
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
    let identity = require_verified_identity(&state, &headers)?;
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
    let identity = require_verified_identity(&state, &headers)?;
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
    let identity = require_verified_identity(&state, &headers)?;
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
    let identity = require_verified_identity(&state, &headers)?;
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
                    now: input.now,
                },
                AgentCreationConfiguration {
                    runner_class: input.runner_class,
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
    let identity = require_verified_identity(&state, &headers)?;
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
    let identity = require_verified_identity(&state, &headers)?;
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
    let identity = require_verified_identity(&state, &headers)?;
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
    let identity = require_verified_identity(&state, &headers)?;
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

async fn lease_agent_creation_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<LeaseAgentCreationRequest>,
) -> Result<Json<Option<AgentCreationLease>>, ApiError> {
    require_service_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: input.runner_id,
                source_host_id: None,
                lease_token: input.lease_token,
                lease_seconds: input.lease_seconds,
                runner_capacity: input.runner_capacity,
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
    require_service_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                lease_seconds: input.lease_seconds,
                source_host_id: input.source_host_id,
                runner_capacity: input.runner_capacity,
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
    require_service_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
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
    require_service_auth(&state, &headers)?;
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
    require_service_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                source_host_id: input.source_host_id,
                source_machine_id: input.source_machine_id,
                runtime_artifact_id: input.runtime_artifact_id,
                state_schema_version: input.state_schema_version,
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
    require_service_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                source_host_id: input.source_host_id,
                source_machine_id: input.source_machine_id,
                runtime_artifact_id: input.runtime_artifact_id,
                state_schema_version: input.state_schema_version,
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
    require_service_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                source_host_id: input.source_host_id,
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
    require_service_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                failure_message: input.failure_message,
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
    require_service_auth(&state, &headers)?;
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
) -> Result<Json<Vec<VisibleProject>>, ApiError> {
    let identity = require_verified_identity(&state, &headers)?;
    Ok(Json(
        state
            .store
            .visible_projects_for_workos_user(&identity.workos_user_id)
            .await?,
    ))
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
}

fn require_verified_identity(
    state: &CoreApiState,
    headers: &HeaderMap,
) -> Result<VerifiedIdentity, ApiError> {
    require_service_auth(state, headers)?;
    let workos_user_id = required_header(headers, WORKOS_USER_ID_HEADER)?;
    let email = normalize_owner_email(Some(&required_header(headers, WORKOS_EMAIL_HEADER)?))
        .ok_or_else(|| ApiError::bad_request("verified email is required"))?;
    let email_verified = required_header(headers, WORKOS_EMAIL_VERIFIED_HEADER)?;
    if !matches!(email_verified.as_str(), "1" | "true" | "yes") {
        return Err(ApiError::forbidden("verified WorkOS email is required"));
    }

    Ok(VerifiedIdentity {
        email,
        workos_user_id,
    })
}

/// Core-side admin authorization. Requires a verified WorkOS identity whose
/// normalized email is in the `FC_CORE_ADMIN_EMAILS` allowlist. Core is the
/// enforcement point: the dashboard's own admin gate is UI-only.
fn require_admin_identity(
    state: &CoreApiState,
    headers: &HeaderMap,
) -> Result<VerifiedIdentity, ApiError> {
    let identity = require_verified_identity(state, headers)?;
    if !state.admin_emails.contains(&identity.email) {
        return Err(ApiError::forbidden(
            "admin access is required for this endpoint",
        ));
    }
    Ok(identity)
}

/// Parse the comma-separated `FC_CORE_ADMIN_EMAILS` allowlist into normalized
/// (trimmed, lowercased) emails. Empty or whitespace-only input yields an
/// empty allowlist, which means no admins.
fn parse_admin_email_allowlist(raw: &str) -> BTreeSet<String> {
    raw.split(',')
        .filter_map(|email| crate::normalize_owner_email(Some(email)))
        .collect()
}

fn require_service_auth(state: &CoreApiState, headers: &HeaderMap) -> Result<(), ApiError> {
    let expected = format!("Bearer {}", state.api_token);
    if header_value(headers, SERVICE_AUTH_HEADER)
        .as_deref()
        .is_some_and(|presented| constant_time_token_eq(presented, &expected))
    {
        return Ok(());
    }

    Err(ApiError::unauthorized("invalid service token"))
}

fn require_finite_private_usage_auth(
    state: &CoreApiState,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let expected = format!("Bearer {}", state.finite_private_usage_api_token);
    if header_value(headers, SERVICE_AUTH_HEADER)
        .as_deref()
        .is_some_and(|presented| constant_time_token_eq(presented, &expected))
    {
        return Ok(());
    }

    Err(ApiError::unauthorized(
        "invalid finite private usage service token",
    ))
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

fn required_header(headers: &HeaderMap, name: &str) -> Result<String, ApiError> {
    header_value(headers, name)
        .ok_or_else(|| ApiError::bad_request(format!("missing {name} header")))
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
            | CoreError::FinitePrivateReservationNotFound => Self::not_found(error.to_string()),
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
            | CoreError::RuntimeRestartUnsupported
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
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    const TOKEN: &str = "core-token";

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
        let app = router(CoreStore::memory(), TOKEN);
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
                published_app_urls: vec!["https://smoke.example.com".to_string()],
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
                    .header("authorization", "Bearer core-token")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_test")
                    .header(WORKOS_EMAIL_HEADER, "test@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
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
        let me: MeResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(me.claimable_candidates.len(), 1);
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
                    .header("authorization", "Bearer core-token")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_test")
                    .header(WORKOS_EMAIL_HEADER, "test@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
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
                    .header("authorization", "Bearer core-token")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_test")
                    .header(WORKOS_EMAIL_HEADER, "test@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let projects: Vec<VisibleProject> = serde_json::from_slice(&body).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(
            projects[0].runtime.as_ref().unwrap().source_host_id,
            "smoke"
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me")
                    .header("authorization", "Bearer core-token")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_prod_google")
                    .header(WORKOS_EMAIL_HEADER, "test@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
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
        assert_eq!(relinked_me.projects.len(), 1);
        assert_eq!(
            relinked_me.projects[0]
                .runtime
                .as_ref()
                .unwrap()
                .source_host_id,
            "smoke"
        );
        assert_eq!(relinked_me.workos_user_id, "user_workos_prod_google");
    }

    #[tokio::test]
    async fn core_api_stores_source_host_relay_endpoints_behind_service_auth() {
        let app = router(CoreStore::memory(), TOKEN);
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
        let app = router(CoreStore::memory(), TOKEN);

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
                    .header("authorization", "Bearer core-token")
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
                    .header("authorization", "Bearer core-token")
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
                    .header("authorization", "Bearer core-token")
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
                    .header("authorization", "Bearer core-token")
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
                    .header("authorization", "Bearer core-token")
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
                    .header("authorization", "Bearer core-token")
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
                    .header("authorization", "Bearer core-token")
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
                    .header("authorization", "Bearer core-token")
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
        let app = router(store, TOKEN);

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
        let app = router(CoreStore::memory(), TOKEN);
        let create = serde_json::to_vec(&CreateAgentRequest {
            display_name: "Oslo Agent".to_string(),
            launch_code: "off2026".to_string(),
            idempotency_key: "browser-submit-1".to_string(),
            runner_class: RunnerClass::Kata,
            profile_picture_url: Some("https://chat.finite.computer/v1/blobs/profile".to_string()),
            now: Some("2026-05-25T12:00:00Z".to_string()),
        })
        .unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/agent-creation-requests")
                    .header("authorization", "Bearer core-token")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_new")
                    .header(WORKOS_EMAIL_HEADER, "new@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
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
                    .header("authorization", "Bearer core-token")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_new")
                    .header(WORKOS_EMAIL_HEADER, "new@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
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
            me.agent_creation_requests[0].runner_class,
            RunnerClass::Kata
        );
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
                    .header("authorization", "Bearer core-token")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_new")
                    .header(WORKOS_EMAIL_HEADER, "new@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
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
            launch_code: "off2026".to_string(),
            idempotency_key: "browser-submit-2".to_string(),
            runner_class: RunnerClass::Phala,
            profile_picture_url: None,
            now: Some("2026-05-25T13:00:00Z".to_string()),
        })
        .unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/agent-creation-requests")
                    .header("authorization", "Bearer core-token")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_new")
                    .header(WORKOS_EMAIL_HEADER, "new@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
                    .header("content-type", "application/json")
                    .body(Body::from(second))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn core_api_lets_runner_lease_and_complete_agent_creation_request() {
        let app = router(CoreStore::memory(), TOKEN);
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
                            "reference": "finite-runtime-v1",
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
            launch_code: "off2026".to_string(),
            idempotency_key: "browser-submit-1".to_string(),
            runner_class: RunnerClass::Phala,
            profile_picture_url: None,
            now: Some("2026-05-25T12:00:00Z".to_string()),
        })
        .unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/agent-creation-requests")
                    .header("authorization", "Bearer core-token")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_new")
                    .header(WORKOS_EMAIL_HEADER, "new@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
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
                    .header("authorization", "Bearer core-token")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_new")
                    .header(WORKOS_EMAIL_HEADER, "new@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
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
                    .header("authorization", "Bearer core-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "runnerId": "runner-oslo-1",
                            "leaseToken": "lease-token-1",
                            "sourceHostId": "oslo-host-1",
                            "sourceMachineId": "oslo-agent-001",
                            "runtimeArtifactId": "artifact-v1",
                            "hostname": "oslo-agent-001.finite.computer",
                            "runtimeHost": "oslo-host-1",
                            "runtimeStatus": "online",
                            "activeInferenceProfile": "finite-private",
                            "hermesAvailable": true,
                            "publishedAppUrls": [],
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
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me/projects")
                    .header("authorization", "Bearer core-token")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_new")
                    .header(WORKOS_EMAIL_HEADER, "new@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let projects: Vec<VisibleProject> = serde_json::from_slice(&body).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(
            projects[0].runtime.as_ref().unwrap().source_machine_id,
            "oslo-agent-001"
        );
    }

    #[tokio::test]
    async fn core_api_skips_full_or_draining_runner_without_blocking_other_runner() {
        let app = router(CoreStore::memory(), TOKEN);
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
                            "reference": "finite-runtime-v1",
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
                    .header("authorization", "Bearer core-token")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_new")
                    .header(WORKOS_EMAIL_HEADER, "new@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "displayName": "Oslo Agent",
                            "launchCode": "off2026",
                            "idempotencyKey": "browser-submit-1",
                            "now": "2026-05-25T12:00:00Z"
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
                "availableMemoryBytes": 8589934592_u64
            }),
            serde_json::json!({
                "draining": false,
                "maxSandboxCount": 1,
                "activeSandboxCount": 1,
                "availableMemoryBytes": 1073741824_u64
            }),
        ] {
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
                    .header("authorization", "Bearer core-token")
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
                                "availableMemoryBytes": 8589934592_u64
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
        let app = router(CoreStore::memory(), TOKEN);
        let create = serde_json::to_vec(&CreateAgentRequest {
            display_name: "Oslo Agent".to_string(),
            launch_code: "off2026".to_string(),
            idempotency_key: "browser-submit-1".to_string(),
            runner_class: RunnerClass::Phala,
            profile_picture_url: None,
            now: Some("2026-05-25T12:00:00Z".to_string()),
        })
        .unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/core/v1/me/agent-creation-requests")
                    .header("authorization", "Bearer core-token")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_new")
                    .header(WORKOS_EMAIL_HEADER, "new@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
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
                    .header("authorization", "Bearer core-token")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_new")
                    .header(WORKOS_EMAIL_HEADER, "new@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let projects: Vec<VisibleProject> = serde_json::from_slice(&body).unwrap();
        assert!(projects.is_empty());
    }

    #[tokio::test]
    async fn core_api_serves_runtime_chat_relay_endpoints() {
        let relay_dir = tempfile::tempdir().unwrap();
        let app = router_with_relay_state_dir(CoreStore::memory(), TOKEN, relay_dir.path());

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
                            "reference": "finite-runtime-v1",
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
                    .header("authorization", "Bearer core-token")
                    .header(WORKOS_USER_ID_HEADER, "user_workos_chat")
                    .header(WORKOS_EMAIL_HEADER, "chat@finite.vip")
                    .header(WORKOS_EMAIL_VERIFIED_HEADER, "true")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "displayName": "Chat Agent",
                            "launchCode": "off2026",
                            "idempotencyKey": "chat-submit-1",
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
    async fn core_api_rejects_missing_service_auth() {
        let app = router(CoreStore::memory(), TOKEN);
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

    #[test]
    fn admin_email_allowlist_is_normalized_and_empty_means_no_admins() {
        let allowlist = parse_admin_email_allowlist(" Admin@Finite.VIP , second@finite.vip ,, ,\t");
        assert_eq!(allowlist.len(), 2);
        assert!(allowlist.contains("admin@finite.vip"));
        assert!(allowlist.contains("second@finite.vip"));
        assert!(!allowlist.contains("Admin@Finite.VIP"));

        assert!(parse_admin_email_allowlist("").is_empty());
        assert!(parse_admin_email_allowlist("  ,  ").is_empty());
    }

    fn admin_router(store: CoreStore) -> Router {
        router_with_admin_emails(
            store,
            TOKEN,
            default_relay_state_dir(),
            " Admin@Finite.VIP , second@finite.vip ",
        )
    }

    fn identity_headers(email: &str, verified: &str) -> Vec<(String, String)> {
        vec![
            ("authorization".to_string(), "Bearer core-token".to_string()),
            (WORKOS_USER_ID_HEADER.to_string(), format!("user_{email}")),
            (WORKOS_EMAIL_HEADER.to_string(), email.to_string()),
            (
                WORKOS_EMAIL_VERIFIED_HEADER.to_string(),
                verified.to_string(),
            ),
        ]
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
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, json)
    }

    /// Provision one hosted agent through the same HTTP flow the dashboard and
    /// runner use, returning (project_id, agent_runtime_id).
    async fn provision_hosted_agent(app: &Router) -> (String, String) {
        let service = [("authorization".to_string(), "Bearer core-token".to_string())];
        let (status, _) = send_json(
            app,
            "PUT",
            "/api/core/v1/runtime-artifacts/artifact-v1",
            &service,
            Some(serde_json::json!({
                "kind": "oci_image",
                "reference": "finite-runtime-v1",
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
                "launchCode": "off2026",
                "idempotencyKey": "browser-submit-1",
                "now": "2026-05-25T12:00:00Z"
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
    async fn core_api_admin_endpoints_require_core_side_admin_allowlist() {
        let app = admin_router(CoreStore::memory());

        // Missing service token entirely.
        let (status, _) = send_json(&app, "GET", "/api/core/v1/admin/runtimes", &[], None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // Service token but no identity headers.
        let service = [("authorization".to_string(), "Bearer core-token".to_string())];
        let (status, _) =
            send_json(&app, "GET", "/api/core/v1/admin/runtimes", &service, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Verified identity that is not allowlisted.
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

        // Allowlisted email but unverified.
        let (status, _) = send_json(
            &app,
            "GET",
            "/api/core/v1/admin/runtimes",
            &identity_headers("admin@finite.vip", "false"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Allowlisted and verified, case-insensitively.
        let (status, body) = send_json(
            &app,
            "GET",
            "/api/core/v1/admin/runtimes",
            &identity_headers("ADMIN@finite.vip", "true"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.as_array().unwrap().is_empty());

        // Every mutating admin endpoint rejects non-admins the same way.
        for (method, uri, body) in [
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
            let (status, _) = send_json(
                &app,
                method,
                &uri,
                &identity_headers("stranger@finite.vip", "true"),
                Some(body),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{uri} must be admin-gated");
        }

        // A router with no allowlist has no admins at all.
        let closed =
            router_with_admin_emails(CoreStore::memory(), TOKEN, default_relay_state_dir(), "");
        let (status, _) = send_json(
            &closed,
            "GET",
            "/api/core/v1/admin/runtimes",
            &identity_headers("admin@finite.vip", "true"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn core_api_admin_runtimes_and_runtime_control_feed_the_runner_queue() {
        let app = admin_router(CoreStore::memory());
        let (project_id, runtime_id) = provision_hosted_agent(&app).await;
        let admin = identity_headers("admin@finite.vip", "true");
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
        assert_eq!(overview["supports_runtime_control"], true);

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

        // Recover uses the same machinery.
        let (status, recover) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/admin/projects/{project_id}/runtime/recover-known-good-chat"),
            &admin,
            Some(serde_json::json!({ "now": "2026-05-25T13:06:00Z" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(recover["kind"], "recover_known_good_chat_runtime");

        // Both admin actions are audited with the admin's email as actor.
        let (status, events) = send_json(
            &app,
            "GET",
            "/api/core/v1/finite-private/admin-audit-events",
            &service,
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
        assert!(admin_actions.contains(&"runtime.admin_recover_known_good_chat".to_string()));
    }

    #[tokio::test]
    async fn core_api_admin_friend_key_lifecycle_returns_raw_key_exactly_once() {
        let app = admin_router(CoreStore::memory());
        let admin = identity_headers("admin@finite.vip", "true");
        let service = [("authorization".to_string(), "Bearer core-token".to_string())];

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
            &service,
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
            &service,
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
            &service,
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
