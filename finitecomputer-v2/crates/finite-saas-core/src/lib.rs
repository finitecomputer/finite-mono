pub mod api;
pub mod store;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

pub const CORE_SCHEMA_SQL: &str = concat!(
    include_str!("../migrations/0001_core.sql"),
    "\n",
    include_str!("../migrations/0002_runtime_upgrade.sql")
);
pub const RUNTIME_UPGRADE_ROLLBACK_RESCUE_SQL: &str =
    include_str!("../migrations/runtime_upgrade_rollback_rescue.sql");
const FIRST_SELF_SERVE_LAUNCH_CODE: &str = "off2026";
const DEFAULT_AGENT_CREATION_LEASE_SECONDS: i64 = 10 * 60;
const MAX_AGENT_CREATION_LEASE_SECONDS: i64 = 60 * 60;
const DEFAULT_FINITE_PRIVATE_LIMIT_PROFILE: &str = "finite-private-generous";
const DEFAULT_FINITE_PRIVATE_BURST_WINDOW_SECONDS: i64 = 5 * 60 * 60;
const DEFAULT_FINITE_PRIVATE_BURST_LIMIT_UNITS: i64 = 5_000_000;
const DEFAULT_FINITE_PRIVATE_WEEKLY_LIMIT_UNITS: i64 = 25_000_000;
const FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS: i64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BillingClass {
    Grandfathered,
    Off2026,
    Standard,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BillingSubscriptionStatus {
    Incomplete,
    IncompleteExpired,
    Trialing,
    Active,
    PastDue,
    Canceled,
    Unpaid,
    Paused,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportCandidateStatus {
    Pending,
    Claimed,
    AdminReview,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserLinkStatus {
    Pending,
    Linked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectMembershipRole {
    Owner,
    Admin,
    Member,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSummaryStatus {
    Online,
    Offline,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeArtifactKind {
    OciImage,
}

/// Product placement choice stored with an agent creation request. Provider
/// vocabulary stops at the runner adapter; feature behavior does not branch on
/// this value.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunnerClass {
    LocalDocker,
    AppleContainer,
    Kata,
    #[default]
    Phala,
    Enclavia,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeControlKind {
    Restart,
    RecoverKnownGoodChatRuntime,
    Upgrade,
    Stop,
    Destroy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeControlRequestStatus {
    Requested,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentCreationRequestStatus {
    Requested,
    Launching,
    Running,
    Failed,
    Cancelled,
}

/// Structured detail captured from a failed store operation. The full detail
/// is meant for server-side logs only; the user-facing surface stays generic.
///
/// For Postgres failures the fields mirror `tokio_postgres::error::DbError`
/// (`as_db_error`): SQLSTATE code, violated constraint, table, column, and the
/// server `DETAIL` line. These are exactly the fields that were being discarded
/// by the old `error.to_string()` == "db error" path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreErrorDetail {
    /// Full server-side message (Postgres primary message or serde error).
    pub message: String,
    /// SQLSTATE code, e.g. "23505" for a unique violation.
    pub code: Option<String>,
    /// Name of the violated constraint, when the failure is a constraint error.
    pub constraint: Option<String>,
    /// Table the failure references.
    pub table: Option<String>,
    /// Column the failure references.
    pub column: Option<String>,
    /// Postgres `DETAIL` line (e.g. "Key (customer_org_id)=(...) already exists.").
    pub detail: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("verified email is required")]
    MissingVerifiedEmail,
    #[error("WorkOS user id is required")]
    MissingWorkosUserId,
    #[error("source host id is required")]
    MissingSourceHostId,
    #[error("source host id must contain only lowercase letters, digits, and hyphens")]
    InvalidSourceHostId,
    #[error("source host relay url must be http or https")]
    InvalidSourceHostRelayUrl,
    #[error("source host relay admin token is required")]
    MissingSourceHostRelayAdminToken,
    #[error("agent display name is required")]
    MissingAgentDisplayName,
    #[error("agent creation idempotency key is required")]
    MissingAgentCreationIdempotencyKey,
    #[error("agent profile picture URL is invalid")]
    InvalidAgentProfilePictureUrl,
    #[error("launch code is required")]
    MissingLaunchCode,
    #[error("launch code is invalid")]
    InvalidLaunchCode,
    #[error("agent creation entitlement is exhausted")]
    AgentCreationEntitlementExhausted,
    #[error("billing is required before creating an agent")]
    BillingRequired,
    #[error("agent creation runner id is required")]
    MissingAgentCreationRunnerId,
    #[error("agent creation lease token is required")]
    MissingAgentCreationLeaseToken,
    #[error("agent creation lease duration is invalid")]
    InvalidAgentCreationLeaseDuration,
    #[error("agent creation request is not available")]
    AgentCreationRequestUnavailable,
    #[error("agent creation request was not found")]
    AgentCreationRequestNotFound,
    #[error("agent creation request lease does not match")]
    AgentCreationRequestLeaseConflict,
    #[error("agent creation request is not launching")]
    AgentCreationRequestNotLaunching,
    #[error("agent creation request cannot be cancelled")]
    AgentCreationRequestNotCancellable,
    #[error("source machine id is required")]
    MissingSourceMachineId,
    #[error("runtime relay token hash is required")]
    MissingRuntimeRelayTokenHash,
    #[error("runtime relay token is required")]
    MissingRuntimeRelayToken,
    #[error("runtime relay token is invalid")]
    InvalidRuntimeRelayToken,
    #[error("runtime heartbeat was not found")]
    RuntimeHeartbeatNotFound,
    #[error("runtime artifact id is required")]
    MissingRuntimeArtifactId,
    #[error("runtime artifact reference is required")]
    MissingRuntimeArtifactReference,
    #[error("runtime artifact version label is required")]
    MissingRuntimeArtifactVersionLabel,
    #[error("runtime artifact state schema version is required")]
    MissingRuntimeArtifactStateSchemaVersion,
    #[error("runtime artifact was not found")]
    RuntimeArtifactNotFound,
    #[error("runtime artifact is not promoted")]
    RuntimeArtifactNotPromoted,
    #[error("runtime artifact is retired")]
    RuntimeArtifactRetired,
    #[error("a promoted or runtime-referenced artifact is immutable")]
    RuntimeArtifactImmutable,
    #[error("project was not found")]
    ProjectNotFound,
    #[error("project runtime was not found")]
    ProjectRuntimeNotFound,
    #[error("runtime restart is not supported for this runtime")]
    RuntimeRestartUnsupported,
    #[error("runtime upgrade is supported only for Kata runtimes created by Core")]
    RuntimeUpgradeUnsupported,
    #[error("runtime upgrades are not enabled for this Core generation")]
    RuntimeUpgradeNotEnabled,
    #[error("runtime upgrade target is incompatible with the mounted state schema")]
    RuntimeUpgradeStateSchemaIncompatible,
    #[error("a different runtime upgrade is already in progress")]
    RuntimeUpgradeTargetConflict,
    #[error("another runtime control operation is already in progress")]
    RuntimeControlOperationConflict,
    #[error("runtime upgrade completion did not match the requested artifact")]
    RuntimeUpgradeCompletionMismatch,
    #[error("runtime control request was not found")]
    RuntimeControlRequestNotFound,
    #[error("runtime control request is not running")]
    RuntimeControlRequestNotRunning,
    #[error("runtime control request lease does not match")]
    RuntimeControlRequestLeaseConflict,
    #[error("runtime control request failure message is required")]
    MissingRuntimeControlFailureMessage,
    #[error("finite private api key is required")]
    MissingFinitePrivateApiKey,
    #[error("finite private api key is invalid")]
    InvalidFinitePrivateApiKey,
    #[error("finite private grant was not found")]
    FinitePrivateGrantNotFound,
    #[error("finite private grant is not active")]
    FinitePrivateGrantNotActive,
    #[error("finite private limit profile was not found")]
    FinitePrivateLimitProfileNotFound,
    #[error("finite private reservation was not found")]
    FinitePrivateReservationNotFound,
    #[error("finite private reservation is already settled")]
    FinitePrivateReservationAlreadySettled,
    #[error("Stripe customer id is required")]
    MissingStripeCustomerId,
    #[error("Stripe subscription id is required")]
    MissingStripeSubscriptionId,
    #[error("Stripe standard price id is required before granting billing entitlement")]
    MissingStripeStandardPriceId,
    #[error("Stripe subscription price is not eligible for hosted agents")]
    StripeSubscriptionPriceMismatch,
    #[error("Stripe customer is already linked to a different org")]
    StripeCustomerConflict,
    #[error("billing account was not found")]
    BillingAccountNotFound,
    #[error("billing subscription status is invalid")]
    InvalidBillingSubscriptionStatus,
    #[error("finite private usage estimate is invalid")]
    InvalidFinitePrivateUsageEstimate,
    #[error("agent creation failure message is required")]
    MissingAgentCreationFailureMessage,
    #[error("timestamp is invalid")]
    InvalidTimestamp,
    #[error("WorkOS user is already linked to a different email")]
    WorkosUserConflict,
    #[error("failed to format current time")]
    TimeFormat(#[from] time::error::Format),
    #[error("store error: {0}")]
    Store(String),
    /// A failed store operation with structured, log-only detail. The `Display`
    /// impl is intentionally generic ("database error") so the detail never
    /// leaks into a user-facing response; it is logged server-side in the
    /// `ApiError` conversion behind a correlation id.
    #[error("database error")]
    Database(Box<StoreErrorDetail>),
}

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExistingHostProjectImport {
    pub source_host_id: String,
    pub source_machine_id: String,
    pub owner_email: Option<String>,
    pub display_name: String,
    pub hostname: Option<String>,
    pub runtime_host: Option<String>,
    pub runtime_status: RuntimeSummaryStatus,
    pub active_inference_profile: Option<String>,
    pub hermes_available: Option<bool>,
    pub published_app_urls: Vec<String>,
    pub known_external_channel_participants: Vec<KnownExternalChannelParticipant>,
    pub admin_visible_to_emails: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnownExternalChannelParticipant {
    pub channel: String,
    pub external_user_id: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreUser {
    pub id: String,
    pub email: String,
    pub status: UserLinkStatus,
    pub workos_user_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomerOrganization {
    pub id: String,
    pub owner_user_id: String,
    pub name: String,
    pub billing_class: BillingClass,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomerBillingAccount {
    pub customer_org_id: String,
    pub stripe_customer_id: Option<String>,
    pub stripe_subscription_id: Option<String>,
    pub stripe_price_id: Option<String>,
    pub subscription_status: Option<BillingSubscriptionStatus>,
    pub current_period_end: Option<String>,
    pub cancel_at_period_end: bool,
    pub last_stripe_event_id: Option<String>,
    /// Unix timestamp (`event.created`) of the most recently APPLIED Stripe
    /// webhook for this account. The event-ordering guard compares against it so
    /// a stale event delivered out of order can't resurrect a canceled sub.
    pub last_stripe_event_created: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BillingOverview {
    pub customer_org: CustomerOrganization,
    pub billing_account: Option<CustomerBillingAccount>,
    pub agent_creation_entitlement: Option<AgentCreationEntitlement>,
    pub can_create_agent: bool,
    pub requires_billing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectImportCandidate {
    pub id: String,
    pub source_host_id: String,
    pub source_machine_id: String,
    pub source_import_key: String,
    pub owner_email: String,
    pub latest_host_owner_email: Option<String>,
    pub pending_user_id: String,
    pub customer_org_id: String,
    pub status: ImportCandidateStatus,
    pub project_id: Option<String>,
    pub agent_runtime_id: Option<String>,
    pub claimed_by_user_id: Option<String>,
    pub host_facts: HostOwnedRuntimeFacts,
    pub known_external_channel_participants: Vec<KnownExternalChannelParticipant>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostOwnedRuntimeFacts {
    pub display_name: String,
    pub hostname: Option<String>,
    pub runtime_host: String,
    pub runtime_status: RuntimeSummaryStatus,
    pub active_inference_profile: Option<String>,
    pub hermes_available: Option<bool>,
    pub published_app_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub customer_org_id: String,
    pub owner_user_id: String,
    pub display_name: String,
    pub import_candidate_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRuntime {
    pub id: String,
    pub project_id: String,
    pub source_host_id: String,
    pub source_machine_id: String,
    pub source_import_key: String,
    pub runtime_artifact_id: Option<String>,
    pub state_schema_version: Option<String>,
    pub host_facts: HostOwnedRuntimeFacts,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeArtifact {
    pub id: String,
    pub kind: RuntimeArtifactKind,
    pub reference: String,
    pub version_label: String,
    pub source_git_sha: Option<String>,
    pub finitec_version: Option<String>,
    pub hermes_source_ref: Option<String>,
    pub finite_platform_plugin_ref: Option<String>,
    pub state_schema_version: String,
    pub base_image: Option<String>,
    pub created_at: String,
    pub promoted_at: Option<String>,
    pub retired_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeRelayCredential {
    pub agent_runtime_id: String,
    pub token_hash: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeStatusSnapshot {
    pub agent_runtime_id: String,
    pub status: RuntimeSummaryStatus,
    pub last_heartbeat_at: Option<String>,
    pub runtime_host: String,
    pub active_inference_profile: Option<String>,
    pub hermes_available: Option<bool>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayHeartbeat {
    pub ok: bool,
    #[serde(rename = "machineId")]
    pub machine_id: String,
    #[serde(rename = "lastSeenAt")]
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayEventsOutput {
    #[serde(rename = "machineId")]
    pub machine_id: String,
    pub events: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectRuntimeLink {
    pub id: String,
    pub project_id: String,
    pub agent_runtime_id: String,
    pub active: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatIdentity {
    pub id: String,
    pub user_id: String,
    pub kind: String,
    pub device_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectRoomMembership {
    pub id: String,
    pub project_id: String,
    pub chat_identity_id: String,
    pub role: ProjectMembershipRole,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCreationEntitlement {
    pub id: String,
    pub customer_org_id: String,
    pub allowed_new_agent_runtimes: i32,
    pub launch_code: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LinkStripeCustomerInput {
    pub verified_email: String,
    pub workos_user_id: String,
    pub stripe_customer_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncStripeSubscriptionInput {
    pub customer_org_id: Option<String>,
    pub stripe_customer_id: String,
    pub stripe_subscription_id: String,
    pub stripe_price_id: Option<String>,
    pub expected_stripe_price_id: Option<String>,
    pub subscription_status: BillingSubscriptionStatus,
    pub current_period_end: Option<String>,
    pub cancel_at_period_end: bool,
    pub stripe_event_id: Option<String>,
    /// Unix timestamp of the Stripe `event.created` for this delivery. Threaded
    /// from the dashboard webhook so Core can order webhooks monotonically and
    /// ignore stale ones (see `sync_stripe_subscription`).
    pub stripe_event_created: Option<i64>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCreationRequest {
    pub id: String,
    pub customer_org_id: String,
    pub owner_user_id: String,
    pub project_id: String,
    pub idempotency_key: String,
    pub display_name: String,
    pub runner_class: RunnerClass,
    pub profile_picture_url: Option<String>,
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
pub struct RuntimeControlRequest {
    pub id: String,
    pub project_id: String,
    pub agent_runtime_id: String,
    pub source_host_id: String,
    pub source_machine_id: String,
    pub requested_by_user_id: String,
    pub kind: RuntimeControlKind,
    /// Present only for an explicit Upgrade operation. Restart deliberately
    /// remains bound to the Runtime's current artifact.
    #[serde(default)]
    pub target_runtime_artifact_id: Option<String>,
    pub status: RuntimeControlRequestStatus,
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
    /// Core-resolved immutable target for Upgrade. Runner adapters never choose
    /// a product release from process-global configuration while handling an
    /// existing Runtime.
    #[serde(default)]
    pub target_runtime_artifact: Option<RuntimeArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceHostRelayEndpoint {
    pub source_host_id: String,
    pub url: String,
    pub admin_token: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpsertSourceHostRelayEndpointInput {
    pub source_host_id: String,
    pub url: String,
    pub admin_token: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpsertRuntimeArtifactInput {
    pub id: String,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinitePrivateGrantStatus {
    Active,
    Revoked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinitePrivateApiKeyStatus {
    Active,
    Revoked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinitePrivateReservationStatus {
    Reserved,
    Settled,
    Denied,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinitePrivateSettlementKind {
    Actual,
    Estimate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinitePrivateLimitProfile {
    pub id: String,
    pub burst_window_seconds: i64,
    pub burst_limit_units: i64,
    pub weekly_limit_units: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinitePrivateGrant {
    pub id: String,
    pub user_id: String,
    pub limit_profile_id: String,
    pub status: FinitePrivateGrantStatus,
    pub current_window_started_at: Option<String>,
    pub current_window_used_units: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinitePrivateApiKey {
    pub id: String,
    pub grant_id: String,
    pub project_id: Option<String>,
    pub agent_runtime_id: Option<String>,
    pub key_hash: String,
    pub status: FinitePrivateApiKeyStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinitePrivateAdminAuditEvent {
    pub id: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub grant_id: Option<String>,
    pub api_key_id: Option<String>,
    pub actor: String,
    pub metadata: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FinitePrivateAdminState {
    pub grants: Vec<FinitePrivateGrant>,
    pub api_keys: Vec<FinitePrivateApiKey>,
    pub admin_audit_events: Vec<FinitePrivateAdminAuditEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinitePrivateReservation {
    pub id: String,
    pub request_id: String,
    pub api_key_id: String,
    pub grant_id: String,
    pub endpoint: String,
    pub model: String,
    pub estimated_usage_units: i64,
    pub reserved_usage_units: i64,
    pub settled_usage_units: Option<i64>,
    pub settlement_kind: Option<FinitePrivateSettlementKind>,
    pub status: FinitePrivateReservationStatus,
    pub usage_formula_version: String,
    pub upstream_status: Option<i32>,
    pub upstream_error_class: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinitePrivateUsageDecision {
    pub decision: String,
    pub reservation_id: Option<String>,
    pub limit_profile: Option<String>,
    pub burst_limit_units: Option<i64>,
    pub burst_remaining_units: Option<i64>,
    pub burst_reset_at: Option<String>,
    pub weekly_limit_units: Option<i64>,
    pub weekly_remaining_units: Option<i64>,
    pub weekly_reset_at: Option<String>,
    pub error: Option<FinitePrivateUsageError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinitePrivateUsageError {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    pub code: String,
    pub retry_after: Option<i64>,
    pub reset_at: Option<String>,
    pub dashboard_url: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApproveFinitePrivateGrantInput {
    pub verified_email: String,
    pub workos_user_id: Option<String>,
    pub limit_profile_id: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IssueFinitePrivateApiKeyInput {
    pub grant_id: String,
    pub raw_key: String,
    pub project_id: Option<String>,
    pub agent_runtime_id: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionFinitePrivateRuntimeKeyInput {
    pub request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    pub source_host_id: Option<String>,
    pub source_machine_id: Option<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionFinitePrivateRuntimeKeyResult {
    pub grant: FinitePrivateGrant,
    pub api_key: FinitePrivateApiKey,
    pub raw_api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RevokeFinitePrivateGrantInput {
    pub grant_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RevokeFinitePrivateApiKeyInput {
    pub key_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RotateFinitePrivateApiKeyInput {
    pub key_id: String,
    pub raw_key: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResetFinitePrivateUsageWindowInput {
    pub grant_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReserveFinitePrivateUsageInput {
    pub request_id: String,
    pub presented_api_key: String,
    pub endpoint: String,
    pub model: String,
    pub estimated_prompt_tokens: i64,
    pub estimated_completion_tokens: i64,
    pub estimated_usage_units: i64,
    pub usage_formula_version: String,
    pub dashboard_url: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SettleFinitePrivateReservationInput {
    pub reservation_id: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettleFinitePrivateReservationResult {
    pub settled: bool,
    pub reservation_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BridgeCoreState {
    pub users: BTreeMap<String, CoreUser>,
    pub customer_orgs: BTreeMap<String, CustomerOrganization>,
    pub project_import_candidates: BTreeMap<String, ProjectImportCandidate>,
    pub projects: BTreeMap<String, Project>,
    pub runtime_artifacts: BTreeMap<String, RuntimeArtifact>,
    pub agent_runtimes: BTreeMap<String, AgentRuntime>,
    pub runtime_relay_credentials: BTreeMap<String, RuntimeRelayCredential>,
    pub runtime_status_snapshots: BTreeMap<String, RuntimeStatusSnapshot>,
    pub project_runtime_links: BTreeMap<String, ProjectRuntimeLink>,
    pub chat_identities: BTreeMap<String, ChatIdentity>,
    pub project_room_memberships: BTreeMap<String, ProjectRoomMembership>,
    pub agent_creation_entitlements: BTreeMap<String, AgentCreationEntitlement>,
    pub agent_creation_requests: BTreeMap<String, AgentCreationRequest>,
    pub runtime_control_requests: BTreeMap<String, RuntimeControlRequest>,
    pub source_host_relays: BTreeMap<String, SourceHostRelayEndpoint>,
    pub finite_private_limit_profiles: BTreeMap<String, FinitePrivateLimitProfile>,
    pub finite_private_grants: BTreeMap<String, FinitePrivateGrant>,
    pub finite_private_api_keys: BTreeMap<String, FinitePrivateApiKey>,
    pub finite_private_admin_audit_events: BTreeMap<String, FinitePrivateAdminAuditEvent>,
    pub finite_private_reservations: BTreeMap<String, FinitePrivateReservation>,
    pub customer_billing_accounts: BTreeMap<String, CustomerBillingAccount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileExistingHostImportsOptions {
    pub allowlisted_owner_emails: Vec<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileExistingHostImportsReport {
    pub created_candidates: Vec<String>,
    pub updated_candidates: Vec<String>,
    pub skipped_records: Vec<SkippedImportRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkippedImportRecord {
    pub source_import_key: String,
    pub reason: SkippedImportReason,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkippedImportReason {
    MissingOwnerEmail,
    OwnerNotAllowlisted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClaimProjectImportsInput {
    pub verified_email: String,
    pub workos_user_id: String,
    pub selected_candidate_ids: Vec<String>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaimProjectImportsResult {
    pub claimed_project_ids: Vec<String>,
    pub already_claimed_project_ids: Vec<String>,
    pub denied_candidate_ids: Vec<String>,
    pub missing_candidate_ids: Vec<String>,
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
    pub runner_class: RunnerClass,
    pub profile_picture_url: Option<String>,
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
pub struct AdminRuntimeUpgradeInput {
    pub admin_verified_email: String,
    pub admin_workos_user_id: String,
    pub project_id: String,
    pub target_runtime_artifact_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminRuntimeControlInput {
    pub admin_verified_email: String,
    pub admin_workos_user_id: String,
    pub project_id: String,
    pub now: Option<String>,
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
    pub runtime_status: RuntimeSummaryStatus,
    pub last_heartbeat_at: Option<String>,
    pub status_updated_at: Option<String>,
    pub runtime_updated_at: String,
    pub hermes_available: Option<bool>,
    pub published_app_urls: Vec<String>,
    pub active_finite_private_key_count: i64,
    pub runtime_link_active: bool,
    pub supports_runtime_control: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminIssueFinitePrivateFriendKeyInput {
    pub admin_verified_email: String,
    pub friend_email: String,
    pub limit_profile_id: Option<String>,
    /// Raw key material generated by the caller; only its hash is stored.
    pub raw_key: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdminIssuedFinitePrivateKey {
    pub grant: FinitePrivateGrant,
    pub api_key: FinitePrivateApiKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminRotateFinitePrivateApiKeyInput {
    pub admin_verified_email: String,
    pub key_id: String,
    /// Replacement raw key material generated by the caller; only its hash is stored.
    pub raw_key: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminRevokeFinitePrivateApiKeyInput {
    pub admin_verified_email: String,
    pub key_id: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminResetFinitePrivateUsageWindowInput {
    pub admin_verified_email: String,
    pub grant_id: String,
    pub now: Option<String>,
}

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
pub struct CompleteRuntimeControlRequestInput {
    pub request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    /// Required for Upgrade and rejected when it does not exactly match the
    /// Core-bound target artifact/schema. Other lifecycle operations leave
    /// these fields empty.
    pub runtime_artifact_id: Option<String>,
    pub state_schema_version: Option<String>,
    pub runtime_host: Option<String>,
    pub published_app_urls: Option<Vec<String>>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailRuntimeControlRequestInput {
    pub request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    pub failure_message: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LinkVerifiedUserInput {
    pub verified_email: String,
    pub workos_user_id: String,
    pub now: Option<String>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunnerLeaseCapacity {
    #[serde(default)]
    pub draining: bool,
    #[serde(default)]
    pub max_sandbox_count: Option<u32>,
    #[serde(default)]
    pub active_sandbox_count: Option<u32>,
    #[serde(default)]
    pub available_memory_bytes: Option<u64>,
    /// Adapter classes this worker can actually reconcile. Empty is accepted
    /// only for backwards-compatible test/old-worker leasing; current workers
    /// always advertise one or more classes.
    #[serde(default)]
    pub runner_classes: Vec<RunnerClass>,
}

impl RunnerLeaseCapacity {
    pub fn accepts_runtime_control(&self) -> bool {
        !self.draining
    }

    pub fn accepts_agent_creation(&self) -> bool {
        self.accepts_runtime_control() && !self.sandbox_limit_reached()
    }

    pub fn supports_runner_class(&self, runner_class: RunnerClass) -> bool {
        self.runner_classes.is_empty() || self.runner_classes.contains(&runner_class)
    }

    pub fn agent_creation_rejection_reason(&self) -> Option<&'static str> {
        if self.draining {
            Some("runner is draining")
        } else if self.sandbox_limit_reached() {
            Some("runner sandbox capacity is full")
        } else {
            None
        }
    }

    fn sandbox_limit_reached(&self) -> bool {
        match (self.active_sandbox_count, self.max_sandbox_count) {
            (Some(active), Some(max)) => active >= max,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCreationLease {
    pub project: Project,
    pub request: AgentCreationRequest,
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
pub struct RegisterAgentCreationRuntimeInput {
    pub request_id: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailAgentCreationRequestInput {
    pub request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    pub failure_message: String,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CancelAgentCreationRequestInput {
    pub request_id: String,
    pub now: Option<String>,
}

impl BridgeCoreState {
    pub fn reconcile_existing_host_imports(
        &mut self,
        records: &[ExistingHostProjectImport],
        options: ReconcileExistingHostImportsOptions,
    ) -> CoreResult<ReconcileExistingHostImportsReport> {
        let now = options.now.unwrap_or(current_time_iso()?);
        let allowlist = options
            .allowlisted_owner_emails
            .into_iter()
            .filter_map(|email| normalize_owner_email(Some(&email)))
            .collect::<BTreeSet<_>>();
        let mut report = ReconcileExistingHostImportsReport {
            created_candidates: Vec::new(),
            updated_candidates: Vec::new(),
            skipped_records: Vec::new(),
        };

        for record in records {
            let source_key = source_import_key(&record.source_host_id, &record.source_machine_id);
            let candidate_id = candidate_id_for(&source_key);
            let owner_email = match normalize_owner_email(record.owner_email.as_deref()) {
                Some(email) => email,
                None => {
                    report.skipped_records.push(SkippedImportRecord {
                        source_import_key: source_key,
                        reason: SkippedImportReason::MissingOwnerEmail,
                    });
                    continue;
                }
            };

            if self.project_import_candidates.contains_key(&candidate_id) {
                self.update_existing_candidate(&candidate_id, &owner_email, record, &now);
                report.updated_candidates.push(candidate_id);
                continue;
            }

            if !allowlist.contains(&owner_email) {
                report.skipped_records.push(SkippedImportRecord {
                    source_import_key: source_key,
                    reason: SkippedImportReason::OwnerNotAllowlisted,
                });
                continue;
            }

            let user = self.ensure_pending_user(&owner_email, &now)?;
            let org = self.ensure_personal_org(&user, BillingClass::Grandfathered, &now)?;
            self.project_import_candidates.insert(
                candidate_id.clone(),
                ProjectImportCandidate {
                    id: candidate_id.clone(),
                    source_host_id: normalize_id_part(&record.source_host_id),
                    source_machine_id: normalize_id_part(&record.source_machine_id),
                    source_import_key: source_key,
                    owner_email,
                    latest_host_owner_email: record
                        .owner_email
                        .as_deref()
                        .and_then(|email| normalize_owner_email(Some(email))),
                    pending_user_id: user.id,
                    customer_org_id: org.id,
                    status: ImportCandidateStatus::Pending,
                    project_id: None,
                    agent_runtime_id: None,
                    claimed_by_user_id: None,
                    host_facts: host_facts_from_record(record),
                    known_external_channel_participants: record
                        .known_external_channel_participants
                        .clone(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            );
            report.created_candidates.push(candidate_id);
        }

        Ok(report)
    }

    pub fn claim_project_imports(
        &mut self,
        input: ClaimProjectImportsInput,
    ) -> CoreResult<ClaimProjectImportsResult> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let verified_email = normalize_owner_email(Some(&input.verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let workos_user_id = input.workos_user_id.trim().to_string();
        if workos_user_id.is_empty() {
            return Err(CoreError::MissingWorkosUserId);
        }

        let user = self.ensure_linked_user(&verified_email, &workos_user_id, &now)?;
        let mut result = ClaimProjectImportsResult::default();
        let selected_candidate_ids = input
            .selected_candidate_ids
            .into_iter()
            .collect::<BTreeSet<_>>();

        for candidate_id in selected_candidate_ids {
            let Some(candidate) = self.project_import_candidates.get(&candidate_id).cloned() else {
                result.missing_candidate_ids.push(candidate_id);
                continue;
            };

            if candidate.owner_email != verified_email || candidate.pending_user_id != user.id {
                result.denied_candidate_ids.push(candidate.id);
                continue;
            }

            if candidate.status == ImportCandidateStatus::Claimed {
                if let Some(project_id) = candidate.project_id {
                    self.ensure_hosted_web_membership(&user, &project_id, &now);
                    result.already_claimed_project_ids.push(project_id);
                }
                continue;
            }

            let project_id = project_id_for(&candidate.id);
            let runtime_id = agent_runtime_id_for(&candidate.id);
            let project = Project {
                id: project_id.clone(),
                customer_org_id: candidate.customer_org_id.clone(),
                owner_user_id: user.id.clone(),
                display_name: candidate.host_facts.display_name.clone(),
                import_candidate_id: Some(candidate.id.clone()),
                created_at: now.clone(),
                updated_at: now.clone(),
            };
            let runtime = AgentRuntime {
                id: runtime_id.clone(),
                project_id: project_id.clone(),
                source_host_id: candidate.source_host_id.clone(),
                source_machine_id: candidate.source_machine_id.clone(),
                source_import_key: candidate.source_import_key.clone(),
                runtime_artifact_id: None,
                state_schema_version: None,
                host_facts: candidate.host_facts.clone(),
                created_at: now.clone(),
                updated_at: now.clone(),
            };
            let link = ProjectRuntimeLink {
                id: project_runtime_link_id_for(&project_id, &runtime_id),
                project_id: project_id.clone(),
                agent_runtime_id: runtime_id.clone(),
                active: true,
                created_at: now.clone(),
            };

            self.projects.insert(project_id.clone(), project);
            self.agent_runtimes.insert(runtime_id.clone(), runtime);
            self.project_runtime_links.insert(link.id.clone(), link);
            self.project_import_candidates.insert(
                candidate.id.clone(),
                ProjectImportCandidate {
                    status: ImportCandidateStatus::Claimed,
                    project_id: Some(project_id.clone()),
                    agent_runtime_id: Some(runtime_id),
                    claimed_by_user_id: Some(user.id.clone()),
                    updated_at: now.clone(),
                    ..candidate
                },
            );
            self.ensure_hosted_web_membership(&user, &project_id, &now);
            result.claimed_project_ids.push(project_id);
        }

        Ok(result)
    }

    pub fn request_agent_creation(
        &mut self,
        input: RequestAgentCreationInput,
    ) -> CoreResult<RequestAgentCreationResult> {
        self.request_agent_creation_configured(input, AgentCreationConfiguration::default())
    }

    pub fn request_agent_creation_configured(
        &mut self,
        input: RequestAgentCreationInput,
        configuration: AgentCreationConfiguration,
    ) -> CoreResult<RequestAgentCreationResult> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let verified_email = normalize_owner_email(Some(&input.verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let workos_user_id = input.workos_user_id.trim().to_string();
        if workos_user_id.is_empty() {
            return Err(CoreError::MissingWorkosUserId);
        }
        let display_name =
            trim_to_option(Some(&input.display_name)).ok_or(CoreError::MissingAgentDisplayName)?;
        let idempotency_key = normalize_idempotency_key(&input.idempotency_key)
            .ok_or(CoreError::MissingAgentCreationIdempotencyKey)?;
        let profile_picture_url =
            normalize_profile_picture_url(configuration.profile_picture_url.as_deref())?;
        let launch_code = trim_to_option(Some(&input.launch_code));
        let billing_class = if launch_code.is_some() {
            BillingClass::Off2026
        } else {
            BillingClass::Standard
        };
        // Resolve the existing org (if any) by natural key so we can gate on
        // billing/launch BEFORE minting any surrogate rows — a failed gate must
        // not leave a stray user/org behind (Postgres rolls back; here we simply
        // don't create until the gate passes).
        let existing_org_id = self
            .find_user_by_email(&verified_email)
            .and_then(|user| self.find_personal_org_by_owner(&user.id))
            .map(|org| org.id);
        if let Some(code) = launch_code.as_deref() {
            self.validate_agent_creation_launch_code(
                existing_org_id.as_deref().unwrap_or_default(),
                code,
            )?;
        } else if !existing_org_id
            .as_deref()
            .is_some_and(|org_id| self.customer_org_has_active_billing(org_id))
        {
            return Err(CoreError::BillingRequired);
        }

        let user = self.ensure_linked_user_with_billing_class(
            &verified_email,
            &workos_user_id,
            billing_class,
            &now,
        )?;
        let org = self.ensure_personal_org(&user, billing_class, &now)?;
        // Idempotency is enforced by the natural key (owner_user_id,
        // idempotency_key) — matching the UNIQUE the DB carries — NOT by deriving
        // the request id from those inputs. Look up an existing request; if
        // present, return it as reused.
        if let Some(existing_request) =
            self.find_agent_creation_request_by_idempotency(&user.id, &idempotency_key)
        {
            let Some(project) = self.projects.get(&existing_request.project_id).cloned() else {
                return Err(CoreError::Store(format!(
                    "agent creation request {} references missing project {}",
                    existing_request.id, existing_request.project_id
                )));
            };
            self.ensure_hosted_web_membership(&user, &project.id, &now);
            return Ok(RequestAgentCreationResult {
                project,
                request: existing_request,
                reused: true,
            });
        }

        let entitlement =
            self.ensure_agent_creation_entitlement(&org.id, launch_code.as_deref(), &now)?;
        let active_request_count = self.active_agent_creation_entitlement_count(&org.id);
        if active_request_count >= entitlement.allowed_new_agent_runtimes {
            return Err(CoreError::AgentCreationEntitlementExhausted);
        }

        // Fresh surrogate ids for the new request and its project.
        let request_id = new_agent_creation_request_id()?;
        let project_id = new_self_service_project_id()?;
        let project = Project {
            id: project_id.clone(),
            customer_org_id: org.id.clone(),
            owner_user_id: user.id.clone(),
            display_name: display_name.clone(),
            import_candidate_id: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let request = AgentCreationRequest {
            id: request_id,
            customer_org_id: org.id,
            owner_user_id: user.id.clone(),
            project_id: project_id.clone(),
            idempotency_key,
            display_name,
            runner_class: configuration.runner_class,
            profile_picture_url,
            status: AgentCreationRequestStatus::Requested,
            requested_launch_code: launch_code,
            agent_runtime_id: None,
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            failure_message: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        self.projects.insert(project.id.clone(), project.clone());
        self.agent_creation_requests
            .insert(request.id.clone(), request.clone());
        self.ensure_hosted_web_membership(&user, &project_id, &request.created_at);

        Ok(RequestAgentCreationResult {
            project,
            request,
            reused: false,
        })
    }

    pub fn link_verified_user(&mut self, input: LinkVerifiedUserInput) -> CoreResult<CoreUser> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let verified_email = normalize_owner_email(Some(&input.verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let workos_user_id = input.workos_user_id.trim().to_string();
        if workos_user_id.is_empty() {
            return Err(CoreError::MissingWorkosUserId);
        }

        self.ensure_linked_user_with_billing_class(
            &verified_email,
            &workos_user_id,
            BillingClass::Standard,
            &now,
        )
    }

    pub fn billing_overview(
        &mut self,
        input: LinkVerifiedUserInput,
    ) -> CoreResult<BillingOverview> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let verified_email = normalize_owner_email(Some(&input.verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let workos_user_id = input.workos_user_id.trim().to_string();
        if workos_user_id.is_empty() {
            return Err(CoreError::MissingWorkosUserId);
        }
        let user = self.ensure_linked_user_with_billing_class(
            &verified_email,
            &workos_user_id,
            BillingClass::Standard,
            &now,
        )?;
        let org = self.ensure_personal_org(&user, BillingClass::Standard, &now)?;
        Ok(self.billing_overview_for_org(&org))
    }

    pub fn link_stripe_customer(
        &mut self,
        input: LinkStripeCustomerInput,
    ) -> CoreResult<CustomerBillingAccount> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let verified_email = normalize_owner_email(Some(&input.verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let workos_user_id = input.workos_user_id.trim().to_string();
        if workos_user_id.is_empty() {
            return Err(CoreError::MissingWorkosUserId);
        }
        let stripe_customer_id = trim_to_option(Some(&input.stripe_customer_id))
            .ok_or(CoreError::MissingStripeCustomerId)?;
        let user = self.ensure_linked_user_with_billing_class(
            &verified_email,
            &workos_user_id,
            BillingClass::Standard,
            &now,
        )?;
        let org = self.ensure_personal_org(&user, BillingClass::Standard, &now)?;
        self.link_stripe_customer_to_org(&org.id, &stripe_customer_id, &now)
    }

    pub fn sync_stripe_subscription(
        &mut self,
        input: SyncStripeSubscriptionInput,
    ) -> CoreResult<CustomerBillingAccount> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let stripe_customer_id = trim_to_option(Some(&input.stripe_customer_id))
            .ok_or(CoreError::MissingStripeCustomerId)?;
        let stripe_subscription_id = trim_to_option(Some(&input.stripe_subscription_id))
            .ok_or(CoreError::MissingStripeSubscriptionId)?;
        let stripe_price_id = trim_to_option(input.stripe_price_id.as_deref());
        let customer_org_id = match trim_to_option(input.customer_org_id.as_deref()) {
            Some(org_id) => org_id,
            None => self
                .customer_billing_accounts
                .values()
                .find(|account| account.stripe_customer_id.as_deref() == Some(&stripe_customer_id))
                .map(|account| account.customer_org_id.clone())
                .ok_or(CoreError::BillingAccountNotFound)?,
        };
        if !self.customer_orgs.contains_key(&customer_org_id) {
            return Err(CoreError::BillingAccountNotFound);
        }
        // Event-ordering guard: for the SAME subscription, ignore a webhook whose
        // Stripe `event.created` predates the last one we applied. Without this a
        // stale `active` delivered after `canceled` resurrects billing.
        if let Some(existing_account) = self.customer_billing_accounts.get(&customer_org_id)
            && existing_account.stripe_subscription_id.as_deref()
                == Some(stripe_subscription_id.as_str())
            && let (Some(last_created), Some(incoming_created)) = (
                existing_account.last_stripe_event_created,
                input.stripe_event_created,
            )
            && incoming_created < last_created
        {
            return Ok(existing_account.clone());
        }
        if let Some(existing_account) = self.customer_billing_accounts.get(&customer_org_id)
            && let Some(existing_subscription_id) =
                existing_account.stripe_subscription_id.as_deref()
            && existing_subscription_id != stripe_subscription_id
            && !should_replace_stripe_subscription(
                existing_account.subscription_status,
                input.subscription_status,
            )
        {
            return Ok(existing_account.clone());
        }
        let mut account =
            self.link_stripe_customer_to_org(&customer_org_id, &stripe_customer_id, &now)?;
        if input.subscription_status.can_create_agent() {
            let expected_price_id = trim_to_option(input.expected_stripe_price_id.as_deref())
                .ok_or(CoreError::MissingStripeStandardPriceId)?;
            if stripe_price_id.as_deref() != Some(expected_price_id.as_str()) {
                return Err(CoreError::StripeSubscriptionPriceMismatch);
            }
        }
        account.stripe_subscription_id = Some(stripe_subscription_id);
        account.stripe_price_id = stripe_price_id;
        account.subscription_status = Some(input.subscription_status);
        account.current_period_end = input.current_period_end;
        account.cancel_at_period_end = input.cancel_at_period_end;
        account.last_stripe_event_id = trim_to_option(input.stripe_event_id.as_deref());
        account.last_stripe_event_created = input
            .stripe_event_created
            .or(account.last_stripe_event_created);
        account.updated_at = now.clone();
        self.customer_billing_accounts
            .insert(customer_org_id.clone(), account.clone());

        if input.subscription_status.can_create_agent() {
            self.ensure_billing_agent_creation_entitlement(&customer_org_id, &now);
            if let Some(org) = self.customer_orgs.get_mut(&customer_org_id) {
                org.billing_class = BillingClass::Standard;
                org.updated_at = now;
            }
        } else if let Some(entitlement) = self
            .agent_creation_entitlements
            .values_mut()
            .find(|entitlement| entitlement.customer_org_id == customer_org_id)
            .filter(|entitlement| entitlement.launch_code.is_none())
        {
            entitlement.allowed_new_agent_runtimes = 0;
            entitlement.updated_at = now;
        }

        Ok(account)
    }

    pub fn request_runtime_restart(
        &mut self,
        input: RequestRuntimeRestartInput,
    ) -> CoreResult<RuntimeControlRequest> {
        self.request_runtime_control(input, RuntimeControlKind::Restart, None)
    }

    pub fn request_runtime_recover_known_good_chat(
        &mut self,
        input: RequestRuntimeRecoverKnownGoodChatInput,
    ) -> CoreResult<RuntimeControlRequest> {
        self.request_runtime_control(input, RuntimeControlKind::RecoverKnownGoodChatRuntime, None)
    }

    pub fn request_runtime_stop(
        &mut self,
        input: RequestRuntimeStopInput,
    ) -> CoreResult<RuntimeControlRequest> {
        self.request_runtime_control(input, RuntimeControlKind::Stop, None)
    }

    pub fn request_runtime_destroy(
        &mut self,
        input: RequestRuntimeDestroyInput,
    ) -> CoreResult<RuntimeControlRequest> {
        self.request_runtime_control(input, RuntimeControlKind::Destroy, None)
    }

    fn request_runtime_control(
        &mut self,
        input: RequestRuntimeRestartInput,
        kind: RuntimeControlKind,
        target_runtime_artifact_id: Option<String>,
    ) -> CoreResult<RuntimeControlRequest> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let verified_email = normalize_owner_email(Some(&input.verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let workos_user_id = input.workos_user_id.trim().to_string();
        if workos_user_id.is_empty() {
            return Err(CoreError::MissingWorkosUserId);
        }
        let user = self.ensure_linked_user(&verified_email, &workos_user_id, &now)?;
        let project = self
            .projects
            .get(&input.project_id)
            .cloned()
            .ok_or(CoreError::ProjectNotFound)?;
        if project.owner_user_id != user.id {
            return Err(CoreError::ProjectNotFound);
        }
        self.enqueue_runtime_control_request(
            &project,
            &user.id,
            kind,
            target_runtime_artifact_id,
            now,
        )
    }

    pub fn admin_request_runtime_restart(
        &mut self,
        input: AdminRuntimeControlInput,
    ) -> CoreResult<RuntimeControlRequest> {
        self.admin_request_runtime_control(input, RuntimeControlKind::Restart, None)
    }

    pub fn admin_request_runtime_recover_known_good_chat(
        &mut self,
        input: AdminRuntimeControlInput,
    ) -> CoreResult<RuntimeControlRequest> {
        self.admin_request_runtime_control(
            input,
            RuntimeControlKind::RecoverKnownGoodChatRuntime,
            None,
        )
    }

    pub fn admin_request_runtime_upgrade(
        &mut self,
        input: AdminRuntimeUpgradeInput,
    ) -> CoreResult<RuntimeControlRequest> {
        self.admin_request_runtime_control(
            AdminRuntimeControlInput {
                admin_verified_email: input.admin_verified_email,
                admin_workos_user_id: input.admin_workos_user_id,
                project_id: input.project_id,
                now: input.now,
            },
            RuntimeControlKind::Upgrade,
            Some(input.target_runtime_artifact_id),
        )
    }

    /// Admin variant of `request_runtime_control`: the acting user does not
    /// have to own the project, and the action is written to the admin audit
    /// log with the admin's verified email as actor. Core-side admin
    /// authorization (`FC_CORE_ADMIN_EMAILS`) happens in the API layer before
    /// this is reachable.
    fn admin_request_runtime_control(
        &mut self,
        input: AdminRuntimeControlInput,
        kind: RuntimeControlKind,
        target_runtime_artifact_id: Option<String>,
    ) -> CoreResult<RuntimeControlRequest> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let admin_workos_user_id = input.admin_workos_user_id.trim().to_string();
        if admin_workos_user_id.is_empty() {
            return Err(CoreError::MissingWorkosUserId);
        }
        let admin_user = self.ensure_linked_user(&admin_email, &admin_workos_user_id, &now)?;
        let project = self
            .projects
            .get(&input.project_id)
            .cloned()
            .ok_or(CoreError::ProjectNotFound)?;
        let request = self.enqueue_runtime_control_request(
            &project,
            &admin_user.id,
            kind,
            target_runtime_artifact_id,
            now.clone(),
        )?;
        self.record_finite_private_admin_audit_event(FinitePrivateAdminAuditRecord {
            action: match kind {
                RuntimeControlKind::Restart => "runtime.admin_restart",
                RuntimeControlKind::RecoverKnownGoodChatRuntime => {
                    "runtime.admin_recover_known_good_chat"
                }
                RuntimeControlKind::Upgrade => "runtime.admin_upgrade",
                RuntimeControlKind::Stop => "runtime.admin_stop",
                RuntimeControlKind::Destroy => "runtime.admin_destroy",
            },
            target_type: "agent_runtime",
            target_id: &request.agent_runtime_id,
            grant_id: None,
            api_key_id: None,
            actor: Some(&admin_email),
            metadata: json!({
                "projectId": request.project_id.clone(),
                "runtimeControlRequestId": request.id.clone(),
                "kind": kind.as_str(),
                "targetRuntimeArtifactId": request.target_runtime_artifact_id.clone(),
            }),
            created_at: &now,
        });
        Ok(request)
    }

    fn enqueue_runtime_control_request(
        &mut self,
        project: &Project,
        requested_by_user_id: &str,
        kind: RuntimeControlKind,
        target_runtime_artifact_id: Option<String>,
        now: String,
    ) -> CoreResult<RuntimeControlRequest> {
        let runtime = self
            .active_runtime_for_project(&project.id)
            .ok_or(CoreError::ProjectRuntimeNotFound)?;
        let artifact_id = runtime
            .runtime_artifact_id
            .as_deref()
            .ok_or(CoreError::RuntimeRestartUnsupported)?;
        let artifact = self
            .runtime_artifacts
            .get(artifact_id)
            .ok_or(CoreError::RuntimeArtifactNotFound)?;
        if !runtime_artifact_supports_control(artifact.kind) {
            return Err(CoreError::RuntimeRestartUnsupported);
        }

        let target_runtime_artifact_id = match kind {
            RuntimeControlKind::Upgrade => {
                let target_id = trim_to_option(target_runtime_artifact_id.as_deref())
                    .ok_or(CoreError::MissingRuntimeArtifactId)?;
                let runner_class = self
                    .agent_creation_requests
                    .values()
                    .find(|request| {
                        request.agent_runtime_id.as_deref() == Some(runtime.id.as_str())
                            && request.status == AgentCreationRequestStatus::Running
                    })
                    .map(|request| request.runner_class)
                    .ok_or(CoreError::RuntimeUpgradeUnsupported)?;
                if runner_class != RunnerClass::Kata {
                    return Err(CoreError::RuntimeUpgradeUnsupported);
                }
                let target = self.launchable_runtime_artifact(&target_id)?;
                if target.kind != RuntimeArtifactKind::OciImage {
                    return Err(CoreError::RuntimeUpgradeUnsupported);
                }
                if !runtime_artifact_reference_is_immutable_oci(&target.reference) {
                    return Err(CoreError::RuntimeUpgradeUnsupported);
                }
                if runtime.state_schema_version.as_deref()
                    != Some(target.state_schema_version.as_str())
                {
                    return Err(CoreError::RuntimeUpgradeStateSchemaIncompatible);
                }
                Some(target.id)
            }
            _ => None,
        };

        if let Some(existing) = self
            .runtime_control_requests
            .values()
            .filter(|request| {
                request.agent_runtime_id == runtime.id
                    && matches!(
                        request.status,
                        RuntimeControlRequestStatus::Requested
                            | RuntimeControlRequestStatus::Running
                    )
            })
            .min_by_key(|request| (request.created_at.clone(), request.id.clone()))
            .cloned()
        {
            if existing.kind != kind {
                return Err(CoreError::RuntimeControlOperationConflict);
            }
            if kind == RuntimeControlKind::Upgrade
                && existing.target_runtime_artifact_id != target_runtime_artifact_id
            {
                return Err(CoreError::RuntimeUpgradeTargetConflict);
            }
            return Ok(existing);
        }

        let request = RuntimeControlRequest {
            id: runtime_control_request_id_for(&runtime.id, kind, &now),
            project_id: project.id.clone(),
            agent_runtime_id: runtime.id,
            source_host_id: runtime.source_host_id,
            source_machine_id: runtime.source_machine_id,
            requested_by_user_id: requested_by_user_id.to_string(),
            kind,
            target_runtime_artifact_id,
            status: RuntimeControlRequestStatus::Requested,
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            failure_message: None,
            created_at: now.clone(),
            updated_at: now,
            completed_at: None,
        };
        self.runtime_control_requests
            .insert(request.id.clone(), request.clone());
        Ok(request)
    }

    pub fn lease_agent_creation_request(
        &mut self,
        input: LeaseAgentCreationRequestInput,
    ) -> CoreResult<Option<AgentCreationLease>> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let now_time = parse_time(&now)?;
        let runner_id = trim_to_option(Some(&input.runner_id))
            .ok_or(CoreError::MissingAgentCreationRunnerId)?;
        let lease_token = trim_to_option(Some(&input.lease_token))
            .ok_or(CoreError::MissingAgentCreationLeaseToken)?;
        let lease_seconds = input
            .lease_seconds
            .unwrap_or(DEFAULT_AGENT_CREATION_LEASE_SECONDS);
        if !(1..=MAX_AGENT_CREATION_LEASE_SECONDS).contains(&lease_seconds) {
            return Err(CoreError::InvalidAgentCreationLeaseDuration);
        }
        if input
            .runner_capacity
            .as_ref()
            .is_some_and(|capacity| !capacity.accepts_agent_creation())
        {
            return Ok(None);
        }
        let lease_expires_at = (now_time + Duration::seconds(lease_seconds)).format(&Rfc3339)?;

        let request_id = self
            .agent_creation_requests
            .values()
            .filter(|request| self.agent_creation_request_is_leasable(request, now_time))
            .filter(|request| {
                input
                    .runner_capacity
                    .as_ref()
                    .is_none_or(|capacity| capacity.supports_runner_class(request.runner_class))
            })
            .min_by_key(|request| (request.created_at.clone(), request.id.clone()))
            .map(|request| request.id.clone());

        let Some(request_id) = request_id else {
            return Ok(None);
        };
        let request = {
            let Some(request) = self.agent_creation_requests.get_mut(&request_id) else {
                return Err(CoreError::AgentCreationRequestUnavailable);
            };

            request.status = AgentCreationRequestStatus::Launching;
            request.runner_id = Some(runner_id);
            request.lease_token = Some(lease_token);
            request.lease_expires_at = Some(lease_expires_at);
            request.failure_message = None;
            request.updated_at = now;
            request.clone()
        };

        let project = self
            .projects
            .get(&request.project_id)
            .cloned()
            .ok_or_else(|| {
                CoreError::Store(format!(
                    "agent creation request {} references missing project {}",
                    request.id, request.project_id
                ))
            })?;
        Ok(Some(AgentCreationLease { project, request }))
    }

    pub fn lease_runtime_control_request(
        &mut self,
        input: LeaseRuntimeControlRequestInput,
    ) -> CoreResult<Option<RuntimeControlLease>> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let now_time = parse_time(&now)?;
        let runner_id = trim_to_option(Some(&input.runner_id))
            .ok_or(CoreError::MissingAgentCreationRunnerId)?;
        let lease_token = trim_to_option(Some(&input.lease_token))
            .ok_or(CoreError::MissingAgentCreationLeaseToken)?;
        let lease_seconds = input
            .lease_seconds
            .unwrap_or(DEFAULT_AGENT_CREATION_LEASE_SECONDS);
        if !(1..=MAX_AGENT_CREATION_LEASE_SECONDS).contains(&lease_seconds) {
            return Err(CoreError::InvalidAgentCreationLeaseDuration);
        }
        if input
            .runner_capacity
            .as_ref()
            .is_some_and(|capacity| !capacity.accepts_runtime_control())
        {
            return Ok(None);
        }
        let source_host_id = input
            .source_host_id
            .as_deref()
            .map(normalize_source_host_id)
            .transpose()?;
        let lease_expires_at = (now_time + Duration::seconds(lease_seconds)).format(&Rfc3339)?;

        loop {
            let request_id = self
                .runtime_control_requests
                .values()
                .filter(|request| {
                    self.runtime_control_request_is_leasable(request, now_time)
                        && source_host_id
                            .as_deref()
                            .is_none_or(|host_id| request.source_host_id == host_id)
                })
                .min_by_key(|request| (request.created_at.clone(), request.id.clone()))
                .map(|request| request.id.clone());

            let Some(request_id) = request_id else {
                return Ok(None);
            };
            // Validate the current target before mutating the lease row.
            // Promotion or retirement may have changed after the admin queued
            // the request. A permanently invalid target is terminal queue work,
            // not an error that may poison the oldest-row scan forever.
            let pending = self
                .runtime_control_requests
                .get(&request_id)
                .cloned()
                .ok_or(CoreError::RuntimeControlRequestNotFound)?;
            let runtime = self
                .agent_runtimes
                .get(&pending.agent_runtime_id)
                .cloned()
                .ok_or(CoreError::ProjectRuntimeNotFound)?;
            let target_result = if pending.kind == RuntimeControlKind::Upgrade {
                pending
                    .target_runtime_artifact_id
                    .as_deref()
                    .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)
                    .and_then(|target_id| {
                        self.compatible_runtime_upgrade_artifact(&runtime, target_id)
                    })
                    .map(Some)
            } else {
                Ok(None)
            };
            let target_runtime_artifact = match target_result {
                Ok(target) => target,
                Err(error) if runtime_upgrade_prelease_rejection_is_terminal(&error) => {
                    let request = self
                        .runtime_control_requests
                        .get_mut(&request_id)
                        .ok_or(CoreError::RuntimeControlRequestNotFound)?;
                    request.status = RuntimeControlRequestStatus::Failed;
                    request.runner_id = None;
                    request.lease_token = None;
                    request.lease_expires_at = None;
                    request.failure_message = Some(format!(
                        "runtime upgrade target rejected before lease: {error}"
                    ));
                    request.updated_at = now.clone();
                    request.completed_at = Some(now.clone());
                    continue;
                }
                Err(error) => return Err(error),
            };
            let request = {
                let Some(request) = self.runtime_control_requests.get_mut(&request_id) else {
                    return Err(CoreError::RuntimeControlRequestNotFound);
                };
                request.status = RuntimeControlRequestStatus::Running;
                request.runner_id = Some(runner_id.clone());
                request.lease_token = Some(lease_token.clone());
                request.lease_expires_at = Some(lease_expires_at.clone());
                request.failure_message = None;
                request.updated_at = now.clone();
                request.clone()
            };
            return Ok(Some(RuntimeControlLease {
                request,
                runtime,
                target_runtime_artifact,
            }));
        }
    }

    pub fn complete_runtime_control_request(
        &mut self,
        input: CompleteRuntimeControlRequestInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let verified = self.verified_runtime_control_request(
            &input.request_id,
            &input.runner_id,
            &input.lease_token,
        )?;
        let upgrade_facts = if verified.kind == RuntimeControlKind::Upgrade {
            let target_id = verified
                .target_runtime_artifact_id
                .as_deref()
                .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?;
            let reported_id = trim_to_option(input.runtime_artifact_id.as_deref())
                .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?;
            let runtime = self
                .agent_runtimes
                .get(&verified.agent_runtime_id)
                .ok_or(CoreError::ProjectRuntimeNotFound)?;
            let target = self
                .runtime_artifacts
                .get(target_id)
                .cloned()
                .ok_or(CoreError::RuntimeArtifactNotFound)?;
            // Retirement after lease must not strand Core behind a target the
            // runner has already atomically swapped into place. Material is
            // immutable, so completion verifies exact identity/schema but does
            // not reapply request-time lifecycle policy.
            self.ensure_runtime_upgrade_artifact_material(runtime, &target)?;
            let reported_schema = trim_to_option(input.state_schema_version.as_deref())
                .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?;
            let runtime_host = trim_to_option(input.runtime_host.as_deref())
                .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?;
            let published_app_urls = input
                .published_app_urls
                .clone()
                .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?;
            if reported_id != target.id || reported_schema != target.state_schema_version {
                return Err(CoreError::RuntimeUpgradeCompletionMismatch);
            }
            Some((
                reported_id,
                reported_schema,
                runtime_host,
                published_app_urls,
            ))
        } else {
            if input.runtime_artifact_id.is_some()
                || input.state_schema_version.is_some()
                || input.runtime_host.is_some()
                || input.published_app_urls.is_some()
            {
                return Err(CoreError::RuntimeUpgradeCompletionMismatch);
            }
            None
        };
        let request = {
            let Some(request) = self.runtime_control_requests.get_mut(&input.request_id) else {
                return Err(CoreError::RuntimeControlRequestNotFound);
            };
            request.status = RuntimeControlRequestStatus::Succeeded;
            request.lease_token = None;
            request.lease_expires_at = None;
            request.failure_message = None;
            request.updated_at = now.clone();
            request.completed_at = Some(now.clone());
            request.clone()
        };
        let completed_status = match request.kind {
            RuntimeControlKind::Restart
            | RuntimeControlKind::RecoverKnownGoodChatRuntime
            | RuntimeControlKind::Upgrade => RuntimeSummaryStatus::Online,
            RuntimeControlKind::Stop | RuntimeControlKind::Destroy => RuntimeSummaryStatus::Offline,
        };
        if let Some(runtime) = self.agent_runtimes.get_mut(&request.agent_runtime_id) {
            runtime.host_facts.runtime_status = completed_status;
            if let Some((artifact_id, schema, runtime_host, published_app_urls)) =
                upgrade_facts.as_ref()
            {
                runtime.runtime_artifact_id = Some(artifact_id.clone());
                runtime.state_schema_version = Some(schema.clone());
                runtime.host_facts.runtime_host = runtime_host.clone();
                runtime.host_facts.published_app_urls = published_app_urls.clone();
                runtime.host_facts.hermes_available = Some(true);
            }
            if request.kind == RuntimeControlKind::Destroy {
                runtime.host_facts.hermes_available = Some(false);
                runtime.host_facts.published_app_urls.clear();
            }
            runtime.updated_at = now.clone();
        }
        if let Some(snapshot) = self
            .runtime_status_snapshots
            .get_mut(&request.agent_runtime_id)
        {
            snapshot.status = completed_status;
            if let Some((_, _, runtime_host, _)) = upgrade_facts.as_ref() {
                snapshot.runtime_host = runtime_host.clone();
                snapshot.hermes_available = Some(true);
            }
            if request.kind == RuntimeControlKind::Destroy {
                snapshot.hermes_available = Some(false);
            }
            snapshot.updated_at = now;
        }
        if request.kind == RuntimeControlKind::Destroy {
            self.offboard_destroyed_runtime(&request);
        }
        Ok(request)
    }

    pub fn fail_runtime_control_request(
        &mut self,
        input: FailRuntimeControlRequestInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let failure_message = trim_to_option(Some(&input.failure_message))
            .ok_or(CoreError::MissingRuntimeControlFailureMessage)?;
        self.verified_runtime_control_request(
            &input.request_id,
            &input.runner_id,
            &input.lease_token,
        )?;
        let Some(request) = self.runtime_control_requests.get_mut(&input.request_id) else {
            return Err(CoreError::RuntimeControlRequestNotFound);
        };
        request.status = RuntimeControlRequestStatus::Failed;
        request.lease_token = None;
        request.lease_expires_at = None;
        request.failure_message = Some(failure_message);
        request.updated_at = now.clone();
        request.completed_at = Some(now.clone());
        if let Some(runtime) = self.agent_runtimes.get_mut(&request.agent_runtime_id) {
            runtime.host_facts.runtime_status = RuntimeSummaryStatus::Stale;
            runtime.updated_at = now.clone();
        }
        if let Some(snapshot) = self
            .runtime_status_snapshots
            .get_mut(&request.agent_runtime_id)
        {
            snapshot.status = RuntimeSummaryStatus::Stale;
            snapshot.updated_at = now;
        }
        Ok(request.clone())
    }

    pub fn register_agent_creation_runtime(
        &mut self,
        input: RegisterAgentCreationRuntimeInput,
    ) -> CoreResult<AgentCreationLease> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let source_host_id = normalize_source_host_id(&input.source_host_id)?;
        let source_machine_id = normalize_id_part(&input.source_machine_id);
        if source_machine_id.is_empty() {
            return Err(CoreError::MissingSourceMachineId);
        }
        let token_hash = trim_to_option(Some(&input.runtime_relay_token_hash))
            .ok_or(CoreError::MissingRuntimeRelayTokenHash)?;
        let artifact_id = trim_to_option(input.runtime_artifact_id.as_deref())
            .ok_or(CoreError::MissingRuntimeArtifactId)?;
        let artifact = self.launchable_runtime_artifact(&artifact_id)?;
        let state_schema_version = trim_to_option(input.state_schema_version.as_deref())
            .unwrap_or_else(|| artifact.state_schema_version.clone());
        let request = self.verified_launching_request(
            &input.request_id,
            &input.runner_id,
            &input.lease_token,
        )?;
        let project = self
            .projects
            .get(&request.project_id)
            .cloned()
            .ok_or_else(|| {
                CoreError::Store(format!(
                    "agent creation request {} references missing project {}",
                    request.id, request.project_id
                ))
            })?;
        let source_import_key = source_import_key(&source_host_id, &source_machine_id);
        if self.agent_runtimes.values().any(|runtime| {
            runtime.source_import_key == source_import_key && runtime.project_id != project.id
        }) {
            return Err(CoreError::Store(format!(
                "runtime source {source_import_key} is already attached to another project"
            )));
        }

        // Resolve the runtime by its natural key (source_import_key is UNIQUE):
        // reuse the existing surrogate id when the source is already known, mint
        // a fresh one otherwise. The id is never derived from the source.
        let runtime_id = match self.find_agent_runtime_by_source_import_key(&source_import_key) {
            Some(existing) => existing.id,
            None => new_agent_runtime_id()?,
        };
        let host_facts = HostOwnedRuntimeFacts {
            display_name: trim_to_option(input.display_name.as_deref())
                .unwrap_or_else(|| request.display_name.clone()),
            hostname: trim_to_option(input.hostname.as_deref()),
            runtime_host: trim_to_option(input.runtime_host.as_deref())
                .unwrap_or_else(|| source_host_id.clone()),
            runtime_status: input
                .runtime_status
                .unwrap_or(RuntimeSummaryStatus::Unknown),
            active_inference_profile: trim_to_option(input.active_inference_profile.as_deref()),
            hermes_available: input.hermes_available,
            published_app_urls: input.published_app_urls,
        };
        let runtime = AgentRuntime {
            id: runtime_id.clone(),
            project_id: project.id.clone(),
            source_host_id,
            source_machine_id,
            source_import_key,
            runtime_artifact_id: Some(artifact.id),
            state_schema_version: Some(state_schema_version),
            host_facts,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        self.agent_runtimes
            .insert(runtime.id.clone(), runtime.clone());
        self.runtime_relay_credentials.insert(
            runtime_id.clone(),
            RuntimeRelayCredential {
                agent_runtime_id: runtime_id.clone(),
                token_hash,
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        );

        for link in self
            .project_runtime_links
            .values_mut()
            .filter(|link| link.project_id == project.id)
        {
            link.active = false;
        }
        let link = ProjectRuntimeLink {
            id: project_runtime_link_id_for(&project.id, &runtime_id),
            project_id: project.id.clone(),
            agent_runtime_id: runtime_id.clone(),
            active: true,
            created_at: now.clone(),
        };
        self.project_runtime_links.insert(link.id.clone(), link);

        let Some(request) = self.agent_creation_requests.get_mut(&input.request_id) else {
            return Err(CoreError::AgentCreationRequestNotFound);
        };
        request.agent_runtime_id = Some(runtime_id);
        request.failure_message = None;
        request.updated_at = now;

        Ok(AgentCreationLease {
            project,
            request: request.clone(),
        })
    }

    pub fn complete_agent_creation_request(
        &mut self,
        input: CompleteAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationLease> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let source_host_id = normalize_source_host_id(&input.source_host_id)?;
        let source_machine_id = normalize_id_part(&input.source_machine_id);
        if source_machine_id.is_empty() {
            return Err(CoreError::MissingSourceMachineId);
        }
        let request = self.verified_launching_request(
            &input.request_id,
            &input.runner_id,
            &input.lease_token,
        )?;
        let existing_runtime = request
            .agent_runtime_id
            .as_ref()
            .and_then(|runtime_id| self.agent_runtimes.get(runtime_id))
            .cloned();
        let artifact_id = trim_to_option(input.runtime_artifact_id.as_deref())
            .or_else(|| existing_runtime.as_ref()?.runtime_artifact_id.clone())
            .ok_or(CoreError::MissingRuntimeArtifactId)?;
        let artifact = self.launchable_runtime_artifact(&artifact_id)?;
        let state_schema_version = trim_to_option(input.state_schema_version.as_deref())
            .or_else(|| existing_runtime.as_ref()?.state_schema_version.clone())
            .unwrap_or_else(|| artifact.state_schema_version.clone());
        let project = self
            .projects
            .get(&request.project_id)
            .cloned()
            .ok_or_else(|| {
                CoreError::Store(format!(
                    "agent creation request {} references missing project {}",
                    request.id, request.project_id
                ))
            })?;
        let source_import_key = source_import_key(&source_host_id, &source_machine_id);
        if self.agent_runtimes.values().any(|runtime| {
            runtime.source_import_key == source_import_key && runtime.project_id != project.id
        }) {
            return Err(CoreError::Store(format!(
                "runtime source {source_import_key} is already attached to another project"
            )));
        }

        // Reuse the runtime already known for this source (registered earlier or
        // resolved by its UNIQUE source_import_key); mint a fresh surrogate id
        // only for a source we have never seen.
        let runtime_id = match self.find_agent_runtime_by_source_import_key(&source_import_key) {
            Some(existing) => existing.id,
            None => new_agent_runtime_id()?,
        };
        let host_facts = HostOwnedRuntimeFacts {
            display_name: trim_to_option(input.display_name.as_deref())
                .unwrap_or_else(|| request.display_name.clone()),
            hostname: trim_to_option(input.hostname.as_deref()),
            runtime_host: trim_to_option(input.runtime_host.as_deref())
                .unwrap_or_else(|| source_host_id.clone()),
            runtime_status: input
                .runtime_status
                .unwrap_or(RuntimeSummaryStatus::Unknown),
            active_inference_profile: trim_to_option(input.active_inference_profile.as_deref()),
            hermes_available: input.hermes_available,
            published_app_urls: input.published_app_urls,
        };
        let runtime = AgentRuntime {
            id: runtime_id.clone(),
            project_id: project.id.clone(),
            source_host_id,
            source_machine_id,
            source_import_key,
            runtime_artifact_id: Some(artifact.id),
            state_schema_version: Some(state_schema_version),
            host_facts,
            created_at: existing_runtime
                .map(|runtime| runtime.created_at)
                .unwrap_or_else(|| now.clone()),
            updated_at: now.clone(),
        };
        self.agent_runtimes
            .insert(runtime.id.clone(), runtime.clone());

        for link in self
            .project_runtime_links
            .values_mut()
            .filter(|link| link.project_id == project.id)
        {
            link.active = false;
        }
        let link = ProjectRuntimeLink {
            id: project_runtime_link_id_for(&project.id, &runtime_id),
            project_id: project.id.clone(),
            agent_runtime_id: runtime_id.clone(),
            active: true,
            created_at: now.clone(),
        };
        self.project_runtime_links.insert(link.id.clone(), link);

        let Some(request) = self.agent_creation_requests.get_mut(&input.request_id) else {
            return Err(CoreError::AgentCreationRequestNotFound);
        };
        request.status = AgentCreationRequestStatus::Running;
        request.agent_runtime_id = Some(runtime_id);
        request.lease_token = None;
        request.lease_expires_at = None;
        request.failure_message = None;
        request.updated_at = now;

        Ok(AgentCreationLease {
            project,
            request: request.clone(),
        })
    }

    pub fn fail_agent_creation_request(
        &mut self,
        input: FailAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationRequest> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let failure_message = trim_to_option(Some(&input.failure_message))
            .ok_or(CoreError::MissingAgentCreationFailureMessage)?;
        self.verified_launching_request(&input.request_id, &input.runner_id, &input.lease_token)?;
        let provisional_runtime_id = self
            .agent_creation_requests
            .get(&input.request_id)
            .and_then(|request| request.agent_runtime_id.clone());
        let Some(request) = self.agent_creation_requests.get_mut(&input.request_id) else {
            return Err(CoreError::AgentCreationRequestNotFound);
        };
        request.status = AgentCreationRequestStatus::Failed;
        request.agent_runtime_id = None;
        request.lease_token = None;
        request.lease_expires_at = None;
        request.failure_message = Some(failure_message);
        request.updated_at = now;
        if let Some(runtime_id) = provisional_runtime_id {
            self.agent_runtimes.remove(&runtime_id);
            self.runtime_relay_credentials.remove(&runtime_id);
            self.runtime_status_snapshots.remove(&runtime_id);
            self.project_runtime_links
                .retain(|_, link| link.agent_runtime_id != runtime_id);
        }
        Ok(request.clone())
    }

    pub fn cancel_agent_creation_request(
        &mut self,
        input: CancelAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationRequest> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let provisional_runtime_id = self
            .agent_creation_requests
            .get(&input.request_id)
            .ok_or(CoreError::AgentCreationRequestNotFound)
            .and_then(|request| match request.status {
                AgentCreationRequestStatus::Running => {
                    Err(CoreError::AgentCreationRequestNotCancellable)
                }
                AgentCreationRequestStatus::Requested
                | AgentCreationRequestStatus::Launching
                | AgentCreationRequestStatus::Failed
                | AgentCreationRequestStatus::Cancelled => Ok(request.agent_runtime_id.clone()),
            })?;

        let Some(request) = self.agent_creation_requests.get_mut(&input.request_id) else {
            return Err(CoreError::AgentCreationRequestNotFound);
        };
        request.status = AgentCreationRequestStatus::Cancelled;
        request.agent_runtime_id = None;
        request.runner_id = None;
        request.lease_token = None;
        request.lease_expires_at = None;
        request.failure_message = None;
        request.updated_at = now;
        if let Some(runtime_id) = provisional_runtime_id {
            self.agent_runtimes.remove(&runtime_id);
            self.runtime_relay_credentials.remove(&runtime_id);
            self.runtime_status_snapshots.remove(&runtime_id);
            self.project_runtime_links
                .retain(|_, link| link.agent_runtime_id != runtime_id);
        }
        Ok(request.clone())
    }

    pub fn record_runtime_heartbeat(&mut self, relay_token: &str) -> CoreResult<RelayHeartbeat> {
        let now = current_time_iso()?;
        let token_hash = hash_runtime_relay_token(relay_token)?;
        let credential = self
            .runtime_relay_credentials
            .values()
            .find(|credential| credential.token_hash == token_hash)
            .cloned()
            .ok_or(CoreError::InvalidRuntimeRelayToken)?;
        let runtime = self
            .agent_runtimes
            .get_mut(&credential.agent_runtime_id)
            .ok_or_else(|| {
                CoreError::Store(format!(
                    "runtime relay credential references missing runtime {}",
                    credential.agent_runtime_id
                ))
            })?;
        runtime.host_facts.runtime_status = RuntimeSummaryStatus::Online;
        runtime.updated_at = now.clone();
        self.runtime_status_snapshots.insert(
            runtime.id.clone(),
            RuntimeStatusSnapshot {
                agent_runtime_id: runtime.id.clone(),
                status: RuntimeSummaryStatus::Online,
                last_heartbeat_at: Some(now.clone()),
                runtime_host: runtime.host_facts.runtime_host.clone(),
                active_inference_profile: runtime.host_facts.active_inference_profile.clone(),
                hermes_available: runtime.host_facts.hermes_available,
                updated_at: now.clone(),
            },
        );
        Ok(RelayHeartbeat {
            ok: true,
            machine_id: runtime.source_machine_id.clone(),
            last_seen_at: now,
        })
    }

    pub fn relay_events_for_runtime(&self, relay_token: &str) -> CoreResult<RelayEventsOutput> {
        let token_hash = hash_runtime_relay_token(relay_token)?;
        let credential = self
            .runtime_relay_credentials
            .values()
            .find(|credential| credential.token_hash == token_hash)
            .ok_or(CoreError::InvalidRuntimeRelayToken)?;
        let runtime = self
            .agent_runtimes
            .get(&credential.agent_runtime_id)
            .ok_or_else(|| {
                CoreError::Store(format!(
                    "runtime relay credential references missing runtime {}",
                    credential.agent_runtime_id
                ))
            })?;
        Ok(RelayEventsOutput {
            machine_id: runtime.source_machine_id.clone(),
            events: Vec::new(),
        })
    }

    pub fn runtime_heartbeat_for_machine(
        &self,
        source_machine_id: &str,
    ) -> CoreResult<RelayHeartbeat> {
        let source_machine_id = normalize_id_part(source_machine_id);
        if source_machine_id.is_empty() {
            return Err(CoreError::MissingSourceMachineId);
        }
        let runtime = self
            .agent_runtimes
            .values()
            .find(|runtime| runtime.source_machine_id == source_machine_id)
            .ok_or(CoreError::RuntimeHeartbeatNotFound)?;
        let snapshot = self
            .runtime_status_snapshots
            .get(&runtime.id)
            .filter(|snapshot| snapshot.status == RuntimeSummaryStatus::Online)
            .and_then(|snapshot| snapshot.last_heartbeat_at.as_ref())
            .ok_or(CoreError::RuntimeHeartbeatNotFound)?;
        Ok(RelayHeartbeat {
            ok: true,
            machine_id: runtime.source_machine_id.clone(),
            last_seen_at: snapshot.clone(),
        })
    }

    pub fn claimable_candidates_for_email(
        &self,
        email: Option<&str>,
    ) -> Vec<ProjectImportCandidate> {
        let Some(normalized) = normalize_owner_email(email) else {
            return Vec::new();
        };

        self.project_import_candidates
            .values()
            .filter(|candidate| {
                candidate.status == ImportCandidateStatus::Pending
                    && candidate.owner_email == normalized
            })
            .cloned()
            .collect()
    }

    fn agent_creation_request_is_leasable(
        &self,
        request: &AgentCreationRequest,
        now: OffsetDateTime,
    ) -> bool {
        match request.status {
            AgentCreationRequestStatus::Requested => true,
            AgentCreationRequestStatus::Launching => request
                .lease_expires_at
                .as_deref()
                .and_then(|value| parse_time(value).ok())
                .is_none_or(|lease_expires_at| lease_expires_at <= now),
            AgentCreationRequestStatus::Running
            | AgentCreationRequestStatus::Failed
            | AgentCreationRequestStatus::Cancelled => false,
        }
    }

    fn runtime_control_request_is_leasable(
        &self,
        request: &RuntimeControlRequest,
        now: OffsetDateTime,
    ) -> bool {
        match request.status {
            RuntimeControlRequestStatus::Requested => true,
            RuntimeControlRequestStatus::Running => request
                .lease_expires_at
                .as_deref()
                .and_then(|value| parse_time(value).ok())
                .is_none_or(|lease_expires_at| lease_expires_at <= now),
            RuntimeControlRequestStatus::Succeeded | RuntimeControlRequestStatus::Failed => false,
        }
    }

    fn verified_launching_request(
        &self,
        request_id: &str,
        runner_id: &str,
        lease_token: &str,
    ) -> CoreResult<AgentCreationRequest> {
        let runner_id =
            trim_to_option(Some(runner_id)).ok_or(CoreError::MissingAgentCreationRunnerId)?;
        let lease_token =
            trim_to_option(Some(lease_token)).ok_or(CoreError::MissingAgentCreationLeaseToken)?;
        let request = self
            .agent_creation_requests
            .get(request_id)
            .cloned()
            .ok_or(CoreError::AgentCreationRequestNotFound)?;
        if request.status != AgentCreationRequestStatus::Launching {
            return Err(CoreError::AgentCreationRequestNotLaunching);
        }
        if request.runner_id.as_deref() != Some(runner_id.as_str())
            || request.lease_token.as_deref() != Some(lease_token.as_str())
        {
            return Err(CoreError::AgentCreationRequestLeaseConflict);
        }
        Ok(request)
    }

    fn verified_runtime_control_request(
        &self,
        request_id: &str,
        runner_id: &str,
        lease_token: &str,
    ) -> CoreResult<RuntimeControlRequest> {
        let runner_id =
            trim_to_option(Some(runner_id)).ok_or(CoreError::MissingAgentCreationRunnerId)?;
        let lease_token =
            trim_to_option(Some(lease_token)).ok_or(CoreError::MissingAgentCreationLeaseToken)?;
        let request = self
            .runtime_control_requests
            .get(request_id)
            .cloned()
            .ok_or(CoreError::RuntimeControlRequestNotFound)?;
        if request.status != RuntimeControlRequestStatus::Running {
            return Err(CoreError::RuntimeControlRequestNotRunning);
        }
        if request.runner_id.as_deref() != Some(runner_id.as_str())
            || request.lease_token.as_deref() != Some(lease_token.as_str())
        {
            return Err(CoreError::RuntimeControlRequestLeaseConflict);
        }
        Ok(request)
    }

    fn active_runtime_for_project(&self, project_id: &str) -> Option<AgentRuntime> {
        self.project_runtime_links
            .values()
            .find(|link| link.project_id == project_id && link.active)
            .and_then(|link| self.agent_runtimes.get(&link.agent_runtime_id))
            .cloned()
    }

    pub fn visible_projects_for_user(&self, user_id: &str) -> Vec<Project> {
        let Some(identity) = self
            .chat_identities
            .values()
            .find(|identity| identity.user_id == user_id)
        else {
            return Vec::new();
        };

        let project_ids = self
            .project_room_memberships
            .values()
            .filter(|membership| membership.chat_identity_id == identity.id)
            .map(|membership| membership.project_id.as_str())
            .collect::<BTreeSet<_>>();

        self.projects
            .values()
            .filter(|project| project_ids.contains(project.id.as_str()))
            .filter(|project| !self.project_has_hidden_cancelled_creation_request(&project.id))
            .cloned()
            .collect()
    }

    fn project_has_hidden_cancelled_creation_request(&self, project_id: &str) -> bool {
        self.agent_creation_requests.values().any(|request| {
            request.project_id == project_id
                && request.status == AgentCreationRequestStatus::Cancelled
                && request.agent_runtime_id.is_none()
        })
    }

    fn update_existing_candidate(
        &mut self,
        candidate_id: &str,
        latest_host_owner_email: &str,
        record: &ExistingHostProjectImport,
        now: &str,
    ) {
        let runtime_id =
            if let Some(candidate) = self.project_import_candidates.get_mut(candidate_id) {
                candidate.latest_host_owner_email = Some(latest_host_owner_email.to_string());
                candidate.host_facts = host_facts_from_record(record);
                candidate.known_external_channel_participants =
                    record.known_external_channel_participants.clone();
                candidate.updated_at = now.to_string();
                candidate.agent_runtime_id.clone()
            } else {
                None
            };

        if let Some(runtime_id) = runtime_id
            && let Some(runtime) = self.agent_runtimes.get_mut(&runtime_id)
        {
            runtime.host_facts = host_facts_from_record(record);
            runtime.updated_at = now.to_string();
        }
    }

    fn find_user_by_email(&self, email: &str) -> Option<CoreUser> {
        self.users
            .values()
            .find(|user| user.email == email)
            .cloned()
    }

    fn find_personal_org_by_owner(&self, owner_user_id: &str) -> Option<CustomerOrganization> {
        self.customer_orgs
            .values()
            .find(|org| org.owner_user_id == owner_user_id)
            .cloned()
    }

    fn find_agent_creation_request_by_idempotency(
        &self,
        owner_user_id: &str,
        idempotency_key: &str,
    ) -> Option<AgentCreationRequest> {
        self.agent_creation_requests
            .values()
            .find(|request| {
                request.owner_user_id == owner_user_id && request.idempotency_key == idempotency_key
            })
            .cloned()
    }

    fn find_agent_runtime_by_source_import_key(
        &self,
        source_import_key: &str,
    ) -> Option<AgentRuntime> {
        self.agent_runtimes
            .values()
            .find(|runtime| runtime.source_import_key == source_import_key)
            .cloned()
    }

    fn ensure_pending_user(&mut self, email: &str, now: &str) -> CoreResult<CoreUser> {
        // Natural-key lookup by email replaces the old `user_id = f(email)`
        // derivation: a wiped+recreated account gets a fresh surrogate id and
        // cannot collide with the previous account's orphaned rows.
        if let Some(existing) = self.find_user_by_email(email) {
            return Ok(existing);
        }

        let id = new_user_id()?;
        let user = CoreUser {
            id: id.clone(),
            email: email.to_string(),
            status: UserLinkStatus::Pending,
            workos_user_id: None,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        };
        self.users.insert(id, user.clone());
        Ok(user)
    }

    fn ensure_linked_user(
        &mut self,
        email: &str,
        workos_user_id: &str,
        now: &str,
    ) -> CoreResult<CoreUser> {
        self.ensure_linked_user_with_billing_class(
            email,
            workos_user_id,
            BillingClass::Grandfathered,
            now,
        )
    }

    fn ensure_linked_user_with_billing_class(
        &mut self,
        email: &str,
        workos_user_id: &str,
        billing_class: BillingClass,
        now: &str,
    ) -> CoreResult<CoreUser> {
        let pending = self.ensure_pending_user(email, now)?;
        if self.users.values().any(|user| {
            user.id != pending.id && user.workos_user_id.as_deref() == Some(workos_user_id)
        }) {
            return Err(CoreError::WorkosUserConflict);
        }

        let user = CoreUser {
            status: UserLinkStatus::Linked,
            workos_user_id: Some(workos_user_id.to_string()),
            updated_at: now.to_string(),
            ..pending
        };
        self.users.insert(user.id.clone(), user.clone());
        self.ensure_personal_org(&user, billing_class, now)?;
        Ok(user)
    }

    fn ensure_personal_org(
        &mut self,
        user: &CoreUser,
        billing_class: BillingClass,
        now: &str,
    ) -> CoreResult<CustomerOrganization> {
        // One personal org per owner: look it up by owner_user_id (the DB carries
        // a matching unique index) and mint a fresh surrogate id only on insert.
        if let Some(existing) = self.find_personal_org_by_owner(&user.id) {
            return Ok(existing);
        }

        let id = new_customer_org_id()?;
        let org = CustomerOrganization {
            id: id.clone(),
            owner_user_id: user.id.clone(),
            name: user.email.clone(),
            billing_class,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        };
        self.customer_orgs.insert(id, org.clone());
        Ok(org)
    }

    fn ensure_hosted_web_membership(&mut self, user: &CoreUser, project_id: &str, now: &str) {
        let identity_id = chat_identity_id_for_user(&user.id);
        self.chat_identities
            .entry(identity_id.clone())
            .or_insert_with(|| ChatIdentity {
                id: identity_id.clone(),
                user_id: user.id.clone(),
                kind: "hosted_web".to_string(),
                device_id: "dashboard-bridge-v1".to_string(),
                created_at: now.to_string(),
            });

        let membership_id = project_room_membership_id_for(project_id, &identity_id);
        self.project_room_memberships
            .entry(membership_id.clone())
            .or_insert_with(|| ProjectRoomMembership {
                id: membership_id,
                project_id: project_id.to_string(),
                chat_identity_id: identity_id,
                role: ProjectMembershipRole::Owner,
                created_at: now.to_string(),
            });
    }

    fn ensure_agent_creation_entitlement(
        &mut self,
        customer_org_id: &str,
        launch_code: Option<&str>,
        now: &str,
    ) -> CoreResult<AgentCreationEntitlement> {
        if let Some(code) = launch_code {
            self.validate_agent_creation_launch_code(customer_org_id, code)?;
        } else if !self.customer_org_has_active_billing(customer_org_id) {
            return Err(CoreError::BillingRequired);
        }

        if let Some(existing) = self
            .agent_creation_entitlements
            .values()
            .find(|entitlement| entitlement.customer_org_id == customer_org_id)
            .cloned()
        {
            return Ok(existing);
        }

        Ok(self.upsert_agent_creation_entitlement(
            customer_org_id,
            1,
            launch_code.map(|_| FIRST_SELF_SERVE_LAUNCH_CODE.to_string()),
            now,
        ))
    }

    fn validate_agent_creation_launch_code(
        &self,
        customer_org_id: &str,
        launch_code: &str,
    ) -> CoreResult<()> {
        if let Some(existing) = self
            .agent_creation_entitlements
            .values()
            .find(|entitlement| entitlement.customer_org_id == customer_org_id)
        {
            if let Some(expected_code) = existing.launch_code.as_deref()
                && launch_code != expected_code
            {
                return Err(CoreError::InvalidLaunchCode);
            }
            return Ok(());
        }

        if launch_code == FIRST_SELF_SERVE_LAUNCH_CODE {
            Ok(())
        } else {
            Err(CoreError::InvalidLaunchCode)
        }
    }

    fn upsert_agent_creation_entitlement(
        &mut self,
        customer_org_id: &str,
        allowed_new_agent_runtimes: i32,
        launch_code: Option<String>,
        now: &str,
    ) -> AgentCreationEntitlement {
        let id = agent_creation_entitlement_id_for(customer_org_id);
        let created_at = self
            .agent_creation_entitlements
            .get(&id)
            .map(|entitlement| entitlement.created_at.clone())
            .unwrap_or_else(|| now.to_string());
        let entitlement = AgentCreationEntitlement {
            id: id.clone(),
            customer_org_id: customer_org_id.to_string(),
            allowed_new_agent_runtimes,
            launch_code,
            created_at,
            updated_at: now.to_string(),
        };
        self.agent_creation_entitlements
            .insert(id, entitlement.clone());
        entitlement
    }

    fn ensure_billing_agent_creation_entitlement(
        &mut self,
        customer_org_id: &str,
        now: &str,
    ) -> AgentCreationEntitlement {
        let id = agent_creation_entitlement_id_for(customer_org_id);
        if let Some(existing) = self.agent_creation_entitlements.get_mut(&id) {
            existing.allowed_new_agent_runtimes = existing.allowed_new_agent_runtimes.max(1);
            existing.updated_at = now.to_string();
            return existing.clone();
        }
        self.upsert_agent_creation_entitlement(customer_org_id, 1, None, now)
    }

    fn active_agent_creation_entitlement_count(&self, customer_org_id: &str) -> i32 {
        let active_runtime_count = self
            .project_runtime_links
            .values()
            .filter(|link| link.active)
            .filter_map(|link| self.projects.get(&link.project_id))
            .filter(|project| {
                project.customer_org_id == customer_org_id && project.import_candidate_id.is_none()
            })
            .count();
        let pending_request_count = self
            .agent_creation_requests
            .values()
            .filter(|request| {
                request.customer_org_id == customer_org_id
                    && matches!(
                        request.status,
                        AgentCreationRequestStatus::Requested
                            | AgentCreationRequestStatus::Launching
                    )
            })
            .count();
        (active_runtime_count + pending_request_count) as i32
    }

    fn customer_org_has_active_billing(&self, customer_org_id: &str) -> bool {
        self.customer_billing_accounts
            .get(customer_org_id)
            .and_then(|account| account.subscription_status)
            .is_some_and(BillingSubscriptionStatus::can_create_agent)
    }

    fn billing_overview_for_org(&self, org: &CustomerOrganization) -> BillingOverview {
        let billing_account = self.customer_billing_accounts.get(&org.id).cloned();
        let agent_creation_entitlement = self
            .agent_creation_entitlements
            .values()
            .find(|entitlement| entitlement.customer_org_id == org.id)
            .cloned();
        let can_create_agent = agent_creation_entitlement
            .as_ref()
            .is_some_and(|entitlement| {
                self.active_agent_creation_entitlement_count(&org.id)
                    < entitlement.allowed_new_agent_runtimes
            })
            && (self.customer_org_has_active_billing(&org.id)
                || org.billing_class == BillingClass::Grandfathered
                || org.billing_class == BillingClass::Off2026);
        BillingOverview {
            customer_org: org.clone(),
            billing_account,
            agent_creation_entitlement,
            can_create_agent,
            requires_billing: !self.customer_org_has_active_billing(&org.id)
                && org.billing_class == BillingClass::Standard,
        }
    }

    fn link_stripe_customer_to_org(
        &mut self,
        customer_org_id: &str,
        stripe_customer_id: &str,
        now: &str,
    ) -> CoreResult<CustomerBillingAccount> {
        if self.customer_billing_accounts.values().any(|account| {
            account.customer_org_id != customer_org_id
                && account.stripe_customer_id.as_deref() == Some(stripe_customer_id)
        }) {
            return Err(CoreError::StripeCustomerConflict);
        }
        let existing = self.customer_billing_accounts.get(customer_org_id).cloned();
        if let Some(existing_customer_id) = existing
            .as_ref()
            .and_then(|account| account.stripe_customer_id.as_deref())
            && existing_customer_id != stripe_customer_id
        {
            return Err(CoreError::StripeCustomerConflict);
        }
        let account = CustomerBillingAccount {
            customer_org_id: customer_org_id.to_string(),
            stripe_customer_id: Some(stripe_customer_id.to_string()),
            stripe_subscription_id: existing
                .as_ref()
                .and_then(|account| account.stripe_subscription_id.clone()),
            stripe_price_id: existing
                .as_ref()
                .and_then(|account| account.stripe_price_id.clone()),
            subscription_status: existing
                .as_ref()
                .and_then(|account| account.subscription_status),
            current_period_end: existing
                .as_ref()
                .and_then(|account| account.current_period_end.clone()),
            cancel_at_period_end: existing
                .as_ref()
                .is_some_and(|account| account.cancel_at_period_end),
            last_stripe_event_id: existing
                .as_ref()
                .and_then(|account| account.last_stripe_event_id.clone()),
            last_stripe_event_created: existing
                .as_ref()
                .and_then(|account| account.last_stripe_event_created),
            created_at: existing
                .as_ref()
                .map(|account| account.created_at.clone())
                .unwrap_or_else(|| now.to_string()),
            updated_at: now.to_string(),
        };
        self.customer_billing_accounts
            .insert(customer_org_id.to_string(), account.clone());
        Ok(account)
    }

    fn offboard_destroyed_runtime(&mut self, request: &RuntimeControlRequest) {
        for link in self
            .project_runtime_links
            .values_mut()
            .filter(|link| link.agent_runtime_id == request.agent_runtime_id)
        {
            link.active = false;
        }
        self.runtime_relay_credentials
            .remove(&request.agent_runtime_id);
        let mut revoked_api_key_ids = Vec::new();
        for key in self.finite_private_api_keys.values_mut().filter(|key| {
            key.agent_runtime_id.as_deref() == Some(request.agent_runtime_id.as_str())
                || key.project_id.as_deref() == Some(request.project_id.as_str())
        }) {
            if key.status == FinitePrivateApiKeyStatus::Active {
                key.status = FinitePrivateApiKeyStatus::Revoked;
                key.updated_at = request.updated_at.clone();
                revoked_api_key_ids.push(key.id.clone());
            }
        }
        if !revoked_api_key_ids.is_empty() {
            self.record_finite_private_admin_audit_event(FinitePrivateAdminAuditRecord {
                action: "finite_private.runtime.destroy_revoke_keys",
                target_type: "agent_runtime",
                target_id: &request.agent_runtime_id,
                grant_id: None,
                api_key_id: None,
                actor: None,
                metadata: json!({
                    "projectId": request.project_id,
                    "revokedApiKeyIds": revoked_api_key_ids,
                }),
                created_at: &request.updated_at,
            });
        }
    }

    pub fn source_host_relay_endpoint(
        &self,
        source_host_id: &str,
    ) -> CoreResult<Option<SourceHostRelayEndpoint>> {
        let source_host_id = normalize_source_host_id(source_host_id)?;
        Ok(self.source_host_relays.get(&source_host_id).cloned())
    }

    pub fn upsert_source_host_relay_endpoint(
        &mut self,
        input: UpsertSourceHostRelayEndpointInput,
    ) -> CoreResult<SourceHostRelayEndpoint> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let source_host_id = normalize_source_host_id(&input.source_host_id)?;
        let url = normalize_source_host_relay_url(&input.url)?;
        let admin_token = input.admin_token.trim();
        if admin_token.is_empty() {
            return Err(CoreError::MissingSourceHostRelayAdminToken);
        }

        let created_at = self
            .source_host_relays
            .get(&source_host_id)
            .map(|endpoint| endpoint.created_at.clone())
            .unwrap_or_else(|| now.clone());
        let endpoint = SourceHostRelayEndpoint {
            source_host_id: source_host_id.clone(),
            url,
            admin_token: admin_token.to_string(),
            created_at,
            updated_at: now.clone(),
        };
        self.source_host_relays
            .insert(source_host_id, endpoint.clone());
        Ok(endpoint)
    }

    pub fn approve_finite_private_grant(
        &mut self,
        input: ApproveFinitePrivateGrantInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let verified_email = normalize_owner_email(Some(&input.verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let limit_profile_id = trim_to_option(input.limit_profile_id.as_deref())
            .unwrap_or_else(|| DEFAULT_FINITE_PRIVATE_LIMIT_PROFILE.to_string());
        self.ensure_finite_private_limit_profile(&limit_profile_id, &now)?;
        let user = match trim_to_option(input.workos_user_id.as_deref()) {
            Some(workos_user_id) => {
                self.ensure_linked_user(&verified_email, &workos_user_id, &now)?
            }
            None => self.ensure_pending_user(&verified_email, &now)?,
        };
        let grant_id = finite_private_grant_id_for_user(&user.id);
        let created_at = self
            .finite_private_grants
            .get(&grant_id)
            .map(|grant| grant.created_at.clone())
            .unwrap_or_else(|| now.clone());
        let grant = FinitePrivateGrant {
            id: grant_id.clone(),
            user_id: user.id,
            limit_profile_id,
            status: FinitePrivateGrantStatus::Active,
            current_window_started_at: None,
            current_window_used_units: 0,
            created_at,
            updated_at: now.clone(),
        };
        self.finite_private_grants.insert(grant_id, grant.clone());
        self.record_finite_private_admin_audit_event(FinitePrivateAdminAuditRecord {
            action: "finite_private.grant.approve",
            target_type: "grant",
            target_id: &grant.id,
            grant_id: Some(&grant.id),
            api_key_id: None,
            actor: None,
            metadata: json!({
                "userId": grant.user_id.clone(),
                "limitProfileId": grant.limit_profile_id.clone(),
                "verifiedEmail": verified_email
            }),
            created_at: &now,
        });
        Ok(grant)
    }

    pub fn issue_finite_private_api_key(
        &mut self,
        input: IssueFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let grant_id =
            trim_to_option(Some(&input.grant_id)).ok_or(CoreError::FinitePrivateGrantNotFound)?;
        let grant = self
            .finite_private_grants
            .get(&grant_id)
            .ok_or(CoreError::FinitePrivateGrantNotFound)?;
        if grant.status != FinitePrivateGrantStatus::Active {
            return Err(CoreError::FinitePrivateGrantNotActive);
        }
        let key_hash = hash_finite_private_api_key(&input.raw_key)?;
        let key_id = finite_private_api_key_id_for(&grant_id, &key_hash);
        let created_at = self
            .finite_private_api_keys
            .get(&key_id)
            .map(|key| key.created_at.clone())
            .unwrap_or_else(|| now.clone());
        let key = FinitePrivateApiKey {
            id: key_id.clone(),
            grant_id,
            project_id: trim_to_option(input.project_id.as_deref()),
            agent_runtime_id: trim_to_option(input.agent_runtime_id.as_deref()),
            key_hash,
            status: FinitePrivateApiKeyStatus::Active,
            created_at,
            updated_at: now.clone(),
        };
        self.finite_private_api_keys.insert(key_id, key.clone());
        self.record_finite_private_admin_audit_event(FinitePrivateAdminAuditRecord {
            action: "finite_private.api_key.issue",
            target_type: "api_key",
            target_id: &key.id,
            grant_id: Some(&key.grant_id),
            api_key_id: Some(&key.id),
            actor: None,
            metadata: json!({
                "projectId": key.project_id.clone(),
                "agentRuntimeId": key.agent_runtime_id.clone()
            }),
            created_at: &now,
        });
        Ok(key)
    }

    pub fn provision_finite_private_runtime_key(
        &mut self,
        input: ProvisionFinitePrivateRuntimeKeyInput,
    ) -> CoreResult<ProvisionFinitePrivateRuntimeKeyResult> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let request = self.verified_launching_request(
            &input.request_id,
            &input.runner_id,
            &input.lease_token,
        )?;
        let project = self
            .projects
            .get(&request.project_id)
            .cloned()
            .ok_or_else(|| {
                CoreError::Store(format!(
                    "agent creation request {} references missing project {}",
                    request.id, request.project_id
                ))
            })?;
        let user = self
            .users
            .get(&request.owner_user_id)
            .cloned()
            .ok_or_else(|| {
                CoreError::Store(format!(
                    "agent creation request {} references missing owner user {}",
                    request.id, request.owner_user_id
                ))
            })?;
        let source_host_id = match input
            .source_host_id
            .as_deref()
            .and_then(|value| trim_to_option(Some(value)))
        {
            Some(value) => Some(normalize_source_host_id(&value)?),
            None => None,
        };
        let source_machine_id = match input
            .source_machine_id
            .as_deref()
            .and_then(|value| trim_to_option(Some(value)))
        {
            Some(value) => {
                let normalized = normalize_id_part(&value);
                if normalized.is_empty() {
                    return Err(CoreError::MissingSourceMachineId);
                }
                Some(normalized)
            }
            None => None,
        };
        // Resolve the runtime to attach the key to by natural key
        // (source_import_key) rather than rederiving its id from the source.
        let agent_runtime_id = match (source_host_id.as_deref(), source_machine_id.as_deref()) {
            (Some(source_host_id), Some(source_machine_id)) => self
                .find_agent_runtime_by_source_import_key(&source_import_key(
                    source_host_id,
                    source_machine_id,
                ))
                .map(|runtime| runtime.id),
            _ => request
                .agent_runtime_id
                .clone()
                .filter(|runtime_id| self.agent_runtimes.contains_key(runtime_id)),
        };

        let grant = self.approve_finite_private_grant(ApproveFinitePrivateGrantInput {
            verified_email: user.email,
            workos_user_id: user.workos_user_id,
            limit_profile_id: None,
            now: Some(now.clone()),
        })?;
        let raw_api_key = generate_finite_private_api_key()?;
        let api_key = self.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
            grant_id: grant.id.clone(),
            raw_key: raw_api_key.clone(),
            project_id: Some(project.id),
            agent_runtime_id,
            now: Some(now),
        })?;

        Ok(ProvisionFinitePrivateRuntimeKeyResult {
            grant,
            api_key,
            raw_api_key,
        })
    }

    pub fn revoke_finite_private_grant(
        &mut self,
        input: RevokeFinitePrivateGrantInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let grant_id =
            trim_to_option(Some(&input.grant_id)).ok_or(CoreError::FinitePrivateGrantNotFound)?;
        let Some(grant) = self.finite_private_grants.get_mut(&grant_id) else {
            return Err(CoreError::FinitePrivateGrantNotFound);
        };
        grant.status = FinitePrivateGrantStatus::Revoked;
        grant.updated_at = now.clone();
        let revoked_api_key_ids = self
            .finite_private_api_keys
            .values_mut()
            .filter(|key| key.grant_id == grant_id)
            .map(|key| {
                key.status = FinitePrivateApiKeyStatus::Revoked;
                key.updated_at = now.clone();
                key.id.clone()
            })
            .collect::<Vec<_>>();
        let grant = grant.clone();
        self.record_finite_private_admin_audit_event(FinitePrivateAdminAuditRecord {
            action: "finite_private.grant.revoke",
            target_type: "grant",
            target_id: &grant.id,
            grant_id: Some(&grant.id),
            api_key_id: None,
            actor: None,
            metadata: json!({ "revokedApiKeyIds": revoked_api_key_ids }),
            created_at: &now,
        });
        Ok(grant)
    }

    pub fn revoke_finite_private_api_key(
        &mut self,
        input: RevokeFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let key_id =
            trim_to_option(Some(&input.key_id)).ok_or(CoreError::InvalidFinitePrivateApiKey)?;
        let Some(key) = self.finite_private_api_keys.get_mut(&key_id) else {
            return Err(CoreError::InvalidFinitePrivateApiKey);
        };
        key.status = FinitePrivateApiKeyStatus::Revoked;
        key.updated_at = now.clone();
        let key = key.clone();
        self.record_finite_private_admin_audit_event(FinitePrivateAdminAuditRecord {
            action: "finite_private.api_key.revoke",
            target_type: "api_key",
            target_id: &key.id,
            grant_id: Some(&key.grant_id),
            api_key_id: Some(&key.id),
            actor: None,
            metadata: json!({}),
            created_at: &now,
        });
        Ok(key)
    }

    pub fn rotate_finite_private_api_key(
        &mut self,
        input: RotateFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let key_id =
            trim_to_option(Some(&input.key_id)).ok_or(CoreError::InvalidFinitePrivateApiKey)?;
        let Some(old_key) = self.finite_private_api_keys.get(&key_id).cloned() else {
            return Err(CoreError::InvalidFinitePrivateApiKey);
        };
        let new_key_hash = hash_finite_private_api_key(&input.raw_key)?;
        if new_key_hash == old_key.key_hash {
            return Err(CoreError::InvalidFinitePrivateApiKey);
        }
        let old_key_id = key_id.clone();
        let new_key = self.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
            grant_id: old_key.grant_id.clone(),
            raw_key: input.raw_key,
            project_id: old_key.project_id.clone(),
            agent_runtime_id: old_key.agent_runtime_id.clone(),
            now: Some(now.clone()),
        })?;
        self.revoke_finite_private_api_key(RevokeFinitePrivateApiKeyInput {
            key_id: old_key_id.clone(),
            now: Some(now.clone()),
        })?;
        self.record_finite_private_admin_audit_event(FinitePrivateAdminAuditRecord {
            action: "finite_private.api_key.rotate",
            target_type: "api_key",
            target_id: &new_key.id,
            grant_id: Some(&new_key.grant_id),
            api_key_id: Some(&new_key.id),
            actor: None,
            metadata: json!({ "oldApiKeyId": old_key_id }),
            created_at: &now,
        });
        Ok(new_key)
    }

    pub fn reset_finite_private_usage_window(
        &mut self,
        input: ResetFinitePrivateUsageWindowInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let grant_id =
            trim_to_option(Some(&input.grant_id)).ok_or(CoreError::FinitePrivateGrantNotFound)?;
        let Some(grant) = self.finite_private_grants.get_mut(&grant_id) else {
            return Err(CoreError::FinitePrivateGrantNotFound);
        };
        grant.current_window_started_at = None;
        grant.current_window_used_units = 0;
        grant.updated_at = now.clone();
        let grant = grant.clone();
        self.record_finite_private_admin_audit_event(FinitePrivateAdminAuditRecord {
            action: "finite_private.grant.reset_window",
            target_type: "grant",
            target_id: &grant.id,
            grant_id: Some(&grant.id),
            api_key_id: None,
            actor: None,
            metadata: json!({}),
            created_at: &now,
        });
        Ok(grant)
    }

    /// Approve (or refresh) a friend grant for a verified email and issue a
    /// Finite Private API key in one step, mirroring the
    /// `finite-private-friend-key-issue` CLI. Records an admin-attributed
    /// audit event on top of the underlying approve/issue events.
    pub fn admin_issue_finite_private_friend_key(
        &mut self,
        input: AdminIssueFinitePrivateFriendKeyInput,
    ) -> CoreResult<AdminIssuedFinitePrivateKey> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let grant = self.approve_finite_private_grant(ApproveFinitePrivateGrantInput {
            verified_email: input.friend_email,
            workos_user_id: None,
            limit_profile_id: input.limit_profile_id,
            now: Some(now.clone()),
        })?;
        let api_key = self.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
            grant_id: grant.id.clone(),
            raw_key: input.raw_key,
            project_id: None,
            agent_runtime_id: None,
            now: Some(now.clone()),
        })?;
        self.record_finite_private_admin_audit_event(FinitePrivateAdminAuditRecord {
            action: "finite_private.friend_key.admin_issue",
            target_type: "api_key",
            target_id: &api_key.id,
            grant_id: Some(&grant.id),
            api_key_id: Some(&api_key.id),
            actor: Some(&admin_email),
            metadata: json!({
                "limitProfileId": grant.limit_profile_id.clone(),
            }),
            created_at: &now,
        });
        Ok(AdminIssuedFinitePrivateKey { grant, api_key })
    }

    pub fn admin_rotate_finite_private_api_key(
        &mut self,
        input: AdminRotateFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let old_key_id = input.key_id.trim().to_string();
        let key = self.rotate_finite_private_api_key(RotateFinitePrivateApiKeyInput {
            key_id: input.key_id,
            raw_key: input.raw_key,
            now: Some(now.clone()),
        })?;
        self.record_finite_private_admin_audit_event(FinitePrivateAdminAuditRecord {
            action: "finite_private.api_key.admin_rotate",
            target_type: "api_key",
            target_id: &key.id,
            grant_id: Some(&key.grant_id),
            api_key_id: Some(&key.id),
            actor: Some(&admin_email),
            metadata: json!({ "oldApiKeyId": old_key_id }),
            created_at: &now,
        });
        Ok(key)
    }

    pub fn admin_revoke_finite_private_api_key(
        &mut self,
        input: AdminRevokeFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let key = self.revoke_finite_private_api_key(RevokeFinitePrivateApiKeyInput {
            key_id: input.key_id,
            now: Some(now.clone()),
        })?;
        self.record_finite_private_admin_audit_event(FinitePrivateAdminAuditRecord {
            action: "finite_private.api_key.admin_revoke",
            target_type: "api_key",
            target_id: &key.id,
            grant_id: Some(&key.grant_id),
            api_key_id: Some(&key.id),
            actor: Some(&admin_email),
            metadata: json!({}),
            created_at: &now,
        });
        Ok(key)
    }

    /// Reset the current burst window for a grant, mirroring the
    /// `finite-private-window-reset` CLI. Weekly limits are computed from a
    /// rolling reservation window and have no reset lever here by design.
    pub fn admin_reset_finite_private_usage_window(
        &mut self,
        input: AdminResetFinitePrivateUsageWindowInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let grant = self.reset_finite_private_usage_window(ResetFinitePrivateUsageWindowInput {
            grant_id: input.grant_id,
            now: Some(now.clone()),
        })?;
        self.record_finite_private_admin_audit_event(FinitePrivateAdminAuditRecord {
            action: "finite_private.grant.admin_window_reset",
            target_type: "grant",
            target_id: &grant.id,
            grant_id: Some(&grant.id),
            api_key_id: None,
            actor: Some(&admin_email),
            metadata: json!({}),
            created_at: &now,
        });
        Ok(grant)
    }

    /// Provisioned-boxes overview for dashboard operators, assembled from
    /// projects, agent runtimes, status snapshots, and Finite Private keys.
    pub fn admin_runtime_overviews(&self) -> Vec<AdminRuntimeOverview> {
        let mut overviews = self
            .agent_runtimes
            .values()
            .map(|runtime| {
                let project = self.projects.get(&runtime.project_id);
                let owner_email = project
                    .and_then(|project| self.users.get(&project.owner_user_id))
                    .map(|user| user.email.clone());
                let snapshot = self.runtime_status_snapshots.get(&runtime.id);
                let artifact = runtime
                    .runtime_artifact_id
                    .as_deref()
                    .and_then(|artifact_id| self.runtime_artifacts.get(artifact_id));
                let active_finite_private_key_count = self
                    .finite_private_api_keys
                    .values()
                    .filter(|key| key.status == FinitePrivateApiKeyStatus::Active)
                    .filter(|key| {
                        key.agent_runtime_id.as_deref() == Some(runtime.id.as_str())
                            || key.project_id.as_deref() == Some(runtime.project_id.as_str())
                    })
                    .count() as i64;
                let runtime_link_active = self
                    .project_runtime_links
                    .values()
                    .any(|link| link.agent_runtime_id == runtime.id && link.active);
                let supports_runtime_control = artifact
                    .map(|artifact| runtime_artifact_supports_control(artifact.kind))
                    .unwrap_or(false);
                AdminRuntimeOverview {
                    project_id: runtime.project_id.clone(),
                    project_display_name: project
                        .map(|project| project.display_name.clone())
                        .unwrap_or_else(|| runtime.host_facts.display_name.clone()),
                    owner_email,
                    agent_runtime_id: runtime.id.clone(),
                    source_host_id: runtime.source_host_id.clone(),
                    source_machine_id: runtime.source_machine_id.clone(),
                    runtime_artifact_id: runtime.runtime_artifact_id.clone(),
                    runtime_artifact_version_label: artifact
                        .map(|artifact| artifact.version_label.clone()),
                    runtime_status: snapshot
                        .map(|snapshot| snapshot.status)
                        .unwrap_or(runtime.host_facts.runtime_status),
                    last_heartbeat_at: snapshot
                        .and_then(|snapshot| snapshot.last_heartbeat_at.clone()),
                    status_updated_at: snapshot.map(|snapshot| snapshot.updated_at.clone()),
                    runtime_updated_at: runtime.updated_at.clone(),
                    hermes_available: snapshot
                        .and_then(|snapshot| snapshot.hermes_available)
                        .or(runtime.host_facts.hermes_available),
                    published_app_urls: runtime.host_facts.published_app_urls.clone(),
                    active_finite_private_key_count,
                    runtime_link_active,
                    supports_runtime_control,
                }
            })
            .collect::<Vec<_>>();
        overviews.sort_by(|left, right| {
            left.source_host_id
                .cmp(&right.source_host_id)
                .then_with(|| left.source_machine_id.cmp(&right.source_machine_id))
                .then_with(|| left.agent_runtime_id.cmp(&right.agent_runtime_id))
        });
        overviews
    }

    pub fn reserve_finite_private_usage(
        &mut self,
        input: ReserveFinitePrivateUsageInput,
    ) -> CoreResult<FinitePrivateUsageDecision> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let now_time = parse_time(&now)?;
        let request_id = trim_to_option(Some(&input.request_id))
            .unwrap_or_else(|| id_from_parts("fp_request", &[&now, &input.endpoint, &input.model]));
        let dashboard_url = trim_to_option(Some(&input.dashboard_url))
            .unwrap_or_else(|| "https://finite.computer/dashboard".to_string());
        if input.estimated_usage_units <= 0
            || input.estimated_prompt_tokens < 0
            || input.estimated_completion_tokens < 0
        {
            return Err(CoreError::InvalidFinitePrivateUsageEstimate);
        }
        let Some((api_key, grant)) = self.finite_private_key_and_grant(&input.presented_api_key)?
        else {
            return Ok(finite_private_denial(
                request_id,
                dashboard_url,
                "Finite Private API key is invalid or revoked.",
                "invalid_api_key",
                None,
                None,
            ));
        };
        let profile = self
            .finite_private_limit_profiles
            .get(&grant.limit_profile_id)
            .cloned()
            .ok_or(CoreError::FinitePrivateLimitProfileNotFound)?;

        let reservation_id = finite_private_reservation_id_for(&api_key.id, &request_id);
        let (weekly_used_units, weekly_reset_at) =
            self.finite_private_weekly_usage(&grant.id, now_time)?;
        if let Some(existing) = self.finite_private_reservations.get(&reservation_id) {
            return Ok(finite_private_allow_decision(
                existing.id.clone(),
                &profile,
                profile.burst_limit_units - grant.current_window_used_units,
                finite_private_window_reset_at(&grant, &profile, now_time)?,
                profile
                    .weekly_limit_units
                    .map(|limit| limit - weekly_used_units),
                weekly_reset_at,
            ));
        }

        let (window_started_at, current_used_units, reset_at) =
            finite_private_active_window(&grant, &profile, now_time)?;
        let remaining_before = profile.burst_limit_units - current_used_units;
        if input.estimated_usage_units > remaining_before {
            let retry_after = (parse_time(&reset_at)? - now_time).whole_seconds().max(0);
            return Ok(finite_private_denial(
                request_id,
                dashboard_url,
                "Finite Private burst window limit reached.",
                "burst_window_limit_exceeded",
                Some(retry_after),
                Some(reset_at),
            ));
        }
        if let Some(weekly_limit_units) = profile.weekly_limit_units {
            let weekly_remaining_before = weekly_limit_units - weekly_used_units;
            if input.estimated_usage_units > weekly_remaining_before {
                let reset_at = weekly_reset_at.clone().unwrap_or_else(|| {
                    (now_time + Duration::seconds(FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS))
                        .format(&Rfc3339)
                        .unwrap_or_else(|_| now.clone())
                });
                let retry_after = (parse_time(&reset_at)? - now_time).whole_seconds().max(0);
                return Ok(finite_private_denial(
                    request_id,
                    dashboard_url,
                    "Finite Private weekly limit reached.",
                    "weekly_limit_exceeded",
                    Some(retry_after),
                    Some(reset_at),
                ));
            }
        }

        let new_used_units = current_used_units + input.estimated_usage_units;
        if let Some(grant_mut) = self.finite_private_grants.get_mut(&grant.id) {
            grant_mut.current_window_started_at = Some(window_started_at);
            grant_mut.current_window_used_units = new_used_units;
            grant_mut.updated_at = now.clone();
        }
        let reservation = FinitePrivateReservation {
            id: reservation_id.clone(),
            request_id,
            api_key_id: api_key.id,
            grant_id: grant.id,
            endpoint: trim_or_fallback(&input.endpoint, "/v1/chat/completions"),
            model: trim_or_fallback(&input.model, "kimi-k2-6"),
            estimated_usage_units: input.estimated_usage_units,
            reserved_usage_units: input.estimated_usage_units,
            settled_usage_units: None,
            settlement_kind: None,
            status: FinitePrivateReservationStatus::Reserved,
            usage_formula_version: trim_or_fallback(&input.usage_formula_version, "2026-05-26.v1"),
            upstream_status: None,
            upstream_error_class: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        self.finite_private_reservations
            .insert(reservation_id.clone(), reservation);
        Ok(finite_private_allow_decision(
            reservation_id,
            &profile,
            profile.burst_limit_units - new_used_units,
            reset_at,
            profile
                .weekly_limit_units
                .map(|limit| limit - (weekly_used_units + input.estimated_usage_units)),
            weekly_reset_at.or_else(|| {
                profile.weekly_limit_units.map(|_| {
                    (now_time + Duration::seconds(FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS))
                        .format(&Rfc3339)
                        .unwrap_or_else(|_| now.clone())
                })
            }),
        ))
    }

    pub fn settle_finite_private_reservation(
        &mut self,
        input: SettleFinitePrivateReservationInput,
    ) -> CoreResult<SettleFinitePrivateReservationResult> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let reservation_id = trim_to_option(Some(&input.reservation_id))
            .ok_or(CoreError::FinitePrivateReservationNotFound)?;
        let request_id = trim_to_option(Some(&input.request_id))
            .ok_or(CoreError::FinitePrivateReservationNotFound)?;
        let Some(existing) = self
            .finite_private_reservations
            .get(&reservation_id)
            .cloned()
        else {
            return Err(CoreError::FinitePrivateReservationNotFound);
        };
        if existing.request_id != request_id {
            return Err(CoreError::FinitePrivateReservationNotFound);
        }
        if existing.status == FinitePrivateReservationStatus::Settled {
            return Err(CoreError::FinitePrivateReservationAlreadySettled);
        }
        let settled_units = input
            .usage_units
            .unwrap_or(existing.reserved_usage_units)
            .max(0);
        if let Some(grant) = self.finite_private_grants.get_mut(&existing.grant_id) {
            let delta = settled_units - existing.reserved_usage_units;
            grant.current_window_used_units = (grant.current_window_used_units + delta).max(0);
            grant.updated_at = now.clone();
        }
        let Some(reservation) = self.finite_private_reservations.get_mut(&reservation_id) else {
            return Err(CoreError::FinitePrivateReservationNotFound);
        };
        reservation.status = FinitePrivateReservationStatus::Settled;
        reservation.settled_usage_units = Some(settled_units);
        reservation.settlement_kind = Some(input.settlement);
        reservation.usage_formula_version = trim_or_fallback(
            &input.usage_formula_version,
            &reservation.usage_formula_version,
        );
        reservation.upstream_status = input.upstream_status;
        reservation.upstream_error_class = trim_to_option(input.upstream_error_class.as_deref());
        reservation.updated_at = now;
        Ok(SettleFinitePrivateReservationResult {
            settled: true,
            reservation_id,
        })
    }

    pub fn runtime_artifact(&self, id: &str) -> CoreResult<Option<RuntimeArtifact>> {
        let id = trim_to_option(Some(id)).ok_or(CoreError::MissingRuntimeArtifactId)?;
        Ok(self.runtime_artifacts.get(&id).cloned())
    }

    pub fn upsert_runtime_artifact(
        &mut self,
        input: UpsertRuntimeArtifactInput,
    ) -> CoreResult<RuntimeArtifact> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let id = trim_to_option(Some(&input.id)).ok_or(CoreError::MissingRuntimeArtifactId)?;
        let reference = trim_to_option(Some(&input.reference))
            .ok_or(CoreError::MissingRuntimeArtifactReference)?;
        let version_label = trim_to_option(Some(&input.version_label))
            .ok_or(CoreError::MissingRuntimeArtifactVersionLabel)?;
        let state_schema_version = trim_to_option(Some(&input.state_schema_version))
            .ok_or(CoreError::MissingRuntimeArtifactStateSchemaVersion)?;
        let existing = self.runtime_artifacts.get(&id).cloned();
        let created_at = existing
            .as_ref()
            .map(|artifact| artifact.created_at.clone())
            .unwrap_or_else(|| now.clone());
        let promoted_at = if input.promoted {
            existing
                .as_ref()
                .and_then(|artifact| artifact.promoted_at.clone())
                .or_else(|| Some(now.clone()))
        } else {
            existing
                .as_ref()
                .and_then(|artifact| artifact.promoted_at.clone())
        };
        let artifact = RuntimeArtifact {
            id: id.clone(),
            kind: input.kind,
            reference,
            version_label,
            source_git_sha: trim_to_option(input.source_git_sha.as_deref()),
            finitec_version: trim_to_option(input.finitec_version.as_deref()),
            hermes_source_ref: trim_to_option(input.hermes_source_ref.as_deref()),
            finite_platform_plugin_ref: trim_to_option(input.finite_platform_plugin_ref.as_deref()),
            state_schema_version,
            base_image: trim_to_option(input.base_image.as_deref()),
            created_at,
            promoted_at,
            retired_at: existing
                .as_ref()
                .and_then(|artifact| artifact.retired_at.clone()),
        };
        if let Some(existing) = existing.as_ref() {
            let referenced = self.agent_runtimes.values().any(|runtime| {
                runtime.runtime_artifact_id.as_deref() == Some(existing.id.as_str())
            });
            if (existing.promoted_at.is_some() || referenced)
                && !runtime_artifact_material_matches(existing, &artifact)
            {
                return Err(CoreError::RuntimeArtifactImmutable);
            }
        }
        self.runtime_artifacts.insert(id, artifact.clone());
        Ok(artifact)
    }

    fn launchable_runtime_artifact(&self, id: &str) -> CoreResult<RuntimeArtifact> {
        let artifact = self
            .runtime_artifacts
            .get(id)
            .cloned()
            .ok_or(CoreError::RuntimeArtifactNotFound)?;
        if artifact.promoted_at.is_none() {
            return Err(CoreError::RuntimeArtifactNotPromoted);
        }
        if artifact.retired_at.is_some() {
            return Err(CoreError::RuntimeArtifactRetired);
        }
        Ok(artifact)
    }

    fn compatible_runtime_upgrade_artifact(
        &self,
        runtime: &AgentRuntime,
        id: &str,
    ) -> CoreResult<RuntimeArtifact> {
        let artifact = self.launchable_runtime_artifact(id)?;
        self.ensure_runtime_upgrade_artifact_material(runtime, &artifact)?;
        Ok(artifact)
    }

    fn ensure_runtime_upgrade_artifact_material(
        &self,
        runtime: &AgentRuntime,
        artifact: &RuntimeArtifact,
    ) -> CoreResult<()> {
        if artifact.kind != RuntimeArtifactKind::OciImage
            || !runtime_artifact_reference_is_immutable_oci(&artifact.reference)
        {
            return Err(CoreError::RuntimeUpgradeUnsupported);
        }
        if runtime.state_schema_version.as_deref() != Some(artifact.state_schema_version.as_str()) {
            return Err(CoreError::RuntimeUpgradeStateSchemaIncompatible);
        }
        Ok(())
    }

    fn ensure_finite_private_limit_profile(
        &mut self,
        id: &str,
        now: &str,
    ) -> CoreResult<FinitePrivateLimitProfile> {
        if let Some(profile) = self.finite_private_limit_profiles.get(id).cloned() {
            return Ok(profile);
        }
        if id != DEFAULT_FINITE_PRIVATE_LIMIT_PROFILE {
            return Err(CoreError::FinitePrivateLimitProfileNotFound);
        }
        let profile = FinitePrivateLimitProfile {
            id: id.to_string(),
            burst_window_seconds: DEFAULT_FINITE_PRIVATE_BURST_WINDOW_SECONDS,
            burst_limit_units: DEFAULT_FINITE_PRIVATE_BURST_LIMIT_UNITS,
            weekly_limit_units: Some(DEFAULT_FINITE_PRIVATE_WEEKLY_LIMIT_UNITS),
            created_at: now.to_string(),
            updated_at: now.to_string(),
        };
        self.finite_private_limit_profiles
            .insert(id.to_string(), profile.clone());
        Ok(profile)
    }

    fn record_finite_private_admin_audit_event(
        &mut self,
        record: FinitePrivateAdminAuditRecord<'_>,
    ) {
        let sequence = (self.finite_private_admin_audit_events.len() + 1).to_string();
        let id = id_from_parts(
            "fp_admin_audit",
            &[
                record.action,
                record.target_type,
                record.target_id,
                record.created_at,
                &sequence,
            ],
        );
        let event = FinitePrivateAdminAuditEvent {
            id: id.clone(),
            action: record.action.to_string(),
            target_type: record.target_type.to_string(),
            target_id: record.target_id.to_string(),
            grant_id: record.grant_id.map(str::to_string),
            api_key_id: record.api_key_id.map(str::to_string),
            actor: record.actor.unwrap_or("finite-saas-core").to_string(),
            metadata: record.metadata,
            created_at: record.created_at.to_string(),
        };
        self.finite_private_admin_audit_events.insert(id, event);
    }

    fn finite_private_key_and_grant(
        &self,
        presented_api_key: &str,
    ) -> CoreResult<Option<(FinitePrivateApiKey, FinitePrivateGrant)>> {
        let key_hash = match hash_finite_private_api_key(presented_api_key) {
            Ok(hash) => hash,
            Err(CoreError::MissingFinitePrivateApiKey) => return Ok(None),
            Err(error) => return Err(error),
        };
        let Some(api_key) = self
            .finite_private_api_keys
            .values()
            .find(|key| key.key_hash == key_hash)
            .cloned()
        else {
            return Ok(None);
        };
        if api_key.status != FinitePrivateApiKeyStatus::Active {
            return Ok(None);
        }
        let Some(grant) = self.finite_private_grants.get(&api_key.grant_id).cloned() else {
            return Ok(None);
        };
        if grant.status != FinitePrivateGrantStatus::Active {
            return Ok(None);
        }
        Ok(Some((api_key, grant)))
    }

    fn finite_private_weekly_usage(
        &self,
        grant_id: &str,
        now_time: OffsetDateTime,
    ) -> CoreResult<(i64, Option<String>)> {
        let window_start = now_time - Duration::seconds(FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS);
        let mut used_units = 0;
        let mut earliest: Option<OffsetDateTime> = None;
        for reservation in self
            .finite_private_reservations
            .values()
            .filter(|reservation| reservation.grant_id == grant_id)
            .filter(|reservation| reservation.status != FinitePrivateReservationStatus::Denied)
        {
            let created_at = parse_time(&reservation.created_at)?;
            if created_at < window_start || created_at > now_time {
                continue;
            }
            used_units += reservation
                .settled_usage_units
                .unwrap_or(reservation.reserved_usage_units);
            earliest = Some(earliest.map_or(created_at, |value| value.min(created_at)));
        }
        let reset_at = earliest
            .map(|created_at| created_at + Duration::seconds(FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS))
            .map(|reset_at| reset_at.format(&Rfc3339))
            .transpose()?;
        Ok((used_units, reset_at))
    }
}

impl BillingClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grandfathered => "grandfathered",
            Self::Off2026 => "off2026",
            Self::Standard => "standard",
        }
    }
}

impl BillingSubscriptionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incomplete => "incomplete",
            Self::IncompleteExpired => "incomplete_expired",
            Self::Trialing => "trialing",
            Self::Active => "active",
            Self::PastDue => "past_due",
            Self::Canceled => "canceled",
            Self::Unpaid => "unpaid",
            Self::Paused => "paused",
        }
    }

    pub fn can_create_agent(self) -> bool {
        matches!(self, Self::Active | Self::Trialing)
    }
}

fn should_replace_stripe_subscription(
    current: Option<BillingSubscriptionStatus>,
    incoming: BillingSubscriptionStatus,
) -> bool {
    match current {
        None => true,
        Some(
            BillingSubscriptionStatus::Canceled | BillingSubscriptionStatus::IncompleteExpired,
        ) => incoming.can_create_agent(),
        Some(_) => false,
    }
}

impl ImportCandidateStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Claimed => "claimed",
            Self::AdminReview => "admin_review",
        }
    }
}

impl UserLinkStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Linked => "linked",
        }
    }
}

impl ProjectMembershipRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }
}

impl RuntimeSummaryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Stale => "stale",
            Self::Unknown => "unknown",
        }
    }
}

impl RuntimeArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OciImage => "oci_image",
        }
    }
}

impl std::str::FromStr for RuntimeArtifactKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_runtime_artifact_kind(value)
            .ok_or_else(|| format!("invalid runtime artifact kind {value}"))
    }
}

impl RuntimeControlKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Restart => "restart",
            Self::RecoverKnownGoodChatRuntime => "recover_known_good_chat_runtime",
            Self::Upgrade => "upgrade",
            Self::Stop => "stop",
            Self::Destroy => "destroy",
        }
    }
}

impl RuntimeControlRequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

impl RunnerClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalDocker => "local_docker",
            Self::AppleContainer => "apple_container",
            Self::Kata => "kata",
            Self::Phala => "phala",
            Self::Enclavia => "enclavia",
        }
    }
}

impl AgentCreationRequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Launching => "launching",
            Self::Running => "running",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl FinitePrivateGrantStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
        }
    }
}

impl FinitePrivateApiKeyStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
        }
    }
}

impl FinitePrivateReservationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Settled => "settled",
            Self::Denied => "denied",
        }
    }
}

impl FinitePrivateSettlementKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Actual => "actual",
            Self::Estimate => "estimate",
        }
    }
}

pub fn parse_runtime_artifact_kind(value: &str) -> Option<RuntimeArtifactKind> {
    match value {
        "oci_image" => Some(RuntimeArtifactKind::OciImage),
        _ => None,
    }
}

pub fn parse_runner_class(value: &str) -> Option<RunnerClass> {
    match value {
        "local_docker" => Some(RunnerClass::LocalDocker),
        "apple_container" => Some(RunnerClass::AppleContainer),
        "kata" => Some(RunnerClass::Kata),
        "phala" => Some(RunnerClass::Phala),
        "enclavia" => Some(RunnerClass::Enclavia),
        _ => None,
    }
}

pub fn parse_billing_class(value: &str) -> Option<BillingClass> {
    match value {
        "grandfathered" => Some(BillingClass::Grandfathered),
        "off2026" => Some(BillingClass::Off2026),
        "standard" => Some(BillingClass::Standard),
        _ => None,
    }
}

pub fn parse_billing_subscription_status(value: &str) -> Option<BillingSubscriptionStatus> {
    match value {
        "incomplete" => Some(BillingSubscriptionStatus::Incomplete),
        "incomplete_expired" => Some(BillingSubscriptionStatus::IncompleteExpired),
        "trialing" => Some(BillingSubscriptionStatus::Trialing),
        "active" => Some(BillingSubscriptionStatus::Active),
        "past_due" => Some(BillingSubscriptionStatus::PastDue),
        "canceled" => Some(BillingSubscriptionStatus::Canceled),
        "unpaid" => Some(BillingSubscriptionStatus::Unpaid),
        "paused" => Some(BillingSubscriptionStatus::Paused),
        _ => None,
    }
}

pub fn parse_import_candidate_status(value: &str) -> Option<ImportCandidateStatus> {
    match value {
        "pending" => Some(ImportCandidateStatus::Pending),
        "claimed" => Some(ImportCandidateStatus::Claimed),
        "admin_review" => Some(ImportCandidateStatus::AdminReview),
        _ => None,
    }
}

pub fn parse_user_link_status(value: &str) -> Option<UserLinkStatus> {
    match value {
        "pending" => Some(UserLinkStatus::Pending),
        "linked" => Some(UserLinkStatus::Linked),
        _ => None,
    }
}

pub fn parse_project_membership_role(value: &str) -> Option<ProjectMembershipRole> {
    match value {
        "owner" => Some(ProjectMembershipRole::Owner),
        "admin" => Some(ProjectMembershipRole::Admin),
        "member" => Some(ProjectMembershipRole::Member),
        _ => None,
    }
}

pub fn parse_runtime_summary_status(value: &str) -> Option<RuntimeSummaryStatus> {
    match value {
        "online" => Some(RuntimeSummaryStatus::Online),
        "offline" => Some(RuntimeSummaryStatus::Offline),
        "stale" => Some(RuntimeSummaryStatus::Stale),
        "unknown" => Some(RuntimeSummaryStatus::Unknown),
        _ => None,
    }
}

pub fn parse_agent_creation_request_status(value: &str) -> Option<AgentCreationRequestStatus> {
    match value {
        "requested" => Some(AgentCreationRequestStatus::Requested),
        "launching" => Some(AgentCreationRequestStatus::Launching),
        "running" => Some(AgentCreationRequestStatus::Running),
        "failed" => Some(AgentCreationRequestStatus::Failed),
        "cancelled" => Some(AgentCreationRequestStatus::Cancelled),
        _ => None,
    }
}

pub fn parse_runtime_control_kind(value: &str) -> Option<RuntimeControlKind> {
    match value {
        "restart" => Some(RuntimeControlKind::Restart),
        "recover_known_good_chat_runtime" => Some(RuntimeControlKind::RecoverKnownGoodChatRuntime),
        "upgrade" => Some(RuntimeControlKind::Upgrade),
        "stop" => Some(RuntimeControlKind::Stop),
        "destroy" => Some(RuntimeControlKind::Destroy),
        _ => None,
    }
}

pub fn parse_runtime_control_request_status(value: &str) -> Option<RuntimeControlRequestStatus> {
    match value {
        "requested" => Some(RuntimeControlRequestStatus::Requested),
        "running" => Some(RuntimeControlRequestStatus::Running),
        "succeeded" => Some(RuntimeControlRequestStatus::Succeeded),
        "failed" => Some(RuntimeControlRequestStatus::Failed),
        _ => None,
    }
}

pub fn parse_finite_private_grant_status(value: &str) -> Option<FinitePrivateGrantStatus> {
    match value {
        "active" => Some(FinitePrivateGrantStatus::Active),
        "revoked" => Some(FinitePrivateGrantStatus::Revoked),
        _ => None,
    }
}

pub fn parse_finite_private_api_key_status(value: &str) -> Option<FinitePrivateApiKeyStatus> {
    match value {
        "active" => Some(FinitePrivateApiKeyStatus::Active),
        "revoked" => Some(FinitePrivateApiKeyStatus::Revoked),
        _ => None,
    }
}

pub fn parse_finite_private_reservation_status(
    value: &str,
) -> Option<FinitePrivateReservationStatus> {
    match value {
        "reserved" => Some(FinitePrivateReservationStatus::Reserved),
        "settled" => Some(FinitePrivateReservationStatus::Settled),
        "denied" => Some(FinitePrivateReservationStatus::Denied),
        _ => None,
    }
}

pub fn parse_finite_private_settlement_kind(value: &str) -> Option<FinitePrivateSettlementKind> {
    match value {
        "actual" => Some(FinitePrivateSettlementKind::Actual),
        "estimate" => Some(FinitePrivateSettlementKind::Estimate),
        _ => None,
    }
}

pub fn normalize_owner_email(value: Option<&str>) -> Option<String> {
    let email = value?.trim().to_lowercase();
    if email.is_empty() { None } else { Some(email) }
}

pub fn source_import_key(source_host_id: &str, source_machine_id: &str) -> String {
    format!(
        "{}:{}",
        normalize_id_part(source_host_id),
        normalize_id_part(source_machine_id)
    )
}

pub fn normalize_source_host_id(value: &str) -> CoreResult<String> {
    let source_host_id = value.trim().to_lowercase();
    if source_host_id.is_empty() {
        return Err(CoreError::MissingSourceHostId);
    }
    if !source_host_id
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return Err(CoreError::InvalidSourceHostId);
    }
    if source_host_id.starts_with('-') || source_host_id.ends_with('-') {
        return Err(CoreError::InvalidSourceHostId);
    }
    Ok(source_host_id)
}

fn normalize_source_host_relay_url(value: &str) -> CoreResult<String> {
    let url = value.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(CoreError::InvalidSourceHostRelayUrl);
    }
    if url.contains(char::is_whitespace) || url.contains('\n') || url.contains('\r') {
        return Err(CoreError::InvalidSourceHostRelayUrl);
    }
    Ok(url.trim_end_matches('/').to_string())
}

fn host_facts_from_record(record: &ExistingHostProjectImport) -> HostOwnedRuntimeFacts {
    HostOwnedRuntimeFacts {
        display_name: trim_or_fallback(&record.display_name, &record.source_machine_id),
        hostname: trim_to_option(record.hostname.as_deref()),
        runtime_host: record
            .runtime_host
            .as_deref()
            .and_then(|value| trim_to_option(Some(value)))
            .unwrap_or_else(|| normalize_id_part(&record.source_host_id)),
        runtime_status: record.runtime_status,
        active_inference_profile: trim_to_option(record.active_inference_profile.as_deref()),
        hermes_available: record.hermes_available,
        published_app_urls: record.published_app_urls.clone(),
    }
}

fn trim_or_fallback(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn trim_to_option(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_idempotency_key(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(128).collect())
    }
}

pub(crate) fn normalize_profile_picture_url(value: Option<&str>) -> CoreResult<Option<String>> {
    let Some(value) = trim_to_option(value) else {
        return Ok(None);
    };
    let valid_scheme = value.starts_with("https://") || value.starts_with("http://");
    if !valid_scheme
        || value.len() > 2_048
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(CoreError::InvalidAgentProfilePictureUrl);
    }
    Ok(Some(value))
}

fn current_time_iso() -> CoreResult<String> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}

fn parse_time(value: &str) -> CoreResult<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| CoreError::InvalidTimestamp)
}

pub fn runtime_relay_token_hash(value: &str) -> CoreResult<String> {
    hash_runtime_relay_token(value)
}

fn hash_runtime_relay_token(value: &str) -> CoreResult<String> {
    let token = trim_to_option(Some(value)).ok_or(CoreError::MissingRuntimeRelayToken)?;
    let digest = Sha256::digest(token.as_bytes());
    Ok(digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>())
}

fn hash_finite_private_api_key(value: &str) -> CoreResult<String> {
    let token = trim_to_option(Some(value)).ok_or(CoreError::MissingFinitePrivateApiKey)?;
    let digest = Sha256::digest(token.as_bytes());
    Ok(digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>())
}

pub fn generate_finite_private_api_key() -> CoreResult<String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| {
        CoreError::Store(format!(
            "failed to generate Finite Private API key: {error}"
        ))
    })?;
    let mut key = String::with_capacity("fpk_live_".len() + bytes.len() * 2);
    key.push_str("fpk_live_");
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut key, "{byte:02x}")
            .map_err(|error| CoreError::Store(format!("failed to render API key: {error}")))?;
    }
    Ok(key)
}

fn normalize_id_part(value: &str) -> String {
    value.trim().to_lowercase()
}

fn id_from_parts(prefix: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            hasher.update([0]);
        }
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let hex = digest
        .iter()
        .take(10)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{prefix}_{hex}")
}

/// Generate an opaque surrogate id: `<prefix>_<20 hex chars of CSPRNG>`.
///
/// Surrogate ids are minted at insert time and are the ONLY way we assign a
/// primary key for a root entity (user, org, agent-creation request, project,
/// runtime). They are NEVER derived from PII or request inputs — that coupling
/// (`user_id = f(email)`) is exactly what let a wiped+recreated same-email
/// account collide with orphans (PERSISTENCE.md anti-pattern #5). Randomness
/// comes from `getrandom` (the OS CSPRNG), the same source the API-key
/// generator uses; this is the server crate, so the workflow-script
/// Math.random/Date.now constraints do not apply.
pub(crate) fn generate_surrogate_id(prefix: &str) -> CoreResult<String> {
    let mut bytes = [0_u8; 10];
    getrandom::getrandom(&mut bytes).map_err(|error| {
        CoreError::Store(format!("failed to generate {prefix} surrogate id: {error}"))
    })?;
    let mut id = String::with_capacity(prefix.len() + 1 + bytes.len() * 2);
    id.push_str(prefix);
    id.push('_');
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut id, "{byte:02x}").map_err(|error| {
            CoreError::Store(format!("failed to render {prefix} surrogate id: {error}"))
        })?;
    }
    Ok(id)
}

pub(crate) fn new_user_id() -> CoreResult<String> {
    generate_surrogate_id("user")
}

pub(crate) fn new_customer_org_id() -> CoreResult<String> {
    generate_surrogate_id("org")
}

fn candidate_id_for(source_key: &str) -> String {
    id_from_parts("import", &[source_key])
}

/// Opaque surrogate id for a project-import candidate, minted at insert time.
/// The candidate's natural key is `source_import_key` (UNIQUE), so reconcile and
/// claim resolve the row by that key rather than rederiving its id from the
/// source identifiers (PERSISTENCE.md anti-pattern #5).
pub(crate) fn new_import_candidate_id() -> CoreResult<String> {
    generate_surrogate_id("import")
}

fn project_id_for(candidate_id: &str) -> String {
    id_from_parts("project", &[candidate_id])
}

fn agent_runtime_id_for(candidate_id: &str) -> String {
    id_from_parts("runtime", &[candidate_id])
}

pub(crate) fn new_agent_runtime_id() -> CoreResult<String> {
    generate_surrogate_id("runtime")
}

fn agent_creation_entitlement_id_for(customer_org_id: &str) -> String {
    id_from_parts("agent_entitlement", &[customer_org_id])
}

pub(crate) fn new_agent_creation_request_id() -> CoreResult<String> {
    generate_surrogate_id("agent_request")
}

pub(crate) fn new_self_service_project_id() -> CoreResult<String> {
    generate_surrogate_id("project")
}

fn project_runtime_link_id_for(project_id: &str, agent_runtime_id: &str) -> String {
    id_from_parts("runtime_link", &[project_id, agent_runtime_id])
}

fn runtime_control_request_id_for(
    agent_runtime_id: &str,
    kind: RuntimeControlKind,
    created_at: &str,
) -> String {
    id_from_parts(
        "runtime_ctl",
        &[agent_runtime_id, kind.as_str(), created_at],
    )
}

fn runtime_artifact_supports_control(kind: RuntimeArtifactKind) -> bool {
    matches!(kind, RuntimeArtifactKind::OciImage)
}

pub(crate) fn runtime_artifact_reference_is_immutable_oci(reference: &str) -> bool {
    let Some((_, digest)) = reference.rsplit_once("@sha256:") else {
        return false;
    };
    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Release identity is every artifact field other than lifecycle timestamps.
/// Once promoted or mounted by a Runtime, an id may only be upserted with this
/// exact material identity; promotion remains a one-way lifecycle transition.
pub(crate) fn runtime_artifact_material_matches(
    existing: &RuntimeArtifact,
    candidate: &RuntimeArtifact,
) -> bool {
    existing.id == candidate.id
        && existing.kind == candidate.kind
        && existing.reference == candidate.reference
        && existing.version_label == candidate.version_label
        && existing.source_git_sha == candidate.source_git_sha
        && existing.finitec_version == candidate.finitec_version
        && existing.hermes_source_ref == candidate.hermes_source_ref
        && existing.finite_platform_plugin_ref == candidate.finite_platform_plugin_ref
        && existing.state_schema_version == candidate.state_schema_version
        && existing.base_image == candidate.base_image
}

pub(crate) fn runtime_upgrade_prelease_rejection_is_terminal(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::MissingRuntimeArtifactId
            | CoreError::RuntimeArtifactNotFound
            | CoreError::RuntimeArtifactNotPromoted
            | CoreError::RuntimeArtifactRetired
            | CoreError::RuntimeUpgradeUnsupported
            | CoreError::RuntimeUpgradeStateSchemaIncompatible
            | CoreError::RuntimeUpgradeCompletionMismatch
    )
}

fn finite_private_grant_id_for_user(user_id: &str) -> String {
    id_from_parts("fp_grant", &[user_id])
}

struct FinitePrivateAdminAuditRecord<'a> {
    action: &'a str,
    target_type: &'a str,
    target_id: &'a str,
    grant_id: Option<&'a str>,
    api_key_id: Option<&'a str>,
    /// Admin identity for operator-initiated actions. `None` means Core itself.
    actor: Option<&'a str>,
    metadata: Value,
    created_at: &'a str,
}

fn finite_private_api_key_id_for(grant_id: &str, key_hash: &str) -> String {
    id_from_parts("fp_key", &[grant_id, key_hash])
}

fn finite_private_reservation_id_for(api_key_id: &str, request_id: &str) -> String {
    id_from_parts("fp_reservation", &[api_key_id, request_id])
}

fn chat_identity_id_for_user(user_id: &str) -> String {
    id_from_parts("chat_identity", &[user_id, "hosted_web"])
}

fn project_room_membership_id_for(project_id: &str, chat_identity_id: &str) -> String {
    id_from_parts("room_member", &[project_id, chat_identity_id])
}

fn finite_private_active_window(
    grant: &FinitePrivateGrant,
    profile: &FinitePrivateLimitProfile,
    now_time: OffsetDateTime,
) -> CoreResult<(String, i64, String)> {
    let current_start = grant
        .current_window_started_at
        .as_deref()
        .map(parse_time)
        .transpose()?;
    let window_start = match current_start {
        Some(start) if now_time < start + Duration::seconds(profile.burst_window_seconds) => start,
        _ => now_time,
    };
    let used_units = if current_start == Some(window_start) {
        grant.current_window_used_units
    } else {
        0
    };
    let reset_at =
        (window_start + Duration::seconds(profile.burst_window_seconds)).format(&Rfc3339)?;
    Ok((window_start.format(&Rfc3339)?, used_units, reset_at))
}

fn finite_private_window_reset_at(
    grant: &FinitePrivateGrant,
    profile: &FinitePrivateLimitProfile,
    now_time: OffsetDateTime,
) -> CoreResult<String> {
    let (_, _, reset_at) = finite_private_active_window(grant, profile, now_time)?;
    Ok(reset_at)
}

fn finite_private_allow_decision(
    reservation_id: String,
    profile: &FinitePrivateLimitProfile,
    burst_remaining_units: i64,
    burst_reset_at: String,
    weekly_remaining_units: Option<i64>,
    weekly_reset_at: Option<String>,
) -> FinitePrivateUsageDecision {
    FinitePrivateUsageDecision {
        decision: "allow".to_string(),
        reservation_id: Some(reservation_id),
        limit_profile: Some(profile.id.clone()),
        burst_limit_units: Some(profile.burst_limit_units),
        burst_remaining_units: Some(burst_remaining_units.max(0)),
        burst_reset_at: Some(burst_reset_at),
        weekly_limit_units: profile.weekly_limit_units,
        weekly_remaining_units: weekly_remaining_units.map(|remaining| remaining.max(0)),
        weekly_reset_at,
        error: None,
    }
}

fn finite_private_denial(
    request_id: String,
    dashboard_url: String,
    message: &str,
    code: &str,
    retry_after: Option<i64>,
    reset_at: Option<String>,
) -> FinitePrivateUsageDecision {
    FinitePrivateUsageDecision {
        decision: "deny".to_string(),
        reservation_id: None,
        limit_profile: None,
        burst_limit_units: None,
        burst_remaining_units: None,
        burst_reset_at: reset_at.clone(),
        weekly_limit_units: None,
        weekly_remaining_units: None,
        weekly_reset_at: reset_at.clone(),
        error: Some(FinitePrivateUsageError {
            message: message.to_string(),
            error_type: "usage_limit".to_string(),
            code: code.to_string(),
            retry_after,
            reset_at,
            dashboard_url,
            request_id,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-05-25T12:00:00Z";
    const LATER: &str = "2026-05-25T13:00:00Z";

    #[test]
    fn existing_host_import_creates_multiple_claimable_candidates_without_visible_projects() {
        let mut state = BridgeCoreState::default();

        let report = state
            .reconcile_existing_host_imports(
                &[
                    import("smoke", "paul-smoke", "Paul Smoke", Some("PAUL@FINITE.VIP")),
                    import("box1", "paul-box", "Paul Box", Some("paul@finite.vip")),
                    import("trf", "paul-trf", "Paul TRF", Some("paul@finite.vip")),
                ],
                options(["paul@finite.vip"], NOW),
            )
            .unwrap();

        assert_eq!(report.created_candidates.len(), 3);
        assert_eq!(
            state
                .claimable_candidates_for_email(Some("paul@finite.vip"))
                .len(),
            3
        );
        assert!(state.projects.is_empty());
        assert!(state.agent_runtimes.is_empty());
    }

    #[test]
    fn allowlist_and_owner_email_decide_imports_not_admin_visibility() {
        let mut state = BridgeCoreState::default();
        let mut rene_bot = import("trf", "grant", "Grant", Some("rene@example.com"));
        rene_bot
            .admin_visible_to_emails
            .push("paul@finite.vip".to_string());

        let report = state
            .reconcile_existing_host_imports(&[rene_bot], options(["paul@finite.vip"], NOW))
            .unwrap();

        assert!(report.created_candidates.is_empty());
        assert_eq!(report.skipped_records.len(), 1);
        assert_eq!(
            report.skipped_records[0].reason,
            SkippedImportReason::OwnerNotAllowlisted
        );
    }

    #[test]
    fn verified_workos_login_claims_selected_projects_and_keeps_test_user_grandfathered() {
        let mut state = BridgeCoreState::default();
        state
            .reconcile_existing_host_imports(
                &[import(
                    "smoke",
                    "test-smoke",
                    "Smoke",
                    Some("test@finite.vip"),
                )],
                options(["test@finite.vip"], NOW),
            )
            .unwrap();
        let candidate_id = state.claimable_candidates_for_email(Some("test@finite.vip"))[0]
            .id
            .clone();

        let result = state
            .claim_project_imports(ClaimProjectImportsInput {
                verified_email: "test@finite.vip".to_string(),
                workos_user_id: "user_workos_test".to_string(),
                selected_candidate_ids: vec![candidate_id],
                now: Some(LATER.to_string()),
            })
            .unwrap();

        assert_eq!(result.claimed_project_ids.len(), 1);
        let user = state.users.values().next().unwrap();
        assert_eq!(user.status, UserLinkStatus::Linked);
        assert_eq!(user.workos_user_id.as_deref(), Some("user_workos_test"));
        let org = state.customer_orgs.values().next().unwrap();
        assert_eq!(org.billing_class, BillingClass::Grandfathered);
        assert_eq!(state.visible_projects_for_user(&user.id).len(), 1);
    }

    #[test]
    fn verified_email_can_relink_to_new_workos_user_and_keep_imported_projects() {
        let mut state = BridgeCoreState::default();
        state
            .reconcile_existing_host_imports(
                &[import(
                    "smoke",
                    "paul-smoke",
                    "Paul Smoke",
                    Some("paul@finite.vip"),
                )],
                options(["paul@finite.vip"], NOW),
            )
            .unwrap();
        let candidate_id = state.claimable_candidates_for_email(Some("paul@finite.vip"))[0]
            .id
            .clone();
        let claimed = state
            .claim_project_imports(ClaimProjectImportsInput {
                verified_email: "paul@finite.vip".to_string(),
                workos_user_id: "user_workos_staging".to_string(),
                selected_candidate_ids: vec![candidate_id],
                now: Some(LATER.to_string()),
            })
            .unwrap();

        let user = state
            .link_verified_user(LinkVerifiedUserInput {
                verified_email: "paul@finite.vip".to_string(),
                workos_user_id: "user_workos_prod_google".to_string(),
                now: Some("2026-05-25T15:00:00Z".to_string()),
            })
            .unwrap();

        assert_eq!(
            user.workos_user_id.as_deref(),
            Some("user_workos_prod_google")
        );
        assert_eq!(
            state.visible_projects_for_user(&user.id)[0].id,
            claimed.claimed_project_ids[0]
        );
        let org = state.find_personal_org_by_owner(&user.id).unwrap();
        assert_eq!(org.billing_class, BillingClass::Grandfathered);
    }

    #[test]
    fn claiming_is_idempotent_and_reimport_updates_only_host_owned_facts() {
        let mut state = BridgeCoreState::default();
        state
            .reconcile_existing_host_imports(
                &[import(
                    "smoke",
                    "paul-smoke",
                    "Paul Smoke",
                    Some("paul@finite.vip"),
                )],
                options(["paul@finite.vip"], NOW),
            )
            .unwrap();
        let candidate_id = state.claimable_candidates_for_email(Some("paul@finite.vip"))[0]
            .id
            .clone();

        let first = state
            .claim_project_imports(ClaimProjectImportsInput {
                verified_email: "paul@finite.vip".to_string(),
                workos_user_id: "user_workos_paul".to_string(),
                selected_candidate_ids: vec![candidate_id.clone()],
                now: Some(LATER.to_string()),
            })
            .unwrap();
        let second = state
            .claim_project_imports(ClaimProjectImportsInput {
                verified_email: "paul@finite.vip".to_string(),
                workos_user_id: "user_workos_paul".to_string(),
                selected_candidate_ids: vec![candidate_id.clone()],
                now: Some("2026-05-25T14:00:00Z".to_string()),
            })
            .unwrap();

        assert_eq!(first.claimed_project_ids.len(), 1);
        assert!(second.claimed_project_ids.is_empty());
        assert_eq!(
            second.already_claimed_project_ids,
            first.claimed_project_ids
        );

        let mut changed = import(
            "smoke",
            "paul-smoke",
            "Renamed Smoke",
            Some("other@finite.vip"),
        );
        changed.hostname = Some("new-smoke.finite.vip".to_string());
        changed.runtime_status = RuntimeSummaryStatus::Online;
        state
            .reconcile_existing_host_imports(&[changed], options(["other@finite.vip"], LATER))
            .unwrap();

        let candidate = state.project_import_candidates.get(&candidate_id).unwrap();
        assert_eq!(candidate.owner_email, "paul@finite.vip");
        assert_eq!(
            candidate.latest_host_owner_email.as_deref(),
            Some("other@finite.vip")
        );
        assert_eq!(candidate.host_facts.display_name, "Renamed Smoke");
        let project = state.projects.get(&first.claimed_project_ids[0]).unwrap();
        assert_eq!(project.owner_user_id, candidate.pending_user_id);
    }

    #[test]
    fn same_historical_machine_id_on_different_hosts_does_not_collide() {
        let mut state = BridgeCoreState::default();

        state
            .reconcile_existing_host_imports(
                &[
                    import("smoke", "grant", "Smoke Grant", Some("rene@example.com")),
                    import("trf", "grant", "TRF Grant", Some("rene@example.com")),
                ],
                options(["rene@example.com"], NOW),
            )
            .unwrap();

        assert_eq!(state.project_import_candidates.len(), 2);
        let keys = state
            .project_import_candidates
            .values()
            .map(|candidate| candidate.source_import_key.as_str())
            .collect::<BTreeSet<_>>();
        assert!(keys.contains("smoke:grant"));
        assert!(keys.contains("trf:grant"));
    }

    #[test]
    fn multi_user_telegram_bot_is_claimable_only_by_owner_without_participant_memberships() {
        let mut state = BridgeCoreState::default();
        let mut grant = import("trf", "grant", "Grant", Some("rene@example.com"));
        grant
            .known_external_channel_participants
            .push(KnownExternalChannelParticipant {
                channel: "telegram".to_string(),
                external_user_id: Some("telegram:paul".to_string()),
                username: Some("paul".to_string()),
                display_name: Some("Paul".to_string()),
            });
        state
            .reconcile_existing_host_imports(
                &[grant],
                options(["paul@finite.vip", "rene@example.com"], NOW),
            )
            .unwrap();
        let candidate_id = state.claimable_candidates_for_email(Some("rene@example.com"))[0]
            .id
            .clone();

        let denied = state
            .claim_project_imports(ClaimProjectImportsInput {
                verified_email: "paul@finite.vip".to_string(),
                workos_user_id: "user_workos_paul".to_string(),
                selected_candidate_ids: vec![candidate_id.clone()],
                now: Some(LATER.to_string()),
            })
            .unwrap();
        assert_eq!(denied.denied_candidate_ids, vec![candidate_id.clone()]);
        assert!(state.projects.is_empty());

        let claimed = state
            .claim_project_imports(ClaimProjectImportsInput {
                verified_email: "rene@example.com".to_string(),
                workos_user_id: "user_workos_rene".to_string(),
                selected_candidate_ids: vec![candidate_id.clone()],
                now: Some("2026-05-25T14:00:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(claimed.claimed_project_ids.len(), 1);
        assert_eq!(state.project_room_memberships.len(), 1);
        assert_eq!(
            state
                .project_import_candidates
                .get(&candidate_id)
                .unwrap()
                .known_external_channel_participants
                .len(),
            1
        );
    }

    #[test]
    fn launch_code_creates_one_self_serve_agent_request_and_visible_project() {
        let mut state = BridgeCoreState::default();

        let first = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let second = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent duplicate submit".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "first-submit".to_string(),
                now: Some(LATER.to_string()),
            })
            .unwrap();

        assert!(!first.reused);
        assert!(second.reused);
        assert_eq!(first.request.id, second.request.id);
        assert_eq!(first.project.id, second.project.id);
        assert_eq!(state.projects.len(), 1);
        assert_eq!(state.agent_runtimes.len(), 0);
        assert_eq!(state.agent_creation_requests.len(), 1);
        let user = state.users.values().next().unwrap();
        let org = state.customer_orgs.values().next().unwrap();
        assert_eq!(org.billing_class, BillingClass::Off2026);
        assert_eq!(
            state.visible_projects_for_user(&user.id),
            vec![first.project]
        );
    }

    #[test]
    fn project_selected_runner_class_routes_to_a_matching_worker() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);
        let requested = state
            .request_agent_creation_configured(
                RequestAgentCreationInput {
                    verified_email: "kata@finite.vip".to_string(),
                    workos_user_id: "user_workos_kata".to_string(),
                    display_name: "Kata Agent".to_string(),
                    launch_code: "off2026".to_string(),
                    idempotency_key: "kata-submit".to_string(),
                    now: Some(NOW.to_string()),
                },
                AgentCreationConfiguration {
                    runner_class: RunnerClass::Kata,
                    profile_picture_url: Some(
                        "https://chat.finite.computer/v1/blobs/profile".to_string(),
                    ),
                },
            )
            .unwrap();
        assert_eq!(requested.request.runner_class, RunnerClass::Kata);

        let phala = state
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "phala-worker".to_string(),
                source_host_id: None,
                lease_token: "phala-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Phala],
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some(LATER.to_string()),
            })
            .unwrap();
        assert!(phala.is_none());

        let kata = state
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "kata-worker".to_string(),
                source_host_id: None,
                lease_token: "kata-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some(LATER.to_string()),
            })
            .unwrap()
            .expect("Kata worker should claim Kata placement");
        assert_eq!(kata.request.id, requested.request.id);
    }

    #[test]
    fn runner_leases_and_completes_self_serve_agent_request() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);
        let requested = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();

        let lease = state
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                source_host_id: None,
                lease_token: "lease-token-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .unwrap()
            .expect("pending request should be leased");
        assert_eq!(lease.project.id, requested.project.id);
        assert_eq!(lease.request.status, AgentCreationRequestStatus::Launching);
        assert_eq!(lease.request.runner_id.as_deref(), Some("runner-oslo-1"));
        assert!(lease.request.lease_expires_at.is_some());

        let none = state
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-2".to_string(),
                source_host_id: None,
                lease_token: "lease-token-2".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some("2026-05-25T13:01:00Z".to_string()),
            })
            .unwrap();
        assert!(none.is_none());

        let completed = state
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "oslo-agent-001".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
                display_name: None,
                hostname: Some("oslo-agent-001.finite.computer".to_string()),
                runtime_host: Some("oslo-host-1".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .unwrap();

        assert_eq!(
            completed.request.status,
            AgentCreationRequestStatus::Running
        );
        assert!(completed.request.lease_token.is_none());
        let runtime_id = completed.request.agent_runtime_id.unwrap();
        let runtime = state.agent_runtimes.get(&runtime_id).unwrap();
        assert_eq!(runtime.project_id, requested.project.id);
        assert_eq!(runtime.runtime_artifact_id.as_deref(), Some("artifact-v1"));
        assert_eq!(runtime.state_schema_version.as_deref(), Some("state-v1"));
        assert_eq!(runtime.source_host_id, "oslo-host-1");
        assert_eq!(runtime.source_machine_id, "oslo-agent-001");
        assert_eq!(
            runtime.host_facts.runtime_status,
            RuntimeSummaryStatus::Online
        );
        assert_eq!(
            state
                .project_runtime_links
                .values()
                .filter(|link| link.project_id == requested.project.id && link.active)
                .count(),
            1
        );
    }

    #[test]
    fn runtime_artifact_promotion_does_not_mutate_healthy_running_agent() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);
        let runtime_a = complete_self_serve_agent(
            &mut state,
            "a@finite.vip",
            "user_workos_a",
            "agent-a",
            "oslo-agent-a",
            "artifact-v1",
            "2026-05-25T13:02:00Z",
        );
        let runtime_a_before = state.agent_runtimes.get(&runtime_a).unwrap().clone();

        promote_runtime_artifact_version(
            &mut state,
            "artifact-v2",
            "finite-runtime-v2",
            "v2",
            "state-v1",
            "2026-05-25T14:00:00Z",
        );

        assert_eq!(
            state.agent_runtimes.get(&runtime_a).unwrap(),
            &runtime_a_before
        );
        assert_eq!(
            state.agent_runtimes[&runtime_a]
                .runtime_artifact_id
                .as_deref(),
            Some("artifact-v1")
        );

        let runtime_b = complete_self_serve_agent(
            &mut state,
            "b@finite.vip",
            "user_workos_b",
            "agent-b",
            "oslo-agent-b",
            "artifact-v2",
            "2026-05-25T14:05:00Z",
        );
        assert_eq!(
            state.agent_runtimes[&runtime_b]
                .runtime_artifact_id
                .as_deref(),
            Some("artifact-v2")
        );
        assert_eq!(
            state.agent_runtimes[&runtime_a]
                .runtime_artifact_id
                .as_deref(),
            Some("artifact-v1")
        );
    }

    #[test]
    fn promoted_or_runtime_referenced_artifact_material_is_immutable() {
        let mut state = BridgeCoreState::default();
        let input = UpsertRuntimeArtifactInput {
            id: "artifact-immutable".to_string(),
            kind: RuntimeArtifactKind::OciImage,
            reference: format!("ghcr.io/finite/runtime@sha256:{}", "a".repeat(64)),
            version_label: "v1".to_string(),
            source_git_sha: Some("git-v1".to_string()),
            finitec_version: Some("finitec-v1".to_string()),
            hermes_source_ref: Some("hermes-v1".to_string()),
            finite_platform_plugin_ref: Some("plugin-v1".to_string()),
            state_schema_version: "state-v1".to_string(),
            base_image: Some("base-v1".to_string()),
            promoted: false,
            now: Some(NOW.to_string()),
        };
        state.upsert_runtime_artifact(input.clone()).unwrap();

        let mut before_promotion = input.clone();
        before_promotion.version_label = "v1-corrected".to_string();
        state
            .upsert_runtime_artifact(before_promotion.clone())
            .unwrap();
        before_promotion.promoted = true;
        state
            .upsert_runtime_artifact(before_promotion.clone())
            .unwrap();

        let mut exact_retry = before_promotion.clone();
        exact_retry.now = Some(LATER.to_string());
        state.upsert_runtime_artifact(exact_retry).unwrap();
        let mut mutation = before_promotion;
        mutation.reference = format!("ghcr.io/finite/runtime@sha256:{}", "b".repeat(64));
        assert!(matches!(
            state.upsert_runtime_artifact(mutation).unwrap_err(),
            CoreError::RuntimeArtifactImmutable
        ));

        let runtime_id = "runtime-references-unpromoted".to_string();
        state
            .runtime_artifacts
            .get_mut("artifact-immutable")
            .unwrap()
            .promoted_at = None;
        state.agent_runtimes.insert(
            runtime_id.clone(),
            AgentRuntime {
                id: runtime_id,
                project_id: "project-test".to_string(),
                source_host_id: "host-test".to_string(),
                source_machine_id: "machine-test".to_string(),
                source_import_key: "host-test/machine-test".to_string(),
                runtime_artifact_id: Some("artifact-immutable".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                host_facts: HostOwnedRuntimeFacts {
                    display_name: "Test Agent".to_string(),
                    hostname: None,
                    runtime_host: "host-test".to_string(),
                    runtime_status: RuntimeSummaryStatus::Online,
                    active_inference_profile: None,
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                },
                created_at: NOW.to_string(),
                updated_at: NOW.to_string(),
            },
        );
        let mut referenced_mutation = input;
        referenced_mutation.version_label = "mutated".to_string();
        assert!(matches!(
            state
                .upsert_runtime_artifact(referenced_mutation)
                .unwrap_err(),
            CoreError::RuntimeArtifactImmutable
        ));
    }

    #[test]
    fn self_serve_agent_creation_requires_promoted_runtime_artifact() {
        let mut state = BridgeCoreState::default();
        state
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: "artifact-v1".to_string(),
                kind: RuntimeArtifactKind::OciImage,
                reference: "ghcr.io/finitecomputer/finite-agent-runtime:v1".to_string(),
                version_label: "v1".to_string(),
                source_git_sha: None,
                finitec_version: None,
                hermes_source_ref: None,
                finite_platform_plugin_ref: None,
                state_schema_version: "state-v1".to_string(),
                base_image: Some("python:3.11-trixie".to_string()),
                promoted: false,
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let requested = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let lease = state
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                source_host_id: None,
                lease_token: "lease-token-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .unwrap()
            .expect("pending request should be leased");

        let error = state
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: lease.request.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "oslo-agent-001".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
                display_name: None,
                hostname: None,
                runtime_host: None,
                runtime_status: None,
                active_inference_profile: None,
                hermes_available: None,
                published_app_urls: Vec::new(),
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .unwrap_err();

        assert!(matches!(error, CoreError::RuntimeArtifactNotPromoted));
        assert!(state.agent_runtimes.is_empty());
        assert_eq!(
            state.agent_creation_requests[&requested.request.id].status,
            AgentCreationRequestStatus::Launching
        );
    }

    #[test]
    fn self_serve_runtime_must_publish_relay_heartbeat_before_running() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);
        let requested = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let lease = state
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                source_host_id: None,
                lease_token: "lease-token-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .unwrap()
            .unwrap();
        let runtime_token = "runtime-token-1";
        let token_hash = runtime_relay_token_hash(runtime_token).unwrap();

        let registered = state
            .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "oslo-agent-001".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
                runtime_relay_token_hash: token_hash,
                display_name: None,
                hostname: None,
                runtime_host: Some("oslo-host-1".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Unknown),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: None,
                published_app_urls: Vec::new(),
                now: Some("2026-05-25T13:01:30Z".to_string()),
            })
            .unwrap();

        assert_eq!(
            registered.request.status,
            AgentCreationRequestStatus::Launching
        );
        assert!(registered.request.agent_runtime_id.is_some());
        assert!(
            state
                .runtime_heartbeat_for_machine("oslo-agent-001")
                .is_err()
        );

        let heartbeat = state.record_runtime_heartbeat(runtime_token).unwrap();
        assert_eq!(heartbeat.machine_id, "oslo-agent-001");
        let events = state.relay_events_for_runtime(runtime_token).unwrap();
        assert_eq!(events.machine_id, "oslo-agent-001");
        assert!(events.events.is_empty());
        assert_eq!(
            state
                .runtime_heartbeat_for_machine("oslo-agent-001")
                .unwrap()
                .last_seen_at,
            heartbeat.last_seen_at
        );

        let completed = state
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: lease.request.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "oslo-agent-001".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
                display_name: None,
                hostname: None,
                runtime_host: Some("oslo-host-1".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .unwrap();

        assert_eq!(
            completed.request.status,
            AgentCreationRequestStatus::Running
        );
        assert_eq!(completed.project.id, requested.project.id);
    }

    #[test]
    fn user_can_request_and_runner_can_complete_oci_runtime_restart() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);
        let runtime_id = complete_self_serve_agent(
            &mut state,
            "new@finite.vip",
            "user_workos_new",
            "first-submit",
            "oslo-agent-001",
            "artifact-v1",
            "2026-05-25T13:02:00Z",
        );
        let project_id = state.agent_runtimes[&runtime_id].project_id.clone();

        let restart = state
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id,
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .unwrap();

        assert_eq!(restart.agent_runtime_id, runtime_id);
        assert_eq!(restart.source_host_id, "oslo-host-1");
        assert_eq!(restart.source_machine_id, "oslo-agent-001");
        assert_eq!(restart.kind, RuntimeControlKind::Restart);
        assert_eq!(restart.status, RuntimeControlRequestStatus::Requested);

        let duplicate = state
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id: restart.project_id.clone(),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(duplicate.id, restart.id);

        let lease = state
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "restart-lease-1".to_string(),
                lease_seconds: Some(60),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: None,
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .unwrap()
            .expect("restart request should lease");

        assert_eq!(lease.request.id, restart.id);
        assert_eq!(lease.request.status, RuntimeControlRequestStatus::Running);
        assert_eq!(lease.runtime.source_machine_id, "oslo-agent-001");

        let stale_complete = state
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: restart.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "wrong-token".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_host: None,
                published_app_urls: None,
                now: Some("2026-05-25T13:04:30Z".to_string()),
            })
            .unwrap_err();
        assert!(matches!(
            stale_complete,
            CoreError::RuntimeControlRequestLeaseConflict
        ));

        let completed = state
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: restart.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "restart-lease-1".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_host: None,
                published_app_urls: None,
                now: Some("2026-05-25T13:05:00Z".to_string()),
            })
            .unwrap();

        assert_eq!(completed.status, RuntimeControlRequestStatus::Succeeded);
        assert!(completed.lease_token.is_none());
        assert_eq!(
            state.agent_runtimes[&runtime_id].host_facts.runtime_status,
            RuntimeSummaryStatus::Online
        );
        assert!(
            state
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "restart-lease-2".to_string(),
                    lease_seconds: Some(60),
                    source_host_id: Some("oslo-host-1".to_string()),
                    runner_capacity: None,
                    now: Some("2026-05-25T13:06:00Z".to_string()),
                })
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn user_can_request_recover_known_good_chat_runtime() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);
        let runtime_id = complete_self_serve_agent(
            &mut state,
            "new@finite.vip",
            "user_workos_new",
            "first-submit",
            "oslo-agent-001",
            "artifact-v1",
            "2026-05-25T13:02:00Z",
        );
        let project_id = state.agent_runtimes[&runtime_id].project_id.clone();

        let recovery = state
            .request_runtime_recover_known_good_chat(RequestRuntimeRecoverKnownGoodChatInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id,
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .unwrap();

        assert_eq!(recovery.agent_runtime_id, runtime_id);
        assert_eq!(recovery.source_host_id, "oslo-host-1");
        assert_eq!(recovery.source_machine_id, "oslo-agent-001");
        assert_eq!(
            recovery.kind,
            RuntimeControlKind::RecoverKnownGoodChatRuntime
        );
        assert_eq!(recovery.status, RuntimeControlRequestStatus::Requested);

        let duplicate = state
            .request_runtime_recover_known_good_chat(RequestRuntimeRecoverKnownGoodChatInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id: recovery.project_id.clone(),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(duplicate.id, recovery.id);
    }

    #[test]
    fn stop_and_destroy_update_plaintext_runtime_status() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);
        let runtime_id = complete_self_serve_agent(
            &mut state,
            "new@finite.vip",
            "user_workos_new",
            "first-submit",
            "oslo-agent-001",
            "artifact-v1",
            "2026-05-25T13:02:00Z",
        );
        let project_id = state.agent_runtimes[&runtime_id].project_id.clone();
        state.runtime_relay_credentials.insert(
            runtime_id.clone(),
            RuntimeRelayCredential {
                agent_runtime_id: runtime_id.clone(),
                token_hash: runtime_relay_token_hash("destroy-test-relay-token").unwrap(),
                created_at: "2026-05-25T13:02:15Z".to_string(),
                updated_at: "2026-05-25T13:02:15Z".to_string(),
            },
        );
        assert!(state.runtime_relay_credentials.contains_key(&runtime_id));
        let grant = state
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: Some("user_workos_new".to_string()),
                limit_profile_id: None,
                now: Some("2026-05-25T13:02:30Z".to_string()),
            })
            .unwrap();
        let runtime_key = state
            .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id,
                raw_key: "fpk_live_destroy_test".to_string(),
                project_id: Some(project_id.clone()),
                agent_runtime_id: Some(runtime_id.clone()),
                now: Some("2026-05-25T13:02:31Z".to_string()),
            })
            .unwrap();
        state
            .agent_runtimes
            .get_mut(&runtime_id)
            .unwrap()
            .host_facts
            .published_app_urls = vec!["https://oslo-agent.example.com/contact".to_string()];

        let stop = state
            .request_runtime_stop(RequestRuntimeStopInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id: project_id.clone(),
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .unwrap();
        let stop_lease = state
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "stop-lease-1".to_string(),
                lease_seconds: Some(60),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: None,
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .unwrap()
            .expect("stop request should lease");
        assert_eq!(stop_lease.request.kind, RuntimeControlKind::Stop);
        state
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: stop.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "stop-lease-1".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_host: None,
                published_app_urls: None,
                now: Some("2026-05-25T13:05:00Z".to_string()),
            })
            .unwrap();
        let stopped_runtime = &state.agent_runtimes[&runtime_id];
        assert_eq!(
            stopped_runtime.host_facts.runtime_status,
            RuntimeSummaryStatus::Offline
        );
        assert_eq!(
            stopped_runtime.host_facts.published_app_urls,
            vec!["https://oslo-agent.example.com/contact".to_string()]
        );

        let destroy = state
            .request_runtime_destroy(RequestRuntimeDestroyInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id,
                now: Some("2026-05-25T13:06:00Z".to_string()),
            })
            .unwrap();
        let destroy_lease = state
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "destroy-lease-1".to_string(),
                lease_seconds: Some(60),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: None,
                now: Some("2026-05-25T13:07:00Z".to_string()),
            })
            .unwrap()
            .expect("destroy request should lease");
        assert_eq!(destroy_lease.request.kind, RuntimeControlKind::Destroy);
        state
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: destroy.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "destroy-lease-1".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_host: None,
                published_app_urls: None,
                now: Some("2026-05-25T13:08:00Z".to_string()),
            })
            .unwrap();
        let destroyed_runtime = &state.agent_runtimes[&runtime_id];
        assert_eq!(
            destroyed_runtime.host_facts.runtime_status,
            RuntimeSummaryStatus::Offline
        );
        assert_eq!(destroyed_runtime.host_facts.hermes_available, Some(false));
        assert!(destroyed_runtime.host_facts.published_app_urls.is_empty());
        assert!(!state.runtime_relay_credentials.contains_key(&runtime_id));
        assert!(
            state
                .project_runtime_links
                .values()
                .filter(|link| link.agent_runtime_id == runtime_id)
                .all(|link| !link.active)
        );
        assert_eq!(
            state.finite_private_api_keys[&runtime_key.id].status,
            FinitePrivateApiKeyStatus::Revoked
        );
        assert!(
            state
                .finite_private_admin_audit_events
                .values()
                .any(|event| event.action == "finite_private.runtime.destroy_revoke_keys")
        );
    }

    #[test]
    fn oci_runtime_artifacts_support_hosted_runtime_control() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);
        state.runtime_artifacts.get_mut("artifact-v1").unwrap().kind =
            RuntimeArtifactKind::OciImage;
        let runtime_id = complete_self_serve_agent(
            &mut state,
            "new@finite.vip",
            "user_workos_new",
            "first-submit",
            "docker-agent-001",
            "artifact-v1",
            "2026-05-25T13:02:00Z",
        );
        let project_id = state.agent_runtimes[&runtime_id].project_id.clone();

        let restart = state
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                project_id,
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .unwrap();

        assert_eq!(restart.agent_runtime_id, runtime_id);
        assert_eq!(restart.kind, RuntimeControlKind::Restart);
    }

    #[test]
    fn imported_legacy_runtime_restart_is_rejected() {
        let mut state = BridgeCoreState::default();
        state
            .reconcile_existing_host_imports(
                &[import(
                    "smoke",
                    "paul-smoke",
                    "Paul Smoke",
                    Some("paul@finite.vip"),
                )],
                options(["paul@finite.vip"], NOW),
            )
            .unwrap();
        let candidate_id = state.claimable_candidates_for_email(Some("paul@finite.vip"))[0]
            .id
            .clone();
        let claimed = state
            .claim_project_imports(ClaimProjectImportsInput {
                verified_email: "paul@finite.vip".to_string(),
                workos_user_id: "user_workos_paul".to_string(),
                selected_candidate_ids: vec![candidate_id],
                now: Some(LATER.to_string()),
            })
            .unwrap();

        let error = state
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "paul@finite.vip".to_string(),
                workos_user_id: "user_workos_paul".to_string(),
                project_id: claimed.claimed_project_ids[0].clone(),
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .unwrap_err();

        assert!(matches!(error, CoreError::RuntimeRestartUnsupported));
    }

    #[test]
    fn runner_lease_can_expire_and_reassign_but_completion_requires_current_token() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);
        let requested = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let first_lease = state
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-a".to_string(),
                source_host_id: None,
                lease_token: "lease-a".to_string(),
                lease_seconds: Some(60),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .unwrap()
            .unwrap();
        assert_eq!(first_lease.request.project_id, requested.project.id);
        let second_lease = state
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-b".to_string(),
                source_host_id: None,
                lease_token: "lease-b".to_string(),
                lease_seconds: Some(60),
                runner_capacity: None,
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .unwrap()
            .unwrap();
        assert_eq!(second_lease.request.runner_id.as_deref(), Some("runner-b"));

        let stale_complete = state
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: requested.request.id,
                runner_id: "runner-a".to_string(),
                lease_token: "lease-a".to_string(),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "oslo-agent-001".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
                display_name: None,
                hostname: None,
                runtime_host: None,
                runtime_status: None,
                active_inference_profile: None,
                hermes_available: None,
                published_app_urls: Vec::new(),
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .unwrap_err();
        assert!(matches!(
            stale_complete,
            CoreError::AgentCreationRequestLeaseConflict
        ));
    }

    #[test]
    fn runner_can_mark_agent_creation_request_failed_without_runtime() {
        let mut state = BridgeCoreState::default();
        let requested = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        state
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                source_host_id: None,
                lease_token: "lease-token-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .unwrap();

        let failed = state
            .fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id: requested.request.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                failure_message: "runner capacity unavailable".to_string(),
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .unwrap();

        assert_eq!(failed.status, AgentCreationRequestStatus::Failed);
        assert_eq!(
            failed.failure_message.as_deref(),
            Some("runner capacity unavailable")
        );
        assert!(failed.agent_runtime_id.is_none());
        assert!(state.agent_runtimes.is_empty());
    }

    #[test]
    fn operator_can_cancel_failed_creation_request_and_user_can_retry() {
        let mut state = BridgeCoreState::default();
        let requested = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        state
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                source_host_id: None,
                lease_token: "lease-token-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .unwrap();
        state
            .fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id: requested.request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                failure_message: "runner capacity unavailable".to_string(),
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .unwrap();

        let cancelled = state
            .cancel_agent_creation_request(CancelAgentCreationRequestInput {
                request_id: requested.request.id,
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .unwrap();

        assert_eq!(cancelled.status, AgentCreationRequestStatus::Cancelled);
        assert!(cancelled.agent_runtime_id.is_none());
        assert!(
            state
                .visible_projects_for_user(&requested.project.owner_user_id)
                .is_empty()
        );

        let retried = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Retry Agent".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "second-submit".to_string(),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(
            retried.request.status,
            AgentCreationRequestStatus::Requested
        );
    }

    #[test]
    fn failed_self_serve_launch_removes_provisional_runtime() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);
        let requested = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let lease = state
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                source_host_id: None,
                lease_token: "lease-token-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .unwrap()
            .unwrap();
        state
            .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "oslo-agent-001".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
                runtime_relay_token_hash: runtime_relay_token_hash("runtime-token-1").unwrap(),
                display_name: None,
                hostname: None,
                runtime_host: Some("oslo-host-1".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Unknown),
                active_inference_profile: None,
                hermes_available: None,
                published_app_urls: Vec::new(),
                now: Some("2026-05-25T13:01:30Z".to_string()),
            })
            .unwrap();

        assert_eq!(state.agent_runtimes.len(), 1);
        assert_eq!(state.runtime_relay_credentials.len(), 1);
        assert_eq!(state.project_runtime_links.len(), 1);

        let failed = state
            .fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id: requested.request.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                failure_message: "runtime did not publish a relay heartbeat".to_string(),
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .unwrap();

        assert_eq!(failed.status, AgentCreationRequestStatus::Failed);
        assert!(failed.agent_runtime_id.is_none());
        assert!(state.agent_runtimes.is_empty());
        assert!(state.runtime_relay_credentials.is_empty());
        assert!(state.project_runtime_links.is_empty());
        assert!(
            state
                .runtime_heartbeat_for_machine("oslo-agent-001")
                .is_err()
        );
    }

    #[test]
    fn self_serve_agent_creation_rejects_bad_code_and_exhausted_entitlement() {
        let mut state = BridgeCoreState::default();

        let bad = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: "wrong".to_string(),
                idempotency_key: "bad-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap_err();
        assert!(matches!(bad, CoreError::InvalidLaunchCode));
        assert!(state.users.is_empty());
        assert!(state.customer_orgs.is_empty());
        assert!(state.agent_creation_entitlements.is_empty());

        state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let exhausted = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Second Agent".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "second-submit".to_string(),
                now: Some(LATER.to_string()),
            })
            .unwrap_err();
        assert!(matches!(
            exhausted,
            CoreError::AgentCreationEntitlementExhausted
        ));
    }

    #[test]
    fn imported_runtime_does_not_consume_self_serve_launch_entitlement() {
        let mut state = BridgeCoreState::default();
        let email = "import-with-launch@finite.vip";
        let workos_user_id = "user_workos_import_with_launch";

        let reconciled = state
            .reconcile_existing_host_imports(
                &[import(
                    "legacy-host",
                    "legacy-agent-001",
                    "Imported Agent",
                    Some(email),
                )],
                options([email], NOW),
            )
            .unwrap();
        let candidate_id = reconciled.created_candidates[0].clone();
        let claimed = state
            .claim_project_imports(ClaimProjectImportsInput {
                verified_email: email.to_string(),
                workos_user_id: workos_user_id.to_string(),
                selected_candidate_ids: vec![candidate_id.clone()],
                now: Some(LATER.to_string()),
            })
            .unwrap();
        let imported_project_id = claimed.claimed_project_ids[0].clone();
        let imported_candidate = state.project_import_candidates[&candidate_id].clone();
        let imported_runtime_id = imported_candidate
            .agent_runtime_id
            .clone()
            .expect("claimed import has a runtime");
        let imported_project = state.projects[&imported_project_id].clone();
        let imported_runtime = state.agent_runtimes[&imported_runtime_id].clone();
        let imported_link = state
            .project_runtime_links
            .values()
            .find(|link| link.project_id == imported_project_id && link.active)
            .cloned()
            .expect("claimed import has an active runtime link");

        let created = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: email.to_string(),
                workos_user_id: workos_user_id.to_string(),
                display_name: "New Hosted Agent".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "first-self-serve-submit".to_string(),
                now: Some("2026-05-25T14:00:00Z".to_string()),
            })
            .expect("an imported runtime must not consume the hosted launch");
        assert!(created.project.import_candidate_id.is_none());

        let exhausted = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: email.to_string(),
                workos_user_id: workos_user_id.to_string(),
                display_name: "Another Hosted Agent".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "second-self-serve-submit".to_string(),
                now: Some("2026-05-25T15:00:00Z".to_string()),
            })
            .unwrap_err();
        assert!(matches!(
            exhausted,
            CoreError::AgentCreationEntitlementExhausted
        ));

        assert_eq!(state.agent_creation_requests.len(), 1);
        assert_eq!(
            state.project_import_candidates[&candidate_id],
            imported_candidate
        );
        assert_eq!(state.projects[&imported_project_id], imported_project);
        assert_eq!(state.agent_runtimes[&imported_runtime_id], imported_runtime);
        assert_eq!(
            state.project_runtime_links[&imported_link.id],
            imported_link
        );
    }

    #[test]
    fn paid_self_serve_agent_creation_requires_active_stripe_billing() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);

        let unpaid = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "paid@finite.vip".to_string(),
                workos_user_id: "user_workos_paid".to_string(),
                display_name: "Paid Agent".to_string(),
                launch_code: String::new(),
                idempotency_key: "paid-submit-before-billing".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap_err();
        assert!(matches!(unpaid, CoreError::BillingRequired));
        assert!(state.users.is_empty());
        assert!(state.customer_orgs.is_empty());

        state
            .link_stripe_customer(LinkStripeCustomerInput {
                verified_email: "paid@finite.vip".to_string(),
                workos_user_id: "user_workos_paid".to_string(),
                stripe_customer_id: "cus_paid".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let org_id = state
            .find_personal_org_by_owner(&state.find_user_by_email("paid@finite.vip").unwrap().id)
            .unwrap()
            .id;
        state
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: "cus_paid".to_string(),
                stripe_subscription_id: "sub_paid".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::Active,
                current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_paid_active".to_string()),
                stripe_event_created: None,
                now: Some(NOW.to_string()),
            })
            .unwrap();

        let overview = state
            .billing_overview(LinkVerifiedUserInput {
                verified_email: "paid@finite.vip".to_string(),
                workos_user_id: "user_workos_paid".to_string(),
                now: Some(LATER.to_string()),
            })
            .unwrap();
        assert!(overview.can_create_agent);
        assert!(!overview.requires_billing);
        assert_eq!(
            overview
                .agent_creation_entitlement
                .as_ref()
                .and_then(|entitlement| entitlement.launch_code.as_deref()),
            None
        );

        let created = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "paid@finite.vip".to_string(),
                workos_user_id: "user_workos_paid".to_string(),
                display_name: "Paid Agent".to_string(),
                launch_code: String::new(),
                idempotency_key: "paid-submit".to_string(),
                now: Some(LATER.to_string()),
            })
            .unwrap();
        assert_eq!(created.request.requested_launch_code, None);
        assert_eq!(
            state.customer_orgs[&org_id].billing_class,
            BillingClass::Standard
        );
        let lease = state
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-paid-1".to_string(),
                source_host_id: None,
                lease_token: "paid-lease-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some("2026-05-25T13:01:00Z".to_string()),
            })
            .unwrap()
            .expect("paid request should be leased");
        let provisioned = state
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-paid-1".to_string(),
                lease_token: "paid-lease-1".to_string(),
                source_host_id: Some("paid-host-1".to_string()),
                source_machine_id: Some("paid-agent-001".to_string()),
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .unwrap();
        let completed = state
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-paid-1".to_string(),
                lease_token: "paid-lease-1".to_string(),
                source_host_id: "paid-host-1".to_string(),
                source_machine_id: "paid-agent-001".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
                display_name: None,
                hostname: None,
                runtime_host: Some("paid-host-1".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: vec!["https://paid-agent.example.com/contact".to_string()],
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .unwrap();
        let runtime_id = completed.request.agent_runtime_id.unwrap();
        assert!(state.agent_runtimes.contains_key(&runtime_id));

        state
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id),
                stripe_customer_id: "cus_paid".to_string(),
                stripe_subscription_id: "sub_paid".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::PastDue,
                current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_paid_past_due".to_string()),
                stripe_event_created: None,
                now: Some("2026-05-25T14:00:00Z".to_string()),
            })
            .unwrap();
        let blocked_after_past_due = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "paid@finite.vip".to_string(),
                workos_user_id: "user_workos_paid".to_string(),
                display_name: "Second Paid Agent".to_string(),
                launch_code: String::new(),
                idempotency_key: "paid-submit-2".to_string(),
                now: Some("2026-05-25T14:01:00Z".to_string()),
            })
            .unwrap_err();
        assert!(matches!(blocked_after_past_due, CoreError::BillingRequired));
        assert!(state.agent_runtimes.contains_key(&runtime_id));
        assert!(
            state
                .project_runtime_links
                .values()
                .any(|link| link.agent_runtime_id == runtime_id && link.active)
        );
        assert_eq!(
            state.finite_private_api_keys[&provisioned.api_key.id].status,
            FinitePrivateApiKeyStatus::Active
        );
    }

    #[test]
    fn stripe_subscription_sync_ignores_non_current_subscription_events() {
        let mut state = BridgeCoreState::default();
        state
            .link_stripe_customer(LinkStripeCustomerInput {
                verified_email: "paid@finite.vip".to_string(),
                workos_user_id: "user_workos_paid".to_string(),
                stripe_customer_id: "cus_paid".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let org_id = state
            .find_personal_org_by_owner(&state.find_user_by_email("paid@finite.vip").unwrap().id)
            .unwrap()
            .id;
        let current = state
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: "cus_paid".to_string(),
                stripe_subscription_id: "sub_current".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::Active,
                current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_current_active".to_string()),
                stripe_event_created: None,
                now: Some(NOW.to_string()),
            })
            .unwrap();
        assert_eq!(
            current.stripe_subscription_id.as_deref(),
            Some("sub_current")
        );

        let ignored = state
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: "cus_paid".to_string(),
                stripe_subscription_id: "sub_second".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::Active,
                current_period_end: Some("2026-07-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_second_active".to_string()),
                stripe_event_created: None,
                now: Some(LATER.to_string()),
            })
            .unwrap();
        assert_eq!(
            ignored.stripe_subscription_id.as_deref(),
            Some("sub_current")
        );
        assert_eq!(
            state.customer_billing_accounts[&org_id]
                .last_stripe_event_id
                .as_deref(),
            Some("evt_current_active")
        );

        state
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: "cus_paid".to_string(),
                stripe_subscription_id: "sub_current".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::Canceled,
                current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_current_canceled".to_string()),
                stripe_event_created: None,
                now: Some("2026-05-25T14:00:00Z".to_string()),
            })
            .unwrap();

        let replacement = state
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: "cus_paid".to_string(),
                stripe_subscription_id: "sub_replacement".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::Active,
                current_period_end: Some("2026-08-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_replacement_active".to_string()),
                stripe_event_created: None,
                now: Some("2026-05-25T15:00:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(
            replacement.stripe_subscription_id.as_deref(),
            Some("sub_replacement")
        );

        let old_event = state
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: "cus_paid".to_string(),
                stripe_subscription_id: "sub_current".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::PastDue,
                current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_current_late_past_due".to_string()),
                stripe_event_created: None,
                now: Some("2026-05-25T16:00:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(
            old_event.stripe_subscription_id.as_deref(),
            Some("sub_replacement")
        );
        assert_eq!(
            state.customer_billing_accounts[&org_id]
                .subscription_status
                .unwrap(),
            BillingSubscriptionStatus::Active
        );
    }

    #[test]
    fn stripe_subscription_sync_ignores_stale_out_of_order_event() {
        // Event-ordering guard: for the SAME subscription, a webhook whose Stripe
        // `event.created` predates the last applied event must be ignored, so a
        // stale `active` delivered after `canceled` cannot resurrect billing.
        let mut state = BridgeCoreState::default();
        state
            .link_stripe_customer(LinkStripeCustomerInput {
                verified_email: "order@finite.vip".to_string(),
                workos_user_id: "user_workos_order".to_string(),
                stripe_customer_id: "cus_order".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let org_id = state
            .find_personal_org_by_owner(&state.find_user_by_email("order@finite.vip").unwrap().id)
            .unwrap()
            .id;

        let mut sync = |status: BillingSubscriptionStatus, event: &str, created: i64| {
            state
                .sync_stripe_subscription(SyncStripeSubscriptionInput {
                    customer_org_id: Some(org_id.clone()),
                    stripe_customer_id: "cus_order".to_string(),
                    stripe_subscription_id: "sub_order".to_string(),
                    stripe_price_id: Some("price_standard".to_string()),
                    expected_stripe_price_id: Some("price_standard".to_string()),
                    subscription_status: status,
                    current_period_end: Some("2026-08-01T12:00:00Z".to_string()),
                    cancel_at_period_end: false,
                    stripe_event_id: Some(event.to_string()),
                    stripe_event_created: Some(created),
                    now: Some(NOW.to_string()),
                })
                .unwrap()
        };

        sync(BillingSubscriptionStatus::Active, "evt_active", 1_000);
        let canceled = sync(BillingSubscriptionStatus::Canceled, "evt_canceled", 2_000);
        assert_eq!(
            canceled.subscription_status,
            Some(BillingSubscriptionStatus::Canceled)
        );

        // Stale `active` (created BEFORE the canceled event) arrives last.
        let stale = sync(BillingSubscriptionStatus::Active, "evt_active_stale", 1_500);
        assert_eq!(
            stale.subscription_status,
            Some(BillingSubscriptionStatus::Canceled),
            "stale out-of-order webhook must be ignored; billing stays canceled"
        );
        assert_eq!(stale.last_stripe_event_id.as_deref(), Some("evt_canceled"));
    }

    #[test]
    fn stripe_subscription_sync_requires_standard_price_before_entitlement() {
        let mut state = BridgeCoreState::default();
        state
            .link_stripe_customer(LinkStripeCustomerInput {
                verified_email: "paid@finite.vip".to_string(),
                workos_user_id: "user_workos_paid".to_string(),
                stripe_customer_id: "cus_paid".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let org_id = state
            .find_personal_org_by_owner(&state.find_user_by_email("paid@finite.vip").unwrap().id)
            .unwrap()
            .id;

        let wrong_price = state
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: "cus_paid".to_string(),
                stripe_subscription_id: "sub_wrong_price".to_string(),
                stripe_price_id: Some("price_other".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::Active,
                current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_wrong_price_active".to_string()),
                stripe_event_created: None,
                now: Some(NOW.to_string()),
            })
            .unwrap_err();
        assert!(matches!(
            wrong_price,
            CoreError::StripeSubscriptionPriceMismatch
        ));
        assert!(state.agent_creation_entitlements.is_empty());

        let missing_expected_price = state
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id),
                stripe_customer_id: "cus_paid".to_string(),
                stripe_subscription_id: "sub_missing_expected".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: None,
                subscription_status: BillingSubscriptionStatus::Trialing,
                current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_missing_expected_trialing".to_string()),
                stripe_event_created: None,
                now: Some(LATER.to_string()),
            })
            .unwrap_err();
        assert!(matches!(
            missing_expected_price,
            CoreError::MissingStripeStandardPriceId
        ));
        assert!(state.agent_creation_entitlements.is_empty());
    }

    #[test]
    fn stripe_subscription_lapse_preserves_launch_code_entitlement() {
        let mut state = BridgeCoreState::default();
        state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "bridge@finite.vip".to_string(),
                workos_user_id: "user_workos_bridge".to_string(),
                display_name: "Bridge Agent".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "bridge-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let org_id = state
            .find_personal_org_by_owner(&state.find_user_by_email("bridge@finite.vip").unwrap().id)
            .unwrap()
            .id;
        assert_eq!(
            state
                .agent_creation_entitlements
                .values()
                .find(|entitlement| entitlement.customer_org_id == org_id)
                .and_then(|entitlement| entitlement.launch_code.as_deref()),
            Some("off2026")
        );

        state
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: "cus_bridge".to_string(),
                stripe_subscription_id: "sub_bridge".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::Active,
                current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_bridge_active".to_string()),
                stripe_event_created: None,
                now: Some(LATER.to_string()),
            })
            .unwrap();
        assert_eq!(
            state.agent_creation_entitlements[&agent_creation_entitlement_id_for(&org_id)]
                .launch_code
                .as_deref(),
            Some("off2026")
        );

        state
            .sync_stripe_subscription(SyncStripeSubscriptionInput {
                customer_org_id: Some(org_id.clone()),
                stripe_customer_id: "cus_bridge".to_string(),
                stripe_subscription_id: "sub_bridge".to_string(),
                stripe_price_id: Some("price_standard".to_string()),
                expected_stripe_price_id: Some("price_standard".to_string()),
                subscription_status: BillingSubscriptionStatus::PastDue,
                current_period_end: Some("2026-06-25T12:00:00Z".to_string()),
                cancel_at_period_end: false,
                stripe_event_id: Some("evt_bridge_past_due".to_string()),
                stripe_event_created: None,
                now: Some("2026-05-25T14:00:00Z".to_string()),
            })
            .unwrap();
        let entitlement =
            &state.agent_creation_entitlements[&agent_creation_entitlement_id_for(&org_id)];
        assert_eq!(entitlement.launch_code.as_deref(), Some("off2026"));
        assert_eq!(entitlement.allowed_new_agent_runtimes, 1);
    }

    #[test]
    fn finite_private_runtime_key_provisioning_is_bound_to_launching_request() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);
        let requested = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let lease = state
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                source_host_id: None,
                lease_token: "lease-token-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .unwrap()
            .expect("request should be leased");

        let provisioned = state
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                source_host_id: Some("oslo-host-1".to_string()),
                source_machine_id: Some("finite-agent_123".to_string()),
                now: Some("2026-05-25T13:01:00Z".to_string()),
            })
            .unwrap();

        assert!(provisioned.raw_api_key.starts_with("fpk_live_"));
        assert_eq!(provisioned.grant.status, FinitePrivateGrantStatus::Active);
        assert_eq!(
            provisioned.api_key.project_id.as_deref(),
            Some(requested.project.id.as_str())
        );
        assert!(provisioned.api_key.agent_runtime_id.is_none());
        assert!(
            !serde_json::to_string(&state.finite_private_api_keys)
                .unwrap()
                .contains(&provisioned.raw_api_key)
        );

        let wrong_lease = state
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id: lease.request.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "wrong-token".to_string(),
                source_host_id: Some("oslo-host-1".to_string()),
                source_machine_id: Some("finite-agent_123".to_string()),
                now: Some("2026-05-25T13:01:00Z".to_string()),
            })
            .unwrap_err();
        assert!(matches!(
            wrong_lease,
            CoreError::AgentCreationRequestLeaseConflict
        ));
    }

    #[test]
    fn finite_private_reserve_and_settle_keeps_core_as_usage_authority() {
        let mut state = BridgeCoreState::default();
        let grant = state
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "private@finite.vip".to_string(),
                workos_user_id: Some("user_workos_private".to_string()),
                limit_profile_id: None,
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let key = state
            .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id.clone(),
                raw_key: "fpk_live_secret".to_string(),
                project_id: Some("project-private".to_string()),
                agent_runtime_id: Some("runtime-private".to_string()),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        assert_ne!(key.key_hash, "fpk_live_secret");
        assert!(
            !serde_json::to_string(&state.finite_private_api_keys)
                .unwrap()
                .contains("fpk_live_secret")
        );

        let reserved = state
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: "req-private-1".to_string(),
                presented_api_key: "fpk_live_secret".to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "kimi-k2-6".to_string(),
                estimated_prompt_tokens: 120_000,
                estimated_completion_tokens: 4_096,
                estimated_usage_units: 250_000,
                usage_formula_version: "2026-05-26.v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: Some("2026-05-25T13:00:00Z".to_string()),
            })
            .unwrap();

        assert_eq!(reserved.decision, "allow");
        assert_eq!(reserved.burst_limit_units, Some(5_000_000));
        assert_eq!(reserved.burst_remaining_units, Some(4_750_000));
        assert_eq!(
            reserved.weekly_limit_units,
            Some(DEFAULT_FINITE_PRIVATE_WEEKLY_LIMIT_UNITS)
        );
        assert_eq!(
            reserved.weekly_remaining_units,
            Some(DEFAULT_FINITE_PRIVATE_WEEKLY_LIMIT_UNITS - 250_000)
        );
        let reservation_id = reserved.reservation_id.clone().unwrap();
        assert_eq!(
            state.finite_private_grants[&grant.id].current_window_used_units,
            250_000
        );

        let settled = state
            .settle_finite_private_reservation(SettleFinitePrivateReservationInput {
                reservation_id: reservation_id.clone(),
                request_id: "req-private-1".to_string(),
                settlement: FinitePrivateSettlementKind::Actual,
                prompt_tokens: Some(120_000),
                completion_tokens: Some(1_200),
                usage_units: Some(160_000),
                usage_formula_version: "2026-05-26.v1".to_string(),
                upstream_status: Some(200),
                upstream_error_class: None,
                now: Some("2026-05-25T13:05:00Z".to_string()),
            })
            .unwrap();

        assert!(settled.settled);
        assert_eq!(
            state.finite_private_grants[&grant.id].current_window_used_units,
            160_000
        );
        let reservation = &state.finite_private_reservations[&reservation_id];
        assert_eq!(reservation.status, FinitePrivateReservationStatus::Settled);
        assert_eq!(
            reservation.settlement_kind,
            Some(FinitePrivateSettlementKind::Actual)
        );
        assert_eq!(reservation.settled_usage_units, Some(160_000));
    }

    #[test]
    fn finite_private_grant_can_start_as_pending_email_and_later_link_workos() {
        let mut state = BridgeCoreState::default();
        let pending_grant = state
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "friend@finite.vip".to_string(),
                workos_user_id: None,
                limit_profile_id: None,
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let pending_user = state.users.get(&pending_grant.user_id).unwrap();
        assert_eq!(pending_user.email, "friend@finite.vip");
        assert_eq!(pending_user.status, UserLinkStatus::Pending);
        assert_eq!(pending_user.workos_user_id, None);

        let linked_grant = state
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "friend@finite.vip".to_string(),
                workos_user_id: Some("user_workos_friend".to_string()),
                limit_profile_id: None,
                now: Some("2026-05-26T13:00:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(linked_grant.id, pending_grant.id);
        let linked_user = state.users.get(&linked_grant.user_id).unwrap();
        assert_eq!(linked_user.status, UserLinkStatus::Linked);
        assert_eq!(
            linked_user.workos_user_id.as_deref(),
            Some("user_workos_friend")
        );
    }

    #[test]
    fn finite_private_admin_operations_write_audit_events_without_raw_keys() {
        let mut state = BridgeCoreState::default();
        let grant = state
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "friend@finite.vip".to_string(),
                workos_user_id: None,
                limit_profile_id: None,
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let key = state
            .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id.clone(),
                raw_key: "fpk_live_first_secret".to_string(),
                project_id: Some("project_friend".to_string()),
                agent_runtime_id: None,
                now: Some("2026-05-26T12:01:00Z".to_string()),
            })
            .unwrap();
        state
            .reset_finite_private_usage_window(ResetFinitePrivateUsageWindowInput {
                grant_id: grant.id.clone(),
                now: Some("2026-05-26T12:02:00Z".to_string()),
            })
            .unwrap();
        let rotated = state
            .rotate_finite_private_api_key(RotateFinitePrivateApiKeyInput {
                key_id: key.id.clone(),
                raw_key: "fpk_live_second_secret".to_string(),
                now: Some("2026-05-26T12:03:00Z".to_string()),
            })
            .unwrap();
        state
            .revoke_finite_private_grant(RevokeFinitePrivateGrantInput {
                grant_id: grant.id.clone(),
                now: Some("2026-05-26T12:04:00Z".to_string()),
            })
            .unwrap();

        let actions = state
            .finite_private_admin_audit_events
            .values()
            .map(|event| event.action.as_str())
            .collect::<BTreeSet<_>>();
        for expected in [
            "finite_private.grant.approve",
            "finite_private.api_key.issue",
            "finite_private.grant.reset_window",
            "finite_private.api_key.rotate",
            "finite_private.grant.revoke",
        ] {
            assert!(actions.contains(expected));
        }
        assert_eq!(
            state
                .finite_private_admin_audit_events
                .values()
                .filter(|event| event.grant_id.as_deref() == Some(grant.id.as_str()))
                .count(),
            state.finite_private_admin_audit_events.len()
        );
        assert_eq!(
            state.finite_private_api_keys[&rotated.id].status,
            FinitePrivateApiKeyStatus::Revoked
        );
        let audit_json = serde_json::to_string(&state.finite_private_admin_audit_events).unwrap();
        assert!(!audit_json.contains("fpk_live_first_secret"));
        assert!(!audit_json.contains("fpk_live_second_secret"));
    }

    #[test]
    fn finite_private_reserve_denies_unknown_key_and_over_limit_without_upstream_work() {
        let mut state = BridgeCoreState::default();
        let unknown = state
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: "req-private-unknown".to_string(),
                presented_api_key: "fpk_live_unknown".to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "kimi-k2-6".to_string(),
                estimated_prompt_tokens: 100,
                estimated_completion_tokens: 100,
                estimated_usage_units: 200,
                usage_formula_version: "2026-05-26.v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: Some("2026-05-25T13:00:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(unknown.decision, "deny");
        assert_eq!(
            unknown.error.as_ref().map(|error| error.code.as_str()),
            Some("invalid_api_key")
        );
        assert!(state.finite_private_reservations.is_empty());

        let grant = state
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "private@finite.vip".to_string(),
                workos_user_id: Some("user_workos_private".to_string()),
                limit_profile_id: None,
                now: Some(NOW.to_string()),
            })
            .unwrap();
        state
            .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id.clone(),
                raw_key: "fpk_live_secret".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: Some(NOW.to_string()),
            })
            .unwrap();

        let denied = state
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: "req-private-over".to_string(),
                presented_api_key: "fpk_live_secret".to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "kimi-k2-6".to_string(),
                estimated_prompt_tokens: 5_000_001,
                estimated_completion_tokens: 0,
                estimated_usage_units: 5_000_001,
                usage_formula_version: "2026-05-26.v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: Some("2026-05-25T13:00:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(denied.decision, "deny");
        assert_eq!(
            denied.error.as_ref().map(|error| error.code.as_str()),
            Some("burst_window_limit_exceeded")
        );
        assert_eq!(
            state.finite_private_grants[&grant.id].current_window_used_units,
            0
        );
        assert!(state.finite_private_reservations.is_empty());
    }

    #[test]
    fn finite_private_weekly_limit_denies_without_upstream_work() {
        let mut state = BridgeCoreState::default();
        state.finite_private_limit_profiles.insert(
            "weekly-small".to_string(),
            FinitePrivateLimitProfile {
                id: "weekly-small".to_string(),
                burst_window_seconds: 3600,
                burst_limit_units: 10_000_000,
                weekly_limit_units: Some(1_000),
                created_at: NOW.to_string(),
                updated_at: NOW.to_string(),
            },
        );
        let grant = state
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "private@finite.vip".to_string(),
                workos_user_id: Some("user_workos_private".to_string()),
                limit_profile_id: Some("weekly-small".to_string()),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        state
            .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id.clone(),
                raw_key: "fpk_live_secret".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: Some(NOW.to_string()),
            })
            .unwrap();

        let allowed = state
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: "req-private-weekly-1".to_string(),
                presented_api_key: "fpk_live_secret".to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "glm-5.2".to_string(),
                estimated_prompt_tokens: 800,
                estimated_completion_tokens: 0,
                estimated_usage_units: 800,
                usage_formula_version: "2026-05-26.v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: Some("2026-05-25T13:00:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(allowed.decision, "allow");
        assert_eq!(allowed.weekly_remaining_units, Some(200));

        let denied = state
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: "req-private-weekly-2".to_string(),
                presented_api_key: "fpk_live_secret".to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "glm-5.2".to_string(),
                estimated_prompt_tokens: 300,
                estimated_completion_tokens: 0,
                estimated_usage_units: 300,
                usage_formula_version: "2026-05-26.v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: Some("2026-05-26T13:00:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(denied.decision, "deny");
        assert_eq!(
            denied.error.as_ref().map(|error| error.code.as_str()),
            Some("weekly_limit_exceeded")
        );
        assert_eq!(state.finite_private_reservations.len(), 1);
    }

    #[test]
    fn schema_is_postgres_first_and_contains_first_bridge_tables() {
        for table in [
            "users",
            "customer_orgs",
            "project_import_candidates",
            "projects",
            "runtime_artifacts",
            "agent_runtimes",
            "runtime_relay_credentials",
            "project_runtime_links",
            "chat_identities",
            "project_room_memberships",
            "runtime_status_snapshots",
            "inference_profiles",
            "agent_creation_entitlements",
            "agent_creation_requests",
            "customer_billing_accounts",
            "finite_private_limit_profiles",
            "finite_private_grants",
            "finite_private_api_keys",
            "finite_private_admin_audit_events",
            "finite_private_reservations",
        ] {
            assert!(CORE_SCHEMA_SQL.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")));
        }

        assert!(CORE_SCHEMA_SQL.contains("JSONB"));
        assert!(CORE_SCHEMA_SQL.contains("TIMESTAMPTZ"));
        assert!(!CORE_SCHEMA_SQL.to_lowercase().contains("sqlite"));
    }

    fn options<const N: usize>(
        allowlisted_owner_emails: [&str; N],
        now: &str,
    ) -> ReconcileExistingHostImportsOptions {
        ReconcileExistingHostImportsOptions {
            allowlisted_owner_emails: allowlisted_owner_emails
                .iter()
                .map(|email| email.to_string())
                .collect(),
            now: Some(now.to_string()),
        }
    }

    fn import(
        source_host_id: &str,
        source_machine_id: &str,
        display_name: &str,
        owner_email: Option<&str>,
    ) -> ExistingHostProjectImport {
        ExistingHostProjectImport {
            source_host_id: source_host_id.to_string(),
            source_machine_id: source_machine_id.to_string(),
            owner_email: owner_email.map(str::to_string),
            display_name: display_name.to_string(),
            hostname: None,
            runtime_host: None,
            runtime_status: RuntimeSummaryStatus::Unknown,
            active_inference_profile: None,
            hermes_available: None,
            published_app_urls: Vec::new(),
            known_external_channel_participants: Vec::new(),
            admin_visible_to_emails: Vec::new(),
        }
    }

    #[test]
    fn admin_runtime_control_skips_owner_check_and_matches_runner_lease_shape() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);
        let runtime_id = complete_self_serve_agent(
            &mut state,
            "owner@finite.vip",
            "user_workos_owner",
            "first-submit",
            "oslo-agent-001",
            "artifact-v1",
            "2026-05-25T13:02:00Z",
        );
        let project_id = state.agent_runtimes[&runtime_id].project_id.clone();

        // The owner-scoped path rejects non-owners outright.
        let denied = state
            .request_runtime_restart(RequestRuntimeRestartInput {
                verified_email: "admin@finite.vip".to_string(),
                workos_user_id: "user_workos_admin".to_string(),
                project_id: project_id.clone(),
                now: Some("2026-05-25T13:03:00Z".to_string()),
            })
            .unwrap_err();
        assert!(matches!(denied, CoreError::ProjectNotFound));

        // The admin path creates the request without owning the project.
        let restart = state
            .admin_request_runtime_restart(AdminRuntimeControlInput {
                admin_verified_email: "Admin@Finite.VIP".to_string(),
                admin_workos_user_id: "user_workos_admin".to_string(),
                project_id: project_id.clone(),
                now: Some("2026-05-25T13:03:30Z".to_string()),
            })
            .unwrap();
        assert_eq!(restart.project_id, project_id);
        assert_eq!(restart.agent_runtime_id, runtime_id);
        assert_eq!(restart.source_host_id, "oslo-host-1");
        assert_eq!(restart.source_machine_id, "oslo-agent-001");
        assert_eq!(restart.kind, RuntimeControlKind::Restart);
        assert_eq!(restart.status, RuntimeControlRequestStatus::Requested);
        assert_eq!(
            restart.requested_by_user_id,
            state.find_user_by_email("admin@finite.vip").unwrap().id
        );

        // Idempotent while an equivalent request is pending, like the owner path.
        let duplicate = state
            .admin_request_runtime_restart(AdminRuntimeControlInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "user_workos_admin".to_string(),
                project_id: project_id.clone(),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(duplicate.id, restart.id);

        // The runner consumes it through the exact same lease machinery.
        let lease = state
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "admin-restart-lease-1".to_string(),
                lease_seconds: Some(60),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: None,
                now: Some("2026-05-25T13:04:30Z".to_string()),
            })
            .unwrap()
            .expect("admin restart request should lease");
        assert_eq!(lease.request.id, restart.id);
        assert_eq!(lease.request.status, RuntimeControlRequestStatus::Running);
        assert_eq!(lease.runtime.source_machine_id, "oslo-agent-001");
        let completed = state
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: restart.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "admin-restart-lease-1".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_host: None,
                published_app_urls: None,
                now: Some("2026-05-25T13:05:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(completed.status, RuntimeControlRequestStatus::Succeeded);

        // Recover uses the same machinery and records its own audit action.
        let recover = state
            .admin_request_runtime_recover_known_good_chat(AdminRuntimeControlInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "user_workos_admin".to_string(),
                project_id,
                now: Some("2026-05-25T13:06:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(
            recover.kind,
            RuntimeControlKind::RecoverKnownGoodChatRuntime
        );

        let actions = state
            .finite_private_admin_audit_events
            .values()
            .map(|event| (event.action.clone(), event.actor.clone()))
            .collect::<Vec<_>>();
        assert!(actions.contains(&(
            "runtime.admin_restart".to_string(),
            "admin@finite.vip".to_string()
        )));
        assert!(actions.contains(&(
            "runtime.admin_recover_known_good_chat".to_string(),
            "admin@finite.vip".to_string()
        )));
    }

    #[test]
    fn admin_friend_key_issue_mirrors_cli_and_records_admin_audit() {
        let mut state = BridgeCoreState::default();
        let raw_key = "fpk_live_test_friend_key_material_0001";
        let issued = state
            .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                friend_email: "Friend@Finite.VIP".to_string(),
                limit_profile_id: None,
                raw_key: raw_key.to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();

        assert_eq!(issued.grant.status, FinitePrivateGrantStatus::Active);
        assert_eq!(issued.grant.limit_profile_id, "finite-private-generous");
        assert_eq!(issued.api_key.status, FinitePrivateApiKeyStatus::Active);
        assert_ne!(issued.api_key.key_hash, raw_key);
        assert!(issued.api_key.project_id.is_none());
        assert!(issued.api_key.agent_runtime_id.is_none());

        let resolved = state
            .finite_private_key_and_grant(raw_key)
            .unwrap()
            .expect("issued raw key should validate");
        assert_eq!(resolved.0.id, issued.api_key.id);
        assert_eq!(resolved.1.id, issued.grant.id);

        let admin_event = state
            .finite_private_admin_audit_events
            .values()
            .find(|event| event.action == "finite_private.friend_key.admin_issue")
            .expect("friend key issue should record an admin audit event");
        assert_eq!(admin_event.actor, "admin@finite.vip");
        assert_eq!(
            admin_event.api_key_id.as_deref(),
            Some(issued.api_key.id.as_str())
        );
    }

    #[test]
    fn admin_rotate_invalidates_old_raw_key_and_revoke_disables_key() {
        let mut state = BridgeCoreState::default();
        let old_raw = "fpk_live_old_raw_key_material_000000001";
        let issued = state
            .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                friend_email: "friend@finite.vip".to_string(),
                limit_profile_id: None,
                raw_key: old_raw.to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();

        let new_raw = "fpk_live_new_raw_key_material_000000002";
        let rotated = state
            .admin_rotate_finite_private_api_key(AdminRotateFinitePrivateApiKeyInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                key_id: issued.api_key.id.clone(),
                raw_key: new_raw.to_string(),
                now: Some(LATER.to_string()),
            })
            .unwrap();
        assert_ne!(rotated.id, issued.api_key.id);
        assert_eq!(rotated.status, FinitePrivateApiKeyStatus::Active);

        assert!(
            state
                .finite_private_key_and_grant(old_raw)
                .unwrap()
                .is_none(),
            "old raw key must stop validating after rotate"
        );
        let resolved = state
            .finite_private_key_and_grant(new_raw)
            .unwrap()
            .expect("new raw key should validate");
        assert_eq!(resolved.0.id, rotated.id);
        assert_eq!(
            state.finite_private_api_keys[&issued.api_key.id].status,
            FinitePrivateApiKeyStatus::Revoked
        );

        let revoked = state
            .admin_revoke_finite_private_api_key(AdminRevokeFinitePrivateApiKeyInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                key_id: rotated.id.clone(),
                now: Some("2026-05-25T14:00:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(revoked.status, FinitePrivateApiKeyStatus::Revoked);
        assert!(
            state
                .finite_private_key_and_grant(new_raw)
                .unwrap()
                .is_none()
        );

        let actions = state
            .finite_private_admin_audit_events
            .values()
            .filter(|event| event.actor == "admin@finite.vip")
            .map(|event| event.action.clone())
            .collect::<Vec<_>>();
        assert!(actions.contains(&"finite_private.api_key.admin_rotate".to_string()));
        assert!(actions.contains(&"finite_private.api_key.admin_revoke".to_string()));
    }

    #[test]
    fn admin_window_reset_clears_burst_window_but_not_weekly_reservations() {
        let mut state = BridgeCoreState::default();
        let raw_key = "fpk_live_reset_raw_key_material_00000003";
        let issued = state
            .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                friend_email: "friend@finite.vip".to_string(),
                limit_profile_id: None,
                raw_key: raw_key.to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();

        let decision = state
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: "req-1".to_string(),
                presented_api_key: raw_key.to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "kimi-k2-6".to_string(),
                estimated_prompt_tokens: 10,
                estimated_completion_tokens: 10,
                estimated_usage_units: 1_000,
                usage_formula_version: "2026-05-26.v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: Some(LATER.to_string()),
            })
            .unwrap();
        assert_eq!(decision.decision, "allow");
        assert_eq!(
            state.finite_private_grants[&issued.grant.id].current_window_used_units,
            1_000
        );

        let reset = state
            .admin_reset_finite_private_usage_window(AdminResetFinitePrivateUsageWindowInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                grant_id: issued.grant.id.clone(),
                now: Some("2026-05-25T14:00:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(reset.current_window_used_units, 0);
        assert!(reset.current_window_started_at.is_none());

        // Weekly usage is a rolling reservation window; reset must not touch it.
        let (weekly_used, _) = state
            .finite_private_weekly_usage(
                &issued.grant.id,
                parse_time("2026-05-25T14:00:00Z").unwrap(),
            )
            .unwrap();
        assert_eq!(weekly_used, 1_000);

        let admin_event = state
            .finite_private_admin_audit_events
            .values()
            .find(|event| event.action == "finite_private.grant.admin_window_reset")
            .expect("window reset should record an admin audit event");
        assert_eq!(admin_event.actor, "admin@finite.vip");
    }

    #[test]
    fn admin_runtime_overviews_assemble_provisioned_box_facts() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);
        let runtime_id = complete_self_serve_agent(
            &mut state,
            "owner@finite.vip",
            "user_workos_owner",
            "first-submit",
            "oslo-agent-001",
            "artifact-v1",
            "2026-05-25T13:02:00Z",
        );
        let project_id = state.agent_runtimes[&runtime_id].project_id.clone();
        let grant = state
            .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                verified_email: "owner@finite.vip".to_string(),
                workos_user_id: Some("user_workos_owner".to_string()),
                limit_profile_id: None,
                now: Some(LATER.to_string()),
            })
            .unwrap();
        state
            .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id.clone(),
                raw_key: "fpk_live_overview_key_material_00000004".to_string(),
                project_id: Some(project_id.clone()),
                agent_runtime_id: Some(runtime_id.clone()),
                now: Some(LATER.to_string()),
            })
            .unwrap();

        let overviews = state.admin_runtime_overviews();
        assert_eq!(overviews.len(), 1);
        let overview = &overviews[0];
        assert_eq!(overview.project_id, project_id);
        assert_eq!(overview.agent_runtime_id, runtime_id);
        assert_eq!(overview.owner_email.as_deref(), Some("owner@finite.vip"));
        assert_eq!(overview.source_host_id, "oslo-host-1");
        assert_eq!(overview.source_machine_id, "oslo-agent-001");
        assert_eq!(overview.runtime_artifact_id.as_deref(), Some("artifact-v1"));
        assert_eq!(
            overview.runtime_artifact_version_label.as_deref(),
            Some("v1")
        );
        assert_eq!(overview.runtime_status, RuntimeSummaryStatus::Online);
        assert_eq!(overview.hermes_available, Some(true));
        assert_eq!(overview.active_finite_private_key_count, 1);
        assert!(overview.runtime_link_active);
        assert!(overview.supports_runtime_control);
    }

    #[test]
    fn explicit_kata_upgrade_binds_compatible_artifact_and_commits_actual_facts_atomically() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);
        let requested = state
            .request_agent_creation_configured(
                RequestAgentCreationInput {
                    verified_email: "upgrade@finite.vip".to_string(),
                    workos_user_id: "workos-upgrade".to_string(),
                    display_name: "Upgrade Agent".to_string(),
                    launch_code: "off2026".to_string(),
                    idempotency_key: "upgrade-agent".to_string(),
                    now: Some(NOW.to_string()),
                },
                AgentCreationConfiguration {
                    runner_class: RunnerClass::Kata,
                    profile_picture_url: None,
                },
            )
            .unwrap();
        state
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "kata-runner".to_string(),
                source_host_id: None,
                lease_token: "launch-lease".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some(LATER.to_string()),
            })
            .unwrap()
            .unwrap();
        let completed = state
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: requested.request.id,
                runner_id: "kata-runner".to_string(),
                lease_token: "launch-lease".to_string(),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "finite-kata-upgrade".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
                display_name: None,
                hostname: None,
                runtime_host: Some("http://127.0.0.1:41001".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: vec!["http://127.0.0.1:41001/contact".to_string()],
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .unwrap();
        let runtime_id = completed.request.agent_runtime_id.unwrap();
        state.runtime_relay_credentials.insert(
            runtime_id.clone(),
            RuntimeRelayCredential {
                agent_runtime_id: runtime_id.clone(),
                token_hash: "existing-relay-token-hash".to_string(),
                created_at: "2026-05-25T13:02:00Z".to_string(),
                updated_at: "2026-05-25T13:02:00Z".to_string(),
            },
        );
        promote_runtime_artifact_version(
            &mut state,
            "artifact-mutable",
            "ghcr.io/finitecomputer/agent-runtime:latest",
            "mutable",
            "state-v1",
            "2026-05-25T13:02:10Z",
        );
        let mutable = state
            .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-admin".to_string(),
                project_id: requested.project.id.clone(),
                target_runtime_artifact_id: "artifact-mutable".to_string(),
                now: Some("2026-05-25T13:02:20Z".to_string()),
            })
            .unwrap_err();
        assert!(matches!(mutable, CoreError::RuntimeUpgradeUnsupported));
        promote_runtime_artifact_version(
            &mut state,
            "artifact-incompatible",
            &format!(
                "ghcr.io/finitecomputer/agent-runtime:future@sha256:{}",
                "c".repeat(64)
            ),
            "future",
            "state-v2",
            "2026-05-25T13:02:30Z",
        );
        let incompatible = state
            .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-admin".to_string(),
                project_id: requested.project.id.clone(),
                target_runtime_artifact_id: "artifact-incompatible".to_string(),
                now: Some("2026-05-25T13:02:40Z".to_string()),
            })
            .unwrap_err();
        assert!(matches!(
            incompatible,
            CoreError::RuntimeUpgradeStateSchemaIncompatible
        ));
        promote_runtime_artifact_version(
            &mut state,
            "artifact-v2",
            &format!(
                "ghcr.io/finitecomputer/agent-runtime:v2@sha256:{}",
                "b".repeat(64)
            ),
            "v2",
            "state-v1",
            "2026-05-25T13:03:00Z",
        );

        let upgrade = state
            .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-admin".to_string(),
                project_id: requested.project.id.clone(),
                target_runtime_artifact_id: "artifact-v2".to_string(),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .unwrap();
        assert_eq!(upgrade.kind, RuntimeControlKind::Upgrade);
        assert_eq!(
            upgrade.target_runtime_artifact_id.as_deref(),
            Some("artifact-v2")
        );
        let conflicting_stop = state
            .request_runtime_stop(RequestRuntimeStopInput {
                verified_email: "upgrade@finite.vip".to_string(),
                workos_user_id: "workos-upgrade".to_string(),
                project_id: requested.project.id.clone(),
                now: Some("2026-05-25T13:04:30Z".to_string()),
            })
            .unwrap_err();
        assert!(matches!(
            conflicting_stop,
            CoreError::RuntimeControlOperationConflict
        ));
        state
            .runtime_artifacts
            .get_mut("artifact-v2")
            .unwrap()
            .retired_at = Some("2026-05-25T13:04:40Z".to_string());
        let healthy_runtime_id = "runtime-healthy-behind-poison".to_string();
        let mut healthy_runtime = state.agent_runtimes[&runtime_id].clone();
        healthy_runtime.id = healthy_runtime_id.clone();
        healthy_runtime.source_machine_id = "healthy-behind-poison".to_string();
        state
            .agent_runtimes
            .insert(healthy_runtime_id.clone(), healthy_runtime);
        let healthy_request_id = "runtime_ctl_healthy_behind_poison".to_string();
        state.runtime_control_requests.insert(
            healthy_request_id.clone(),
            RuntimeControlRequest {
                id: healthy_request_id.clone(),
                project_id: requested.project.id.clone(),
                agent_runtime_id: healthy_runtime_id,
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "healthy-behind-poison".to_string(),
                requested_by_user_id: "healthy-user".to_string(),
                kind: RuntimeControlKind::Restart,
                target_runtime_artifact_id: None,
                status: RuntimeControlRequestStatus::Requested,
                runner_id: None,
                lease_token: None,
                lease_expires_at: None,
                failure_message: None,
                created_at: "2026-05-25T13:04:45Z".to_string(),
                updated_at: "2026-05-25T13:04:45Z".to_string(),
                completed_at: None,
            },
        );
        let healthy_lease = state
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "kata-runner".to_string(),
                lease_token: "must-not-stick".to_string(),
                lease_seconds: Some(300),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: None,
                now: Some("2026-05-25T13:04:50Z".to_string()),
            })
            .unwrap()
            .expect("poisoned upgrade must not starve the next healthy request");
        assert_eq!(healthy_lease.request.id, healthy_request_id);
        assert_eq!(
            state.runtime_control_requests[&upgrade.id].status,
            RuntimeControlRequestStatus::Failed
        );
        assert!(
            state.runtime_control_requests[&upgrade.id]
                .failure_message
                .as_deref()
                .unwrap_or_default()
                .contains("retired")
        );
        state
            .runtime_artifacts
            .get_mut("artifact-v2")
            .unwrap()
            .retired_at = None;
        let upgrade = state
            .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-admin".to_string(),
                project_id: requested.project.id.clone(),
                target_runtime_artifact_id: "artifact-v2".to_string(),
                now: Some("2026-05-25T13:04:55Z".to_string()),
            })
            .unwrap();
        let lease = state
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "kata-runner".to_string(),
                lease_token: "upgrade-lease".to_string(),
                lease_seconds: Some(300),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some("2026-05-25T13:05:00Z".to_string()),
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            lease
                .target_runtime_artifact
                .as_ref()
                .map(|artifact| artifact.id.as_str()),
            Some("artifact-v2")
        );

        let mismatch = state
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: upgrade.id.clone(),
                runner_id: "kata-runner".to_string(),
                lease_token: "upgrade-lease".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                runtime_host: Some("http://127.0.0.1:41002".to_string()),
                published_app_urls: Some(vec!["http://127.0.0.1:41002/contact".to_string()]),
                now: Some("2026-05-25T13:06:00Z".to_string()),
            })
            .unwrap_err();
        assert!(matches!(
            mismatch,
            CoreError::RuntimeUpgradeCompletionMismatch
        ));
        assert_eq!(
            state.runtime_control_requests[&upgrade.id].status,
            RuntimeControlRequestStatus::Running
        );
        assert_eq!(
            state.agent_runtimes[&runtime_id]
                .runtime_artifact_id
                .as_deref(),
            Some("artifact-v1")
        );

        state
            .runtime_artifacts
            .get_mut("artifact-v2")
            .unwrap()
            .retired_at = Some("2026-05-25T13:06:30Z".to_string());
        state
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: upgrade.id.clone(),
                runner_id: "kata-runner".to_string(),
                lease_token: "upgrade-lease".to_string(),
                runtime_artifact_id: Some("artifact-v2".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                runtime_host: Some("http://127.0.0.1:41002".to_string()),
                published_app_urls: Some(vec!["http://127.0.0.1:41002/contact".to_string()]),
                now: Some("2026-05-25T13:06:40Z".to_string()),
            })
            .unwrap();
        let runtime = &state.agent_runtimes[&runtime_id];
        assert_eq!(runtime.runtime_artifact_id.as_deref(), Some("artifact-v2"));
        assert_eq!(runtime.source_machine_id, "finite-kata-upgrade");
        assert_eq!(runtime.host_facts.runtime_host, "http://127.0.0.1:41002");
        assert!(state.runtime_relay_credentials.contains_key(&runtime_id));
        assert!(
            state
                .project_runtime_links
                .values()
                .any(|link| { link.agent_runtime_id == runtime_id && link.active })
        );
        assert!(state.finite_private_api_keys.values().all(|key| {
            key.agent_runtime_id.as_deref() != Some(runtime_id.as_str())
                || key.status == FinitePrivateApiKeyStatus::Active
        }));
        assert!(
            state
                .finite_private_admin_audit_events
                .values()
                .any(|event| {
                    event.action == "runtime.admin_upgrade"
                        && event.metadata["targetRuntimeArtifactId"] == "artifact-v2"
                })
        );
    }

    #[test]
    fn runtime_upgrade_rejects_non_kata_runtime_before_leasing() {
        let mut state = BridgeCoreState::default();
        promote_runtime_artifact(&mut state);
        let runtime_id = complete_self_serve_agent(
            &mut state,
            "not-kata@finite.vip",
            "workos-not-kata",
            "not-kata",
            "not-kata-runtime",
            "artifact-v1",
            LATER,
        );
        promote_runtime_artifact_version(
            &mut state,
            "artifact-mutable",
            "ghcr.io/finitecomputer/agent-runtime:latest",
            "mutable",
            "state-v1",
            "2026-05-25T13:03:00Z",
        );
        let project_id = state.agent_runtimes[&runtime_id].project_id.clone();
        let error = state
            .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-admin".to_string(),
                project_id,
                target_runtime_artifact_id: "artifact-mutable".to_string(),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .unwrap_err();
        assert!(matches!(error, CoreError::RuntimeUpgradeUnsupported));
        assert!(state.runtime_control_requests.is_empty());
    }

    fn promote_runtime_artifact(state: &mut BridgeCoreState) {
        promote_runtime_artifact_version(
            state,
            "artifact-v1",
            "finite-runtime-v1",
            "v1",
            "state-v1",
            NOW,
        );
    }

    fn promote_runtime_artifact_version(
        state: &mut BridgeCoreState,
        id: &str,
        reference: &str,
        version_label: &str,
        state_schema_version: &str,
        now: &str,
    ) {
        state
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: id.to_string(),
                kind: RuntimeArtifactKind::OciImage,
                reference: reference.to_string(),
                version_label: version_label.to_string(),
                source_git_sha: Some("git-sha".to_string()),
                finitec_version: Some("finitec-test".to_string()),
                hermes_source_ref: Some("hermes-ref".to_string()),
                finite_platform_plugin_ref: Some("plugin-ref".to_string()),
                state_schema_version: state_schema_version.to_string(),
                base_image: Some("python:3.11-trixie".to_string()),
                promoted: true,
                now: Some(now.to_string()),
            })
            .unwrap();
    }

    fn complete_self_serve_agent(
        state: &mut BridgeCoreState,
        email: &str,
        workos_user_id: &str,
        idempotency_key: &str,
        source_machine_id: &str,
        artifact_id: &str,
        now: &str,
    ) -> String {
        let requested = state
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: email.to_string(),
                workos_user_id: workos_user_id.to_string(),
                display_name: source_machine_id.to_string(),
                launch_code: "off2026".to_string(),
                idempotency_key: idempotency_key.to_string(),
                now: Some(NOW.to_string()),
            })
            .unwrap();
        let lease = state
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                source_host_id: None,
                lease_token: format!("lease-{source_machine_id}"),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .unwrap()
            .unwrap();
        let completed = state
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: requested.request.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: format!("lease-{source_machine_id}"),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: source_machine_id.to_string(),
                runtime_artifact_id: Some(artifact_id.to_string()),
                state_schema_version: None,
                display_name: None,
                hostname: None,
                runtime_host: Some("oslo-host-1".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                now: Some(now.to_string()),
            })
            .unwrap();
        assert_eq!(lease.project.id, completed.project.id);
        completed.request.agent_runtime_id.unwrap()
    }
}
