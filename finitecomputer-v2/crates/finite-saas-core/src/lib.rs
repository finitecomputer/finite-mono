pub mod api;
pub mod auth;
pub mod billing;
pub mod launch_codes;
pub mod store;
#[cfg(test)]
pub(crate) mod test_support;

// Re-exported so the crate's public surface is unchanged: Stripe billing
// concepts are now defined in (and owned by) `billing` alone, but existing
// `crate::...` imports in store/api/tests keep resolving.
pub use billing::{
    BillingSubscriptionStatus, CustomerBillingAccount, LinkStripeCustomerInput,
    LinkStripeCustomerRequest, SyncStripeSubscriptionInput, SyncStripeSubscriptionRequest,
    parse_billing_subscription_status,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

pub const CORE_SCHEMA_SQL: &str = concat!(
    include_str!("../migrations/0001_core.sql"),
    "\n",
    include_str!("../migrations/0002_runtime_upgrade.sql"),
    "\n",
    include_str!("../migrations/0003_launch_codes.sql"),
    "\n",
    include_str!("../migrations/0004_membership_archive.sql"),
    "\n",
    include_str!("../migrations/0005_phala_expand.sql"),
    "\n",
    include_str!("../migrations/0006_runtime_capabilities_expand.sql"),
    "\n",
    include_str!("../migrations/0007_provider_creation_operations.sql"),
    "\n",
    include_str!("../migrations/0008_agent_creation_provisional_runtime.sql"),
    "\n",
    include_str!("../migrations/0009_artifact_recovery_support.sql"),
    "\n",
    include_str!("../migrations/0010_align_finite_private_generous.sql"),
    "\n",
    include_str!("../migrations/0011_agent_email.sql"),
    "\n",
    include_str!("../migrations/0012_runtime_retirement_snapshots.sql"),
    "\n",
    include_str!("../migrations/0013_double_finite_private_default.sql"),
    "\n",
    include_str!("../migrations/0014_finite_private_user_controls.sql"),
    "\n",
    include_str!("../migrations/0015_runner_capacity_fences.sql"),
    "\n",
    include_str!("../migrations/0016_runtime_cold_relocation.sql"),
    "\n",
    include_str!("../migrations/0017_rfc3339_reads.sql"),
    "\n",
    include_str!("../migrations/0018_finite_private_5x_profile.sql"),
    "\n",
    include_str!("../migrations/0019_brain_agent_departure_facts.sql"),
    "\n",
    include_str!("../migrations/0020_runtime_offboarding_phases.sql"),
    "\n",
    include_str!("../migrations/0021_runtime_lifecycle.sql"),
    "\n",
    include_str!("../migrations/0022_runtime_health_reports.sql"),
    "\n",
    include_str!("../migrations/0023_agent_creation_owner_chat_account_id.sql")
);
pub const RUNTIME_UPGRADE_ROLLBACK_RESCUE_SQL: &str =
    include_str!("../migrations/runtime_upgrade_rollback_rescue.sql");
pub const RUNTIME_LIFECYCLE_REVERSE_REMAP_SQL: &str =
    include_str!("../migrations/runtime_lifecycle_reverse_remap.sql");
const DEFAULT_AGENT_CREATION_LEASE_SECONDS: i64 = 10 * 60;
const MAX_AGENT_CREATION_LEASE_SECONDS: i64 = 60 * 60;
const DEFAULT_FINITE_PRIVATE_LIMIT_PROFILE: &str = "finite-private-generous-v2";
pub const FINITE_PRIVATE_5X_LIMIT_PROFILE: &str = "finite-private-generous-5x-v1";
const DEFAULT_FINITE_PRIVATE_BURST_WINDOW_SECONDS: i64 = 5 * 60 * 60;
const DEFAULT_FINITE_PRIVATE_BURST_LIMIT_UNITS: i64 = 100_000_000;
const FINITE_PRIVATE_5X_BURST_LIMIT_UNITS: i64 = 500_000_000;
const DEFAULT_FINITE_PRIVATE_WEEKLY_LIMIT_UNITS: Option<i64> = None;
const FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS: i64 = 7 * 24 * 60 * 60;

/// Declare an enum together with the one wire string for each variant.
///
/// The string is written once and drives serde, `as_str`, and the `parse_*`
/// function, so a new variant cannot encode one way in the JSON API and another
/// in its database column. Those three used to be separate hand-written
/// surfaces with nothing forcing them to agree.
macro_rules! wire_enum {
    (
        $(#[doc = $doc:literal])*
        $name:ident { $($variant:ident => $wire:literal),+ $(,)? }
        parse: $parse:ident
    ) => {
        $(#[doc = $doc])*
        #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
        pub enum $name {
            $(
                #[serde(rename = $wire)]
                $variant,
            )+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire,)+
                }
            }
        }

        pub fn $parse(value: &str) -> Option<$name> {
            match value {
                $($wire => Some($name::$variant),)+
                _ => None,
            }
        }
    };
}

wire_enum! {
    BillingClass {
    Grandfathered => "grandfathered",
    Sponsored => "sponsored",
    Standard => "standard",
    }
    parse: parse_billing_class
}

// Path-accessible so `billing` (declared above the textual macro definition)
// can reuse the same single-source-of-truth enum declaration.
pub(crate) use wire_enum;

wire_enum! {
    UserLinkStatus {
    Pending => "pending",
    Linked => "linked",
    }
    parse: parse_user_link_status
}

wire_enum! {
    ProjectMembershipRole {
    Owner => "owner",
    Admin => "admin",
    Member => "member",
    }
    parse: parse_project_membership_role
}

wire_enum! {
    RuntimeSummaryStatus {
    Online => "online",
    Offline => "offline",
    Stale => "stale",
    Unknown => "unknown",
    }
    parse: parse_runtime_summary_status
}

wire_enum! {
/// The single forward-only offboarding state of a Runtime. Each phase is
/// written in the same transaction as the side effect it records and never
/// moves backward: a destroy request (`RetirementRequested`), a stored
/// verified retirement receipt (`ReceiptVerified`), recorded compute removal
/// (`ComputeRemoved`), the offboarding boundary (`LinkDeactivated`), and the
/// terminal departure record (`Archived`). No phase means the Runtime is
/// live. Purge User Data stays the separate retention-gated path (ADR 0001).
    OffboardingPhase {
    RetirementRequested => "retirement_requested",
    ReceiptVerified => "receipt_verified",
    ComputeRemoved => "compute_removed",
    LinkDeactivated => "link_deactivated",
    Archived => "archived",
    }
    parse: parse_offboarding_phase
}

impl OffboardingPhase {
    fn rank(self) -> u8 {
        match self {
            Self::RetirementRequested => 1,
            Self::ReceiptVerified => 2,
            Self::ComputeRemoved => 3,
            Self::LinkDeactivated => 4,
            Self::Archived => 5,
        }
    }

    /// True when `next` keeps the phase moving strictly forward. Restating
    /// the current phase is allowed so an idempotent replay never regresses.
    pub fn transition_allowed(current: Option<Self>, next: Self) -> bool {
        current.is_none_or(|current| current.rank() <= next.rank())
    }

    /// True when this phase has reached or passed `phase`.
    pub fn reached(self, phase: Self) -> bool {
        self.rank() >= phase.rank()
    }

    /// Derive the phase from the durable facts a pre-phase-machine Core
    /// recorded. Mirrors the 0020 backfill exactly; the pre-deploy census
    /// enumerates these flag combinations. A stored verified receipt proves
    /// the runner removed compute before completing the destroy, so receipt
    /// plus an active link is the half-retired ghost (`ComputeRemoved`) and
    /// receipt plus an inactive link is a completed retirement (`Archived`).
    /// An inactive link with the project's active link on another Runtime is
    /// a relocation leftover, not an offboarding (`None`).
    pub fn from_legacy_facts(
        has_verified_receipt: bool,
        destroy_request_active: bool,
        link_active: bool,
        any_link_exists: bool,
        project_has_active_link: bool,
    ) -> Option<Self> {
        if has_verified_receipt {
            return Some(if link_active {
                Self::ComputeRemoved
            } else {
                Self::Archived
            });
        }
        if destroy_request_active && link_active {
            return Some(Self::RetirementRequested);
        }
        if link_active || !any_link_exists || project_has_active_link {
            return None;
        }
        Some(Self::Archived)
    }
}

impl std::fmt::Display for OffboardingPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

wire_enum! {
    RuntimeArtifactKind {
    OciImage => "oci_image",
    }
    parse: parse_runtime_artifact_kind
}

wire_enum! {
/// Customer-facing hosting promise. Provider placement remains a separate,
/// Core-owned fact and is never inferred from BillingClass.
    HostingTier {
    Standard => "standard",
    Confidential => "confidential",
    }
    parse: parse_hosting_tier
}

wire_enum! {
/// Provider-neutral minimum compute shape. Runner adapters translate this
/// closed value to a provider-specific size and verify the returned capacity.
    RuntimeResourceClass {
    Vcpu4Memory8Gib => "vcpu4_memory8_gib",
    Vcpu2Memory4Gib => "vcpu2_memory4_gib",
    }
    parse: parse_runtime_resource_class
}

wire_enum! {
/// Product placement choice stored with an agent creation request. Provider
/// vocabulary stops at the runner adapter; feature behavior does not branch on
/// this value.
    RunnerClass {
    LocalDocker => "local_docker",
    AppleContainer => "apple_container",
    Kata => "kata",
    Phala => "phala",
    Enclavia => "enclavia",
    }
    parse: parse_runner_class
}

/// Immutable placement resolved by Core. Replacement and recovery copy this
/// value rather than rerunning current product policy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimePlacement {
    pub runner_class: RunnerClass,
    pub runtime_resource_class: RuntimeResourceClass,
}

impl RuntimePlacement {
    pub const fn for_hosting_tier(tier: HostingTier) -> Self {
        match tier {
            HostingTier::Standard => Self {
                runner_class: RunnerClass::Kata,
                runtime_resource_class: RuntimeResourceClass::Vcpu4Memory8Gib,
            },
            HostingTier::Confidential => Self {
                runner_class: RunnerClass::Phala,
                runtime_resource_class: RuntimeResourceClass::Vcpu2Memory4Gib,
            },
        }
    }

    /// Compatibility bridge for proven Kata/Phala rows written before the
    /// placement columns existed. Other experimental adapters have no durable
    /// resource-class fact, so callers must leave the expand fields null.
    pub const fn from_legacy_runner_class(runner_class: RunnerClass) -> Option<Self> {
        match runner_class {
            RunnerClass::Kata => Some(Self::for_hosting_tier(HostingTier::Standard)),
            RunnerClass::Phala => Some(Self::for_hosting_tier(HostingTier::Confidential)),
            RunnerClass::LocalDocker | RunnerClass::AppleContainer | RunnerClass::Enclavia => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEndpointContractV1 {
    pub service_port: u16,
    pub health_path: String,
    pub contact_path: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBootIntent {
    #[default]
    Normal,
    RecoverKnownGood,
}

/// Complete immutable launch input owned by Core. The envelope is explicitly
/// versioned so readers reject an unknown contract instead of guessing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSpecV1 {
    pub operation_id: String,
    pub project_id: String,
    pub agent_runtime_id: String,
    pub placement: RuntimePlacement,
    pub runtime_artifact_id: String,
    pub runtime_image_digest: String,
    pub state_schema_version: String,
    pub durable_state_id: String,
    pub endpoints: RuntimeEndpointContractV1,
    #[serde(default)]
    pub boot_intent: RuntimeBootIntent,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub secret_references: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "schema", content = "spec")]
pub enum RuntimeSpecEnvelope {
    #[serde(rename = "runtime_spec.v1")]
    V1(RuntimeSpecV1),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRuntimeHandleV1 {
    pub runner_class: RunnerClass,
    /// Adapter-owned JSON. Core stores and returns it without interpreting
    /// provider ids or copying them into source host/machine identity.
    pub opaque: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "schema", content = "handle")]
pub enum ProviderRuntimeHandleEnvelope {
    #[serde(rename = "provider_runtime_handle.v1")]
    V1(ProviderRuntimeHandleV1),
}

impl ProviderRuntimeHandleEnvelope {
    pub const fn runner_class(&self) -> RunnerClass {
        match self {
            Self::V1(handle) => handle.runner_class,
        }
    }
}

/// Versioned, provider-neutral journal for one creation request. Provider
/// identifiers remain opaque to Core; the request, placement, correlation, and
/// transition order are Core-owned invariants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "schema", content = "operation")]
pub enum ProviderOperationEnvelope {
    #[serde(rename = "provider_operation.v1")]
    V1(ProviderOperationV1),
}

impl ProviderOperationEnvelope {
    pub const fn v1(&self) -> &ProviderOperationV1 {
        match self {
            Self::V1(operation) => operation,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderOperationV1 {
    pub agent_creation_request_id: String,
    pub correlation_id: String,
    pub placement: RuntimePlacement,
    pub transitions: Vec<ProviderOperationTransitionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderOperationTransitionRecord {
    pub sequence: u32,
    pub transition: ProviderOperationTransition,
    pub recorded_at: String,
}

/// Append-only creation states. `provider_facts` is deliberately untyped and
/// bounded: adapters can persist reconciliation evidence without teaching Core
/// provider vocabulary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderOperationTransition {
    CorrelationReserved,
    /// Core acknowledged that the runner is about to perform the first
    /// provider mutation. A crash after this boundary must reconcile the
    /// reserved correlation; it may never be treated as a pre-provider
    /// failure merely because no response facts were persisted yet.
    ProvisionStarted,
    Provisioned {
        provider_facts: Value,
    },
    ProvisionUnknown {
        provider_facts: Value,
    },
    CommitStarted,
    ProviderHandleRecorded {
        provider_runtime_handle: ProviderRuntimeHandleEnvelope,
    },
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordProviderOperationTransitionInput {
    pub request_id: String,
    pub runner_id: String,
    pub lease_token: String,
    pub correlation_id: String,
    pub placement: RuntimePlacement,
    pub transition: ProviderOperationTransition,
}

/// Provider-neutral controls the current Runtime can actually perform. This
/// deliberately excludes the not-yet-proven ensure/inspect/adopt contract;
/// internal adapter helpers are not product capabilities.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeCapabilitiesV1 {
    #[serde(default)]
    pub restart: bool,
    #[serde(default)]
    pub recover_known_good_chat: bool,
    #[serde(default)]
    pub runtime_upgrade: bool,
    #[serde(default)]
    pub stop: bool,
    #[serde(default)]
    pub runtime_retirement: bool,
}

/// Versioned persisted Runtime capability advertisement. Missing and empty
/// advertisements support no controls; callers must never infer support from
/// placement, provider handles, or Runtime artifacts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "schema", content = "capabilities")]
pub enum RuntimeCapabilitiesEnvelope {
    #[serde(rename = "runtime_capabilities.v1")]
    V1(RuntimeCapabilitiesV1),
}

impl RuntimeCapabilitiesEnvelope {
    pub const fn v1(&self) -> &RuntimeCapabilitiesV1 {
        match self {
            Self::V1(capabilities) => capabilities,
        }
    }

    pub const fn supports(&self, kind: RuntimeControlKind) -> bool {
        let capabilities = self.v1();
        match kind {
            RuntimeControlKind::Restart => capabilities.restart,
            RuntimeControlKind::RecoverKnownGoodChatRuntime => capabilities.recover_known_good_chat,
            RuntimeControlKind::Upgrade => capabilities.runtime_upgrade,
            RuntimeControlKind::Stop => capabilities.stop,
            RuntimeControlKind::Destroy => capabilities.runtime_retirement,
        }
    }

    pub const fn supports_any_control(&self) -> bool {
        let capabilities = self.v1();
        capabilities.restart
            || capabilities.recover_known_good_chat
            || capabilities.runtime_upgrade
            || capabilities.stop
            || capabilities.runtime_retirement
    }
}

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
    #[error("agent display name is required")]
    MissingAgentDisplayName,
    #[error("agent creation idempotency key is required")]
    MissingAgentCreationIdempotencyKey,
    #[error("agent profile picture URL is invalid")]
    InvalidAgentProfilePictureUrl,
    #[error("owner chat account id must be 64 lowercase hex characters")]
    InvalidOwnerChatAccountId,
    #[error("runtime contact endpoint is invalid")]
    InvalidRuntimeContactEndpoint,
    #[error("agent runtime id is required")]
    MissingAgentRuntimeId,
    #[error("runtime health report is invalid or out of bounds")]
    InvalidRuntimeHealthReport,
    #[error("provider runtime handle does not match the persisted placement")]
    ProviderRuntimeHandlePlacementMismatch,
    #[error("provider operation correlation id is required or invalid")]
    InvalidProviderOperationCorrelation,
    #[error("provider operation facts are invalid or contain secret material")]
    InvalidProviderOperationFacts,
    #[error("Runner capacity could not produce a safe in-flight reservation")]
    InvalidInFlightCapacityReservation,
    #[error("provider operation identity does not match the creation request")]
    ProviderOperationIdentityMismatch,
    #[error("provider operation transition is out of order")]
    ProviderOperationTransitionConflict,
    #[error("provider operation boundary has not been reached")]
    ProviderOperationBoundaryNotReached,
    #[error("runtime spec does not match its persisted project, placement, runtime, or artifact")]
    RuntimeSpecMismatch,
    #[error("runtime capability advertisement changed during creation")]
    RuntimeCapabilitiesMismatch,
    #[error("runtime capability advertisement exceeds current placement policy")]
    RuntimeCapabilitiesNotAuthorized,
    #[error("no promoted runtime artifact is available for a new runtime")]
    RuntimeArtifactUnavailable,
    #[error("hosting tier is required before creating an agent")]
    MissingHostingTier,
    #[error("selected hosting tier is not authorized by this account or Launch Code")]
    HostingTierNotAuthorized,
    #[error("launch code is required")]
    MissingLaunchCode,
    #[error("launch code is invalid")]
    InvalidLaunchCode,
    #[error("launch code batch name is required")]
    MissingLaunchCodeBatchName,
    #[error("launch code batch name is invalid")]
    InvalidLaunchCodeBatchName,
    #[error("launch code batch size is invalid")]
    InvalidLaunchCodeBatchSize,
    #[error("launch code batch expiry must be between one hour and 30 days")]
    InvalidLaunchCodeBatchExpiry,
    #[error("launch code batch was not found")]
    LaunchCodeBatchNotFound,
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
    #[error("the requested runtime control is not supported for this runtime")]
    RuntimeControlUnsupported,
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
    #[error("runtime retirement snapshot receipt did not match the leased runtime")]
    RuntimeRetirementSnapshotMismatch,
    #[error("runtime retirement snapshot receipt conflicts with the stored receipt")]
    RuntimeRetirementSnapshotConflict,
    #[error("runtime retirement is not enabled for this Core generation")]
    RuntimeRetirementNotEnabled,
    #[error("all unrecoverable runtime archive acknowledgements are required")]
    UnrecoverableRuntimeArchiveAcknowledgementRequired,
    #[error("unrecoverable runtime archive owner does not match")]
    UnrecoverableRuntimeArchiveOwnerMismatch,
    #[error("runtime has provider metadata and cannot use unrecoverable legacy archival")]
    UnrecoverableRuntimeArchiveProviderMetadataPresent,
    #[error("the compute-absent acknowledgement is required for retired runtime offboarding")]
    RetiredRuntimeOffboardAcknowledgementRequired,
    #[error("retired runtime offboard owner does not match")]
    RetiredRuntimeOffboardOwnerMismatch,
    #[error("a verified runtime retirement receipt is required for retired runtime offboarding")]
    RetiredRuntimeOffboardReceiptMissing,
    #[error("runtime offboarding phase cannot regress from {current} to {attempted}")]
    OffboardingPhaseRegression {
        current: OffboardingPhase,
        attempted: OffboardingPhase,
    },
    #[error(
        "runtime offboarding is already at {phase}; resume it with runtime-offboard-retired-exact instead of enqueueing a new destroy"
    )]
    RuntimeOffboardingResumeRequired { phase: OffboardingPhase },
    #[error("runtime control request was not found")]
    RuntimeControlRequestNotFound,
    #[error("runtime control request is not in the launching phase")]
    RuntimeControlRequestNotLaunching,
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
pub struct BillingOverview {
    pub customer_org: CustomerOrganization,
    pub billing_account: Option<CustomerBillingAccount>,
    pub agent_creation_entitlement: Option<AgentCreationEntitlement>,
    pub can_create_agent: bool,
    pub requires_billing: bool,
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
    /// Canonical human-facing Finite Identity for this hosted Agent Principal.
    /// Authorization continues to use the principal key resolved from it.
    #[serde(default)]
    pub agent_email: Option<String>,
    /// LEGACY ROWS ONLY. Set on projects created by the abandoned 2026-07
    /// existing-host import bridge (deleted; see git history for the
    /// reconcile/claim machinery). Production may still hold such rows from
    /// its near-ship test run. Nothing writes this anymore; a `Some` value
    /// means "hide from user-facing project lists" (`public_visible_projects`
    /// in api.rs). A future importer should define its own linkage rather
    /// than resurrecting this field's semantics.
    pub import_candidate_id: Option<String>,
    #[serde(default)]
    pub hosting_tier: Option<HostingTier>,
    #[serde(default)]
    pub placement: Option<RuntimePlacement>,
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
    #[serde(default)]
    pub placement: Option<RuntimePlacement>,
    #[serde(default)]
    pub provider_runtime_handle: Option<ProviderRuntimeHandleEnvelope>,
    #[serde(default)]
    pub provider_runtime_handle_history: Vec<ProviderRuntimeHandleEnvelope>,
    #[serde(default)]
    pub contact_endpoint: Option<String>,
    #[serde(default)]
    pub runtime_capabilities: Option<RuntimeCapabilitiesEnvelope>,
    pub host_facts: HostOwnedRuntimeFacts,
    pub created_at: String,
    pub updated_at: String,
}

impl AgentRuntime {
    /// Fail-closed capability gate for restart/stop/upgrade/etc. Note this is
    /// also what keeps legacy rows inert: runtimes imported by the abandoned
    /// 2026-07 import bridge (and any other row without a capabilities
    /// envelope) have `runtime_capabilities: NULL` and refuse every control.
    pub fn supports_runtime_control(&self, kind: RuntimeControlKind) -> bool {
        self.runtime_capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.supports(kind))
    }
}

/// Standing runtime readiness, ferried by the Runner (2026-08 audit synthesis,
/// H1 slice 3): the Runner polls each live Runtime's `/contact` on a bounded
/// cadence and posts one report per Runtime to Core. Core stores only the
/// latest report on the runtime row (migration 0022) and projects readiness
/// at read time — there is no history table and no background sweeper.
pub const RUNTIME_HEALTH_REPORT_DEFAULT_INTERVAL_SECONDS: i64 = 60;
pub const RUNTIME_HEALTH_REPORT_MIN_INTERVAL_SECONDS: i64 = 5;
pub const RUNTIME_HEALTH_REPORT_MAX_INTERVAL_SECONDS: i64 = 3600;
/// A report older than this many poll intervals is stale: the projection then
/// reads `unknown` ("the runner stopped reporting"), never a frozen `ready`.
pub const RUNTIME_HEALTH_REPORT_STALE_MULTIPLIER: i64 = 3;
pub const MAX_RUNTIME_HEALTH_REPORT_REASON_CHARS: usize = 512;

wire_enum! {
/// Read-time projection of one runtime's standing readiness. `ready` requires
/// a fresh report saying ready; `not_ready` is a fresh report saying not
/// ready (with the reported reason); `unknown` is no report, a stale report,
/// or a runtime Core does not consider online.
    RuntimeHealthStatus {
    Ready => "ready",
    NotReady => "not_ready",
    Unknown => "unknown",
    }
    parse: parse_runtime_health_status
}

/// The latest stored runner-ferried health report, as read back from the
/// runtime row. Every field is `None` until the runner's standing poller
/// first reports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredRuntimeHealth {
    pub reported_at: Option<String>,
    pub observed_at: Option<String>,
    pub ready: Option<bool>,
    pub reason: Option<String>,
    pub report_interval_seconds: Option<i64>,
    pub reporting_npub: Option<String>,
}

/// One runtime's standing readiness as projected at read time. The raw report
/// fields always ride along as evidence; `status` is the only derived fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeHealthProjection {
    pub status: RuntimeHealthStatus,
    pub reason: Option<String>,
    pub reported_at: Option<String>,
    pub observed_at: Option<String>,
    pub agent_npub: Option<String>,
}

/// Project standing readiness from the latest stored report. Reports only
/// speak for runtimes Core considers `online`; an intentionally offline
/// runtime carries no standing readiness claim and projects `unknown`.
/// Freshness is measured from `reported_at` (Core's receive clock), never the
/// runner's `observed_at`, so runner clock skew cannot extend freshness.
pub fn project_runtime_health(
    runtime_status: RuntimeSummaryStatus,
    health: &StoredRuntimeHealth,
    now: &str,
) -> CoreResult<RuntimeHealthProjection> {
    let status = if runtime_status != RuntimeSummaryStatus::Online {
        RuntimeHealthStatus::Unknown
    } else if let (Some(ready), Some(reported_at)) = (health.ready, health.reported_at.as_deref()) {
        let interval_seconds = health
            .report_interval_seconds
            .unwrap_or(RUNTIME_HEALTH_REPORT_DEFAULT_INTERVAL_SECONDS)
            .clamp(
                RUNTIME_HEALTH_REPORT_MIN_INTERVAL_SECONDS,
                RUNTIME_HEALTH_REPORT_MAX_INTERVAL_SECONDS,
            );
        let age = parse_time(now)? - parse_time(reported_at)?;
        if age > Duration::seconds(interval_seconds * RUNTIME_HEALTH_REPORT_STALE_MULTIPLIER) {
            RuntimeHealthStatus::Unknown
        } else if ready {
            RuntimeHealthStatus::Ready
        } else {
            RuntimeHealthStatus::NotReady
        }
    } else {
        RuntimeHealthStatus::Unknown
    };
    Ok(RuntimeHealthProjection {
        status,
        reason: health.reason.clone(),
        reported_at: health.reported_at.clone(),
        observed_at: health.observed_at.clone(),
        agent_npub: health.reporting_npub.clone(),
    })
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
    #[serde(default)]
    pub recover_known_good_chat: bool,
    pub created_at: String,
    pub promoted_at: Option<String>,
    pub retired_at: Option<String>,
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

pub const RUNTIME_RELOCATION_SCHEMA: &str = "runtime_relocation.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeRelocationV1 {
    pub source_host_id: String,
    pub source_machine_id: String,
    pub target_source_host_id: String,
    pub expected_agent_npub: String,
    pub durable_state_manifest_sha256: String,
    /// Operator-attested recovery variant: the source compute no longer
    /// exists (container/task absent at the provider), so there is no stop
    /// receipt to present and the runtime reads `stale`, not `offline`.
    /// Absence is a stronger single-writer guarantee than a stop receipt —
    /// the runbook's bounded absence probe is the attestation's basis.
    /// Additive within runtime_relocation.v1; absent means false.
    #[serde(default)]
    pub source_compute_absent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "schema", content = "relocation")]
pub enum RuntimeRelocationEnvelope {
    #[serde(rename = "runtime_relocation.v1")]
    V1(RuntimeRelocationV1),
}

impl RuntimeRelocationEnvelope {
    pub const fn v1(&self) -> &RuntimeRelocationV1 {
        match self {
            Self::V1(relocation) => relocation,
        }
    }
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

pub const RUNTIME_RETIREMENT_SNAPSHOT_SCHEMA: &str = "runtime_retirement_snapshot.v1";
pub const RUNTIME_RETIREMENT_BACKEND_BORG: &str = "borg";
pub const RUNTIME_RETIREMENT_RETENTION_INDEFINITE: &str = "indefinite_until_purge";

/// Restore-relevant facts produced only after an exact retirement ZIP has
/// been uploaded and read back successfully. Locators are opaque archive
/// names, never repository URLs or credentials.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeRetirementSnapshotReceipt {
    pub schema: String,
    pub request_id: String,
    pub project_id: String,
    pub agent_runtime_id: String,
    pub durable_state_id: String,
    pub runtime_artifact_id: String,
    pub backend: String,
    pub locator: String,
    pub zip_bytes: u64,
    pub zip_sha256: String,
    pub manifest_sha256: String,
    pub created_at: String,
    pub verified_at: String,
    pub recovery_authority_id: String,
    pub retention_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRetirementSnapshot {
    pub receipt: RuntimeRetirementSnapshotReceipt,
    pub stored_at: String,
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
    #[serde(default)]
    pub recover_known_good_chat: bool,
    pub promoted: bool,
    pub now: Option<String>,
}

wire_enum! {
    FinitePrivateGrantStatus {
    Active => "active",
    Revoked => "revoked",
    }
    parse: parse_finite_private_grant_status
}

wire_enum! {
    FinitePrivateApiKeyStatus {
    Active => "active",
    Revoked => "revoked",
    }
    parse: parse_finite_private_api_key_status
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinitePrivateReservationStatus {
    Reserved,
    Settled,
    Denied,
}

wire_enum! {
    FinitePrivateSettlementKind {
    Actual => "actual",
    Estimate => "estimate",
    }
    parse: parse_finite_private_settlement_kind
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
    #[serde(default)]
    pub burst_window_epoch: i64,
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
pub struct FinitePrivateAdminProject {
    pub id: String,
    pub display_name: String,
    pub agent_runtime_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FinitePrivateAdminAccount {
    pub user_id: String,
    pub email: String,
    pub grant: FinitePrivateGrant,
    pub api_keys: Vec<FinitePrivateApiKey>,
    pub projects: Vec<FinitePrivateAdminProject>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FinitePrivateAdminState {
    /// Account-centric operator view. The legacy flat arrays remain during the
    /// additive dashboard/Core rollout so mixed versions fail gracefully.
    #[serde(default)]
    pub accounts: Vec<FinitePrivateAdminAccount>,
    #[serde(default)]
    pub profiles: Vec<FinitePrivateLimitProfile>,
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
    #[serde(default)]
    pub burst_window_epoch: i64,
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
pub struct FinitePrivateUsageNotice {
    pub threshold_remaining_percent: i64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FinitePrivateUsageStatus {
    pub burst_limit_units: i64,
    pub burst_used_units: i64,
    pub burst_remaining_units: i64,
    pub burst_reset_at: String,
    pub free_daily_reset_available: bool,
    pub free_daily_reset_available_again_at: String,
    pub notice: Option<FinitePrivateUsageNotice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FinitePrivateDailyResetResult {
    pub performed: bool,
    pub status: FinitePrivateUsageStatus,
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

/// Approve a grant and issue its first API key as one unit.
///
/// The two steps must share a transaction: a caller that approves and then
/// issues separately can leave a grant with no key behind when the second step
/// fails, and cannot be previewed by `--dry-run` at all because the rolled-back
/// grant is invisible to the key issue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IssueFinitePrivateFriendKeyInput {
    pub verified_email: String,
    pub workos_user_id: Option<String>,
    pub limit_profile_id: Option<String>,
    /// Raw key material generated by the caller; only its hash is stored.
    pub raw_key: String,
    pub project_id: Option<String>,
    pub agent_runtime_id: Option<String>,
    pub now: Option<String>,
}

/// Grant and key created together by [`IssueFinitePrivateFriendKeyInput`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssuedFinitePrivateFriendKey {
    pub grant: FinitePrivateGrant,
    pub api_key: FinitePrivateApiKey,
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

/// Exact operator boundary for a stopped Runtime cold relocation. Durable
/// state transfer remains a separately observable step; the target Runner
/// refuses to launch unless its tree hashes to this request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminRuntimeRelocateExactInput {
    pub admin_verified_email: String,
    pub admin_workos_user_id: String,
    pub project_id: String,
    pub expected_agent_runtime_id: String,
    pub expected_source_host_id: String,
    pub expected_source_machine_id: String,
    pub target_source_host_id: String,
    pub expected_agent_npub: String,
    pub durable_state_manifest_sha256: String,
    /// Recovery variant (same attestation pattern as
    /// `AdminArchiveUnrecoverableRuntimeInput`): the operator has verified
    /// via the runbook's bounded probe that no container or task exists for
    /// the source machine. Relaxes exactly two gates — `stale` is accepted
    /// alongside `offline`, and the succeeded-stop-receipt requirement is
    /// waived (stopping absent compute fails by definition). Every other
    /// exact-match check still applies.
    #[serde(default)]
    pub operator_observed_compute_absent: bool,
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
    pub runtime_status: RuntimeSummaryStatus,
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
    /// until the runner's standing poller first reports (and whenever reports
    /// go stale), so this never displays a frozen last-known `ready`.
    pub runtime_health: RuntimeHealthProjection,
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
pub struct AdminAssignFinitePrivateLimitProfileInput {
    pub admin_verified_email: String,
    pub grant_id: String,
    pub limit_profile_id: String,
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

/// The canonical runtime-control lifecycle state machine (2026-08 audit H1).
///
/// Every transition the store can write is a typed, consuming method here:
/// legal orderings are the only expressible programs, so an illegal
/// transition is unrepresentable rather than guarded against at runtime.
/// `Succeeded` exists only as the successor of `Ready`, which exists only as
/// the successor of `ComputeUp`: `succeeded` can never again be written for
/// "compute exists". Rehydrating from a persisted row goes through each
/// phase's `from_status`, the one honest runtime boundary.
///
/// Kind-consistency of completions (Stop confirms with `Plain`, Destroy with
/// a receipt, Upgrade with artifact facts) is enforced upstream by
/// [`RuntimeControlCompletion::parse`], which is keyed on the request kind.
pub mod runtime_lifecycle {
    use super::{RuntimeControlCompletion, RuntimeControlRequestStatus, RuntimeLifecycleStage};

    /// Phase markers. `Failed` carries its named stage; every other marker
    /// is a zero-sized proof of position in the machine.
    pub mod phase {
        use super::RuntimeLifecycleStage;

        #[derive(Debug, Clone, Copy)]
        pub struct Requested;
        #[derive(Debug, Clone, Copy)]
        pub struct Launching;
        #[derive(Debug, Clone, Copy)]
        pub struct ComputeUp;
        #[derive(Debug, Clone, Copy)]
        pub struct Ready;
        #[derive(Debug, Clone, Copy)]
        pub struct Succeeded;
        #[derive(Debug, Clone, Copy)]
        pub struct Stopped;
        #[derive(Debug, Clone, Copy)]
        pub struct Failed {
            pub stage: RuntimeLifecycleStage,
        }
    }

    /// The persisted status a phase marker stands for.
    pub trait LifecyclePhase {
        const STATUS: RuntimeControlRequestStatus;
    }

    macro_rules! lifecycle_phase {
        ($($phase:ty => $status:expr),+ $(,)?) => {$(
            impl LifecyclePhase for $phase {
                const STATUS: RuntimeControlRequestStatus = $status;
            }
        )+};
    }

    lifecycle_phase! {
        phase::Requested => RuntimeControlRequestStatus::Requested,
        phase::Launching => RuntimeControlRequestStatus::Launching,
        phase::ComputeUp => RuntimeControlRequestStatus::ComputeUp,
        phase::Ready => RuntimeControlRequestStatus::Ready,
        phase::Succeeded => RuntimeControlRequestStatus::Succeeded,
        phase::Stopped => RuntimeControlRequestStatus::Stopped,
        phase::Failed => RuntimeControlRequestStatus::Failed,
    }

    /// A runtime-control request at lifecycle phase `S`. Constructing one
    /// requires either starting at [`phase::Requested`] or proving the
    /// persisted status matches the phase via `from_status`.
    #[derive(Debug, Clone, Copy)]
    pub struct RuntimeLifecycle<S: LifecyclePhase> {
        phase: S,
    }

    impl<S: LifecyclePhase> RuntimeLifecycle<S> {
        fn next<T: LifecyclePhase>(phase: T) -> RuntimeLifecycle<T> {
            RuntimeLifecycle { phase }
        }

        pub fn status(&self) -> RuntimeControlRequestStatus {
            S::STATUS
        }
    }

    impl RuntimeLifecycle<phase::Requested> {
        pub fn enqueue() -> Self {
            Self::next(phase::Requested)
        }

        pub fn from_status(status: RuntimeControlRequestStatus) -> Option<Self> {
            (status == RuntimeControlRequestStatus::Requested).then(Self::enqueue)
        }

        /// The Runner leased the request and owns the launch.
        pub fn lease(self) -> RuntimeLifecycle<phase::Launching> {
            Self::next(phase::Launching)
        }

        pub fn fail(self, stage: RuntimeLifecycleStage) -> RuntimeLifecycle<phase::Failed> {
            Self::next(phase::Failed { stage })
        }
    }

    impl RuntimeLifecycle<phase::Launching> {
        pub fn from_status(status: RuntimeControlRequestStatus) -> Option<Self> {
            (status == RuntimeControlRequestStatus::Launching).then(|| Self::next(phase::Launching))
        }

        /// The Runner reports compute exists, carrying the kind-checked
        /// completion. This proves the runtime is up; it never proves the
        /// runtime is ready.
        pub fn compute_up(
            self,
            completion: &RuntimeControlCompletion,
        ) -> RuntimeLifecycle<phase::ComputeUp> {
            let _ = completion;
            Self::next(phase::ComputeUp)
        }

        /// Stop and Destroy confirm directly into their own terminal; a
        /// stopped runtime has no readiness phase.
        pub fn confirm_stopped(
            self,
            completion: &RuntimeControlCompletion,
        ) -> RuntimeLifecycle<phase::Stopped> {
            let _ = completion;
            Self::next(phase::Stopped)
        }

        /// Retirement requeues itself (Destroy only; the kind gate stays in
        /// the store, which owns the request row).
        pub fn retry(self) -> RuntimeLifecycle<phase::Requested> {
            Self::next(phase::Requested)
        }

        pub fn fail(self, stage: RuntimeLifecycleStage) -> RuntimeLifecycle<phase::Failed> {
            Self::next(phase::Failed { stage })
        }
    }

    impl RuntimeLifecycle<phase::ComputeUp> {
        pub fn from_status(status: RuntimeControlRequestStatus) -> Option<Self> {
            (status == RuntimeControlRequestStatus::ComputeUp).then(|| Self::next(phase::ComputeUp))
        }

        /// The runtime's readiness probe fired. This is the only edge into
        /// `Ready`.
        pub fn ready(self) -> RuntimeLifecycle<phase::Ready> {
            Self::next(phase::Ready)
        }

        pub fn fail(self, stage: RuntimeLifecycleStage) -> RuntimeLifecycle<phase::Failed> {
            Self::next(phase::Failed { stage })
        }
    }

    impl RuntimeLifecycle<phase::Ready> {
        pub fn from_status(status: RuntimeControlRequestStatus) -> Option<Self> {
            (status == RuntimeControlRequestStatus::Ready).then(|| Self::next(phase::Ready))
        }

        /// Terminal success. Only reachable from `Ready`.
        pub fn succeed(self) -> RuntimeLifecycle<phase::Succeeded> {
            Self::next(phase::Succeeded)
        }

        pub fn fail(self, stage: RuntimeLifecycleStage) -> RuntimeLifecycle<phase::Failed> {
            Self::next(phase::Failed { stage })
        }
    }

    impl RuntimeLifecycle<phase::Failed> {
        pub fn stage(&self) -> RuntimeLifecycleStage {
            self.phase.stage
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
    /// Adapter classes this worker can actually reconcile. Empty claims no
    /// creation or lifecycle work. An omitted capacity object remains the
    /// bounded N-1 compatibility path for an old worker.
    #[serde(default)]
    pub runner_classes: Vec<RunnerClass>,
    /// Exact control operations this worker can reconcile. Omitted or an
    /// all-false envelope supports no lifecycle leases.
    #[serde(default)]
    pub runtime_capabilities: Option<RuntimeCapabilitiesEnvelope>,
}

impl RunnerLeaseCapacity {
    /// Phala provider inventory can lag an accepted paid provision. Core must
    /// therefore reserve and count the in-flight creation atomically instead
    /// of letting the worker make a second, racy capacity decision.
    pub fn requires_core_in_flight_reservation(&self) -> bool {
        self.runner_classes.as_slice() == [RunnerClass::Phala]
    }

    pub fn validate_runtime_capability_policy(&self) -> CoreResult<()> {
        let Some(capabilities) = self.runtime_capabilities.as_ref() else {
            return Ok(());
        };
        let capabilities = capabilities.v1();
        if (capabilities.recover_known_good_chat || capabilities.runtime_retirement)
            && (self.runner_classes.is_empty()
                || self
                    .runner_classes
                    .iter()
                    .any(|runner_class| *runner_class != RunnerClass::Kata))
        {
            return Err(CoreError::RuntimeCapabilitiesNotAuthorized);
        }
        if capabilities.runtime_upgrade
            && self
                .runner_classes
                .iter()
                .any(|runner_class| *runner_class != RunnerClass::Kata)
        {
            return Err(CoreError::RuntimeCapabilitiesNotAuthorized);
        }
        Ok(())
    }

    pub fn accepts_runtime_control(&self) -> bool {
        !self.runner_classes.is_empty()
            && self
                .runtime_capabilities
                .as_ref()
                .is_some_and(RuntimeCapabilitiesEnvelope::supports_any_control)
    }

    pub fn accepts_agent_creation(&self) -> bool {
        !self.runner_classes.is_empty()
            && !self.draining
            && (self.requires_core_in_flight_reservation() || !self.sandbox_limit_reached())
    }

    pub fn supports_runtime_control(&self, kind: RuntimeControlKind) -> bool {
        self.runtime_capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.supports(kind))
    }

    pub fn agent_creation_rejection_reason(&self) -> Option<&'static str> {
        if self.runner_classes.is_empty() {
            Some("runner advertises no classes")
        } else if self.draining {
            Some("runner is draining")
        } else if !self.requires_core_in_flight_reservation() && self.sandbox_limit_reached() {
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

#[derive(Debug, Clone, Copy)]
struct InFlightCapacityBounds {
    runner_class: RunnerClass,
    provider_inventory_count: u32,
    max_sandbox_count: u32,
}

fn in_flight_capacity_bounds(
    capacity: &RunnerLeaseCapacity,
) -> CoreResult<Option<InFlightCapacityBounds>> {
    if !capacity.requires_core_in_flight_reservation() {
        return Ok(None);
    }
    let provider_inventory_count = capacity
        .active_sandbox_count
        .ok_or(CoreError::InvalidInFlightCapacityReservation)?;
    let max_sandbox_count = capacity
        .max_sandbox_count
        .filter(|maximum| *maximum > 0)
        .ok_or(CoreError::InvalidInFlightCapacityReservation)?;
    if provider_inventory_count > max_sandbox_count {
        return Err(CoreError::InvalidInFlightCapacityReservation);
    }
    Ok(Some(InFlightCapacityBounds {
        runner_class: RunnerClass::Phala,
        provider_inventory_count,
        max_sandbox_count,
    }))
}

fn in_flight_capacity_reservation(
    request: &AgentCreationRequest,
    placement: Option<RuntimePlacement>,
    capacity: InFlightCapacityBounds,
    core_in_flight_count: u32,
) -> CoreResult<InFlightCapacityReservationEnvelope> {
    let placement = placement.ok_or(CoreError::InvalidInFlightCapacityReservation)?;
    if request.runner_class != capacity.runner_class
        || placement.runner_class != capacity.runner_class
        || core_in_flight_count == 0
        || core_in_flight_count > capacity.max_sandbox_count
    {
        return Err(CoreError::InvalidInFlightCapacityReservation);
    }
    Ok(InFlightCapacityReservationEnvelope::V1(
        InFlightCapacityReservationV1 {
            request_id: request.id.clone(),
            placement,
            provider_inventory_count: capacity.provider_inventory_count,
            core_in_flight_count,
            max_sandbox_count: capacity.max_sandbox_count,
        },
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "schema", content = "reservation")]
pub enum InFlightCapacityReservationEnvelope {
    #[serde(rename = "in_flight_capacity_reservation.v1")]
    V1(InFlightCapacityReservationV1),
}

impl InFlightCapacityReservationEnvelope {
    pub const fn v1(&self) -> &InFlightCapacityReservationV1 {
        match self {
            Self::V1(reservation) => reservation,
        }
    }
}

/// Core's atomic acknowledgement that one creation request owns an in-flight
/// provider-capacity slot. `provider_inventory_count` is the exact count the
/// Runner submitted with its lease request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InFlightCapacityReservationV1 {
    pub request_id: String,
    pub placement: RuntimePlacement,
    pub provider_inventory_count: u32,
    pub core_in_flight_count: u32,
    pub max_sandbox_count: u32,
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

/// The runner's wire request for `POST /api/core/v1/runtime-health-reports`.
/// The source host comes from the runner credential, never from the body, so
/// a runner can only report for runtimes on its own host; a body naming a
/// runtime outside the credential's scope is rejected.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHealthReportRequest {
    pub agent_runtime_id: String,
    pub ready: bool,
    /// Bounded not-ready reason: the guest's `/contact` error or the runner's
    /// `unreachable` marker for a transport failure.
    #[serde(default)]
    pub reason: Option<String>,
    /// When the runner read `/contact` (runner clock; evidence only).
    pub observed_at: String,
    /// The Agent Principal npub the runner pinned and observed; the
    /// anti-port-squat cross-check evidence.
    #[serde(default)]
    pub agent_npub: Option<String>,
    /// The runner's poll cadence; the read-time projection declares staleness
    /// after `RUNTIME_HEALTH_REPORT_STALE_MULTIPLIER` intervals.
    #[serde(default)]
    pub report_interval_seconds: Option<i64>,
    #[serde(default)]
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordRuntimeHealthReportInput {
    pub source_host_id: String,
    pub agent_runtime_id: String,
    pub ready: bool,
    pub reason: Option<String>,
    pub observed_at: String,
    pub agent_npub: Option<String>,
    pub report_interval_seconds: Option<i64>,
    pub now: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHealthReportAck {
    pub agent_runtime_id: String,
    pub recorded_at: String,
}

impl std::str::FromStr for RuntimeArtifactKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_runtime_artifact_kind(value)
            .ok_or_else(|| format!("invalid runtime artifact kind {value}"))
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

pub fn normalize_owner_email(value: Option<&str>) -> Option<String> {
    let email = value?.trim().to_lowercase();
    if email.is_empty() { None } else { Some(email) }
}

/// The natural key for a runtime: `source_host_id:source_machine_id`
/// (UNIQUE on `agent_runtimes.source_import_key`). The name is an artifact of
/// the deleted existing-host import bridge, but the key itself is live
/// identity machinery — every registration resolves runtimes through it.
/// Renaming the column would be schema surgery for zero behavior change, so
/// the legacy name stays.
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

fn valid_agent_npub(value: &str) -> bool {
    value.starts_with("npub1")
        && value.len() <= 256
        && value
            .chars()
            .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
}

fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn validate_runtime_relocation_registration(
    request: &AgentCreationRequest,
    existing_runtime: Option<&AgentRuntime>,
    reported_source_host_id: &str,
    reported_source_machine_id: &str,
) -> CoreResult<()> {
    let Some(relocation) = request
        .relocation
        .as_ref()
        .map(RuntimeRelocationEnvelope::v1)
    else {
        return Ok(());
    };
    let existing_runtime = existing_runtime.ok_or(CoreError::RuntimeSpecMismatch)?;
    // `offline` is the cleanly-stopped case; `stale` and `online` are
    // acceptable only when the envelope itself was minted under the
    // operator's compute-absent attestation (a failed control marks a
    // runtime stale, and absent compute can never produce the stop
    // receipt that would make it offline; `online` is the pre-death last
    // report, equally frozen once the operator attests the compute is
    // absent — keep in sync with the enqueue gate in store.rs).
    let source_status_frozen = match existing_runtime.host_facts.runtime_status {
        RuntimeSummaryStatus::Offline => true,
        RuntimeSummaryStatus::Online => relocation.source_compute_absent,
        RuntimeSummaryStatus::Stale => relocation.source_compute_absent,
        _ => false,
    };
    let source_is_frozen = relocation.source_host_id == existing_runtime.source_host_id
        && relocation.source_machine_id == existing_runtime.source_machine_id
        && source_status_frozen;
    let target_is_registered = relocation.target_source_host_id == existing_runtime.source_host_id
        && relocation.source_machine_id == existing_runtime.source_machine_id;
    if request.agent_runtime_id.as_deref() != Some(existing_runtime.id.as_str())
        || request.target_source_host_id.as_deref()
            != Some(relocation.target_source_host_id.as_str())
        || relocation.target_source_host_id != reported_source_host_id
        || relocation.source_machine_id != reported_source_machine_id
        || (!source_is_frozen && !target_is_registered)
    {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    Ok(())
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

const RUNTIME_SPEC_SERVICE_PORT: u16 = 8080;
const RUNTIME_SPEC_HEALTH_PATH: &str = "/healthz";
const RUNTIME_SPEC_CONTACT_PATH: &str = "/contact";
pub(crate) const FINITE_PRIVATE_SECRET_REFERENCE: &str = "FINITE_PRIVATE_API_KEY";

pub(crate) struct RuntimeSpecIdentity<'a> {
    pub operation_id: &'a str,
    pub project_id: &'a str,
    pub agent_runtime_id: &'a str,
    pub placement: RuntimePlacement,
}

pub(crate) fn build_runtime_spec_v1(
    identity: RuntimeSpecIdentity<'_>,
    artifact: &RuntimeArtifact,
    durable_state_id: &str,
    environment: BTreeMap<String, String>,
    secret_references: Vec<String>,
    boot_intent: RuntimeBootIntent,
) -> CoreResult<RuntimeSpecEnvelope> {
    let spec = RuntimeSpecV1 {
        operation_id: identity.operation_id.to_string(),
        project_id: identity.project_id.to_string(),
        agent_runtime_id: identity.agent_runtime_id.to_string(),
        placement: identity.placement,
        runtime_artifact_id: artifact.id.clone(),
        // This is the complete immutable OCI reference, including its digest;
        // adapters must not reconstruct a repository from process state.
        runtime_image_digest: artifact.reference.clone(),
        state_schema_version: artifact.state_schema_version.clone(),
        durable_state_id: durable_state_id.to_string(),
        endpoints: RuntimeEndpointContractV1 {
            service_port: RUNTIME_SPEC_SERVICE_PORT,
            health_path: RUNTIME_SPEC_HEALTH_PATH.to_string(),
            contact_path: RUNTIME_SPEC_CONTACT_PATH.to_string(),
        },
        boot_intent,
        environment,
        secret_references,
    };
    validate_runtime_spec_v1(&spec, artifact)?;
    Ok(RuntimeSpecEnvelope::V1(spec))
}

pub(crate) fn runtime_spec_secret_references(
    configured_references: &[String],
) -> CoreResult<Vec<String>> {
    let mut seen = BTreeSet::from([FINITE_PRIVATE_SECRET_REFERENCE.to_string()]);
    let mut references = vec![FINITE_PRIVATE_SECRET_REFERENCE.to_string()];

    for reference in configured_references {
        if !runtime_spec_environment_key_is_valid(reference)
            || runtime_spec_reserved_environment_key(reference)
            || !runtime_spec_secret_environment_key(reference)
            || !seen.insert(reference.clone())
        {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        references.push(reference.clone());
    }

    if references.len() > 64 {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    Ok(references)
}

pub(crate) fn runtime_spec_v1(spec: &RuntimeSpecEnvelope) -> &RuntimeSpecV1 {
    match spec {
        RuntimeSpecEnvelope::V1(spec) => spec,
    }
}

pub fn runtime_retirement_archive_locator(request_id: &str) -> String {
    format!("retirement-{request_id}")
}

pub(crate) fn validate_runtime_retirement_snapshot_receipt(
    receipt: &RuntimeRetirementSnapshotReceipt,
    request: &RuntimeControlRequest,
    runtime: &AgentRuntime,
    runtime_spec: &RuntimeSpecEnvelope,
    now: &str,
) -> CoreResult<()> {
    let spec = runtime_spec_v1(runtime_spec);
    let created_at = parse_time(&receipt.created_at)
        .map_err(|_| CoreError::RuntimeRetirementSnapshotMismatch)?;
    let verified_at = parse_time(&receipt.verified_at)
        .map_err(|_| CoreError::RuntimeRetirementSnapshotMismatch)?;
    let now = parse_time(now).map_err(|_| CoreError::RuntimeRetirementSnapshotMismatch)?;
    let opaque_value_valid = |value: &str| {
        !value.is_empty()
            && value.len() <= 256
            && !value.chars().any(char::is_control)
            && !value.contains("//")
    };
    let hash_valid = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    if request.kind != RuntimeControlKind::Destroy
        || receipt.schema != RUNTIME_RETIREMENT_SNAPSHOT_SCHEMA
        || receipt.request_id != request.id
        || receipt.project_id != request.project_id
        || receipt.agent_runtime_id != request.agent_runtime_id
        || receipt.project_id != runtime.project_id
        || receipt.durable_state_id != spec.durable_state_id
        || receipt.runtime_artifact_id != spec.runtime_artifact_id
        || runtime.runtime_artifact_id.as_deref() != Some(receipt.runtime_artifact_id.as_str())
        || receipt.backend != RUNTIME_RETIREMENT_BACKEND_BORG
        || receipt.locator != runtime_retirement_archive_locator(&request.id)
        || receipt.zip_bytes == 0
        || receipt.zip_bytes > i64::MAX as u64
        || !hash_valid(&receipt.zip_sha256)
        || !hash_valid(&receipt.manifest_sha256)
        || !opaque_value_valid(&receipt.recovery_authority_id)
        || receipt.retention_policy != RUNTIME_RETIREMENT_RETENTION_INDEFINITE
        || created_at > verified_at
        || verified_at > now
    {
        return Err(CoreError::RuntimeRetirementSnapshotMismatch);
    }
    Ok(())
}

pub(crate) fn validate_runtime_spec_binding(
    envelope: &RuntimeSpecEnvelope,
    operation_id: Option<&str>,
    project_id: &str,
    agent_runtime_id: &str,
    placement: RuntimePlacement,
    artifact: &RuntimeArtifact,
) -> CoreResult<()> {
    let spec = runtime_spec_v1(envelope);
    validate_runtime_spec_v1(spec, artifact)?;
    if operation_id.is_some_and(|operation_id| spec.operation_id != operation_id)
        || spec.project_id != project_id
        || spec.agent_runtime_id != agent_runtime_id
        || spec.placement != placement
    {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    Ok(())
}

pub(crate) fn runtime_operation_spec_v1(
    current: &RuntimeSpecEnvelope,
    identity: RuntimeSpecIdentity<'_>,
    current_artifact: &RuntimeArtifact,
    desired_artifact: &RuntimeArtifact,
    boot_intent: RuntimeBootIntent,
    refreshed_environment: Option<&BTreeMap<String, String>>,
    refreshed_secret_references: Option<&[String]>,
) -> CoreResult<RuntimeSpecEnvelope> {
    validate_runtime_spec_binding(
        current,
        None,
        identity.project_id,
        identity.agent_runtime_id,
        identity.placement,
        current_artifact,
    )?;
    let current = runtime_spec_v1(current);
    let secret_references = if let Some(configured) = refreshed_secret_references {
        runtime_spec_secret_references(configured)?
    } else {
        current.secret_references.clone()
    };
    // No carry-forward of `OWNER_CHAT_NPUBS_ENV` here: the value is only a
    // birth-time seed. The sidecar's SQLite store consumes it once into the
    // Welcome admission policy on first boot, so an upgrade-time environment
    // refresh that drops it cannot reopen admission after that first boot.
    let environment = match refreshed_environment {
        Some(configured) => configured.clone(),
        None => current.environment.clone(),
    };
    build_runtime_spec_v1(
        identity,
        desired_artifact,
        &current.durable_state_id,
        environment,
        secret_references,
        boot_intent,
    )
}

fn validate_runtime_spec_v1(spec: &RuntimeSpecV1, artifact: &RuntimeArtifact) -> CoreResult<()> {
    let ids_valid = [
        spec.operation_id.as_str(),
        spec.project_id.as_str(),
        spec.agent_runtime_id.as_str(),
        spec.runtime_artifact_id.as_str(),
        spec.durable_state_id.as_str(),
    ]
    .iter()
    .all(|value| {
        !value.trim().is_empty()
            && value.len() <= 256
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    });
    if !ids_valid
        || artifact.kind != RuntimeArtifactKind::OciImage
        || !runtime_artifact_reference_is_immutable_oci(&artifact.reference)
        || spec.runtime_artifact_id != artifact.id
        || spec.runtime_image_digest != artifact.reference
        || spec.state_schema_version != artifact.state_schema_version
        || spec.endpoints.service_port != RUNTIME_SPEC_SERVICE_PORT
        || spec.endpoints.health_path != RUNTIME_SPEC_HEALTH_PATH
        || spec.endpoints.contact_path != RUNTIME_SPEC_CONTACT_PATH
    {
        return Err(CoreError::RuntimeSpecMismatch);
    }

    validate_runtime_spec_environment(&spec.environment)?;

    let mut references = BTreeSet::new();
    for reference in &spec.secret_references {
        if !runtime_spec_environment_key_is_valid(reference)
            || spec.environment.contains_key(reference)
            || !references.insert(reference)
            || (reference != FINITE_PRIVATE_SECRET_REFERENCE
                && !runtime_spec_secret_environment_key(reference))
        {
            return Err(CoreError::RuntimeSpecMismatch);
        }
    }
    if references.len() > 64 {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    Ok(())
}

pub(crate) fn validate_runtime_spec_environment(
    environment: &BTreeMap<String, String>,
) -> CoreResult<()> {
    let mut total_environment_bytes = 0usize;
    for (key, value) in environment {
        if !runtime_spec_environment_key_is_valid(key)
            || runtime_spec_reserved_environment_key(key)
            || runtime_spec_secret_environment_key(key)
            || value.is_empty()
            || value.len() > 4 * 1024
            || value.contains('\0')
        {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        total_environment_bytes = total_environment_bytes
            .saturating_add(key.len())
            .saturating_add(value.len());
    }
    if environment.len() > 64 || total_environment_bytes > 32 * 1024 {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    Ok(())
}

fn runtime_spec_environment_key_is_valid(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_uppercase() || (index > 0 && byte.is_ascii_digit())
        })
}

fn runtime_spec_reserved_environment_key(key: &str) -> bool {
    matches!(
        key,
        "FINITE_SERVER_URL"
            | "FINITECHAT_SERVER_URL"
            | "FINITECHAT_HOME"
            | "FINITE_HOME"
            | "HERMES_HOME"
            | "FINITECHAT_WORKSPACE"
            | "FINITE_AGENT_HTTP_HOST"
            | "FINITE_AGENT_HTTP_PORT"
            | "FINITECHAT_HERMES_AGENT_DEVICE_ID"
            | "FINITE_AGENT_ID"
            | "FINITE_AGENT_NAME"
            | "FINITECHAT_HERMES_AGENT_NAME"
            | "FINITECHAT_HERMES_ROOM_NAME"
            | "FINITECHAT_HERMES_AGENT_PICTURE_URL"
            | "FINITECHAT_HERMES_INBOUND_STREAM"
            | "FINITECHAT_ALLOW_ALL_USERS"
            | "FINITE_ALLOW_ALL_USERS"
            | "GATEWAY_ALLOW_ALL_USERS"
            | "FINITE_DEFAULT_INFERENCE_PROFILE"
            | "FINITE_PRIVATE_MODEL"
            | "FINITE_PRIVATE_BASE_URL"
            | "FINITE_PRIVATE_API_KEY"
            | "FINITECHAT_HERMES_MODEL"
            | "FINITECHAT_HERMES_PROVIDER"
            | "FINITECHAT_HERMES_BASE_URL"
            | "FINITECHAT_HERMES_API_MODE"
            | "FINITE_AGENT_BOOT_INTENT_JSON"
            | "FINITE_AGENT_STATE_ROOT"
            | "OPENAI_API_KEY"
    )
}

fn runtime_spec_secret_environment_key(key: &str) -> bool {
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"]
        .iter()
        .any(|part| key.split('_').any(|segment| segment == *part))
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

/// Lease-time spec-environment key carrying the owner chat account id list
/// (comma-separated 64-hex account ids). The runtime image derives the Hermes
/// adapter allowlist and the sidecar Welcome allowlist from it. Deliberately
/// not a reserved key: it is Core-managed per-request spec state, not
/// Core-global operator configuration.
pub(crate) const OWNER_CHAT_NPUBS_ENV: &str = "FINITECHAT_OWNER_NPUBS";

/// The dashboard submits the hosted-device `identity.account_id`, which is the
/// account's 64-hex public key; the Hermes adapter's `user_id` and the chat
/// sidecar Welcome allowlist both consume that same hex form, so Core stores
/// the canonical lowercase hex and accepts nothing else.
pub(crate) fn normalize_owner_chat_account_id(value: Option<&str>) -> CoreResult<Option<String>> {
    let Some(value) = trim_to_option(value) else {
        return Ok(None);
    };
    let normalized = value.to_ascii_lowercase();
    if normalized.len() != 64
        || !normalized
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CoreError::InvalidOwnerChatAccountId);
    }
    Ok(Some(normalized))
}

pub(crate) fn normalize_runtime_contact_endpoint(
    value: Option<&str>,
) -> CoreResult<Option<String>> {
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
        return Err(CoreError::InvalidRuntimeContactEndpoint);
    }
    Ok(Some(value.trim_end_matches('/').to_string()))
}

pub(crate) fn runtime_upgrade_contact_endpoint(
    published_app_urls: &[String],
) -> CoreResult<String> {
    let mut contact_endpoint = None;
    for published_url in published_app_urls {
        let normalized = normalize_runtime_contact_endpoint(Some(published_url))?
            .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?;
        if !normalized.ends_with("/contact") {
            continue;
        }
        if contact_endpoint.replace(normalized).is_some() {
            return Err(CoreError::RuntimeUpgradeCompletionMismatch);
        }
    }
    contact_endpoint.ok_or(CoreError::RuntimeUpgradeCompletionMismatch)
}

pub(crate) fn merge_provider_runtime_handle(
    existing: Option<&AgentRuntime>,
    incoming: Option<ProviderRuntimeHandleEnvelope>,
    placement: Option<RuntimePlacement>,
) -> CoreResult<(
    Option<ProviderRuntimeHandleEnvelope>,
    Vec<ProviderRuntimeHandleEnvelope>,
)> {
    let mut current = existing.and_then(|runtime| runtime.provider_runtime_handle.clone());
    let mut history = existing
        .map(|runtime| runtime.provider_runtime_handle_history.clone())
        .unwrap_or_default();
    if let Some(incoming) = incoming {
        let placement = placement.ok_or(CoreError::ProviderRuntimeHandlePlacementMismatch)?;
        if incoming.runner_class() != placement.runner_class {
            return Err(CoreError::ProviderRuntimeHandlePlacementMismatch);
        }
        if history.last() != Some(&incoming) {
            history.push(incoming.clone());
        }
        current = Some(incoming);
    }
    Ok((current, history))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderOperationTransitionKind {
    CorrelationReserved,
    ProvisionStarted,
    Provisioned,
    ProvisionUnknown,
    CommitStarted,
    ProviderHandleRecorded,
    Ready,
}

impl ProviderOperationTransition {
    fn kind(&self) -> ProviderOperationTransitionKind {
        match self {
            Self::CorrelationReserved => ProviderOperationTransitionKind::CorrelationReserved,
            Self::ProvisionStarted => ProviderOperationTransitionKind::ProvisionStarted,
            Self::Provisioned { .. } => ProviderOperationTransitionKind::Provisioned,
            Self::ProvisionUnknown { .. } => ProviderOperationTransitionKind::ProvisionUnknown,
            Self::CommitStarted => ProviderOperationTransitionKind::CommitStarted,
            Self::ProviderHandleRecorded { .. } => {
                ProviderOperationTransitionKind::ProviderHandleRecorded
            }
            Self::Ready => ProviderOperationTransitionKind::Ready,
        }
    }
}

fn normalize_provider_operation_correlation(value: &str) -> CoreResult<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 128
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(CoreError::InvalidProviderOperationCorrelation);
    }
    Ok(value.to_string())
}

fn provider_operation_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace('-', "_");
    [
        "secret",
        "token",
        "password",
        "credential",
        "authorization",
        "api_key",
        "private_key",
        "environment",
        "env",
    ]
    .iter()
    .any(|term| key == *term || key.ends_with(&format!("_{term}")))
}

fn validate_provider_operation_facts(value: &Value) -> CoreResult<()> {
    if !value.is_object()
        || serde_json::to_vec(value)
            .map_err(|_| CoreError::InvalidProviderOperationFacts)?
            .len()
            > 16 * 1024
    {
        return Err(CoreError::InvalidProviderOperationFacts);
    }

    fn visit(value: &Value, depth: usize, entries: &mut usize) -> bool {
        if depth > 8 {
            return false;
        }
        match value {
            Value::Object(object) => object.iter().all(|(key, value)| {
                *entries += 1;
                *entries <= 256
                    && key.len() <= 128
                    && !provider_operation_secret_key(key)
                    && visit(value, depth + 1, entries)
            }),
            Value::Array(array) => {
                *entries += array.len();
                *entries <= 256 && array.iter().all(|value| visit(value, depth + 1, entries))
            }
            Value::String(value) => value.len() <= 2_048,
            Value::Null | Value::Bool(_) | Value::Number(_) => true,
        }
    }

    if visit(value, 0, &mut 0) {
        Ok(())
    } else {
        Err(CoreError::InvalidProviderOperationFacts)
    }
}

fn validate_provider_operation_transition(
    transition: &ProviderOperationTransition,
) -> CoreResult<()> {
    match transition {
        ProviderOperationTransition::Provisioned { provider_facts }
        | ProviderOperationTransition::ProvisionUnknown { provider_facts } => {
            validate_provider_operation_facts(provider_facts)
        }
        ProviderOperationTransition::CorrelationReserved
        | ProviderOperationTransition::ProvisionStarted
        | ProviderOperationTransition::CommitStarted
        | ProviderOperationTransition::ProviderHandleRecorded { .. }
        | ProviderOperationTransition::Ready => Ok(()),
    }
}

pub(crate) fn append_provider_operation_transition(
    existing: Option<&ProviderOperationEnvelope>,
    request_id: &str,
    correlation_id: &str,
    placement: RuntimePlacement,
    transition: ProviderOperationTransition,
    recorded_at: &str,
) -> CoreResult<ProviderOperationEnvelope> {
    let correlation_id = normalize_provider_operation_correlation(correlation_id)?;
    validate_provider_operation_transition(&transition)?;

    let mut operation = match existing {
        Some(ProviderOperationEnvelope::V1(operation)) => {
            if operation.agent_creation_request_id != request_id
                || operation.correlation_id != correlation_id
                || operation.placement != placement
            {
                return Err(CoreError::ProviderOperationIdentityMismatch);
            }
            operation.clone()
        }
        None => ProviderOperationV1 {
            agent_creation_request_id: request_id.to_string(),
            correlation_id,
            placement,
            transitions: Vec::new(),
        },
    };

    if let Some(persisted) = operation
        .transitions
        .iter()
        .find(|persisted| persisted.transition.kind() == transition.kind())
    {
        if persisted.transition == transition {
            return Ok(ProviderOperationEnvelope::V1(operation));
        }
        return Err(CoreError::ProviderOperationTransitionConflict);
    }

    let previous = operation
        .transitions
        .last()
        .map(|record| record.transition.kind());
    let legal = matches!(
        (previous, transition.kind()),
        (None, ProviderOperationTransitionKind::CorrelationReserved)
            | (
                Some(ProviderOperationTransitionKind::CorrelationReserved),
                ProviderOperationTransitionKind::ProvisionStarted
            )
            | (
                Some(ProviderOperationTransitionKind::ProvisionStarted),
                ProviderOperationTransitionKind::Provisioned
                    | ProviderOperationTransitionKind::ProvisionUnknown
            )
            | (
                Some(ProviderOperationTransitionKind::ProvisionUnknown),
                ProviderOperationTransitionKind::Provisioned
            )
            | (
                Some(ProviderOperationTransitionKind::Provisioned),
                ProviderOperationTransitionKind::CommitStarted
            )
            | (
                Some(ProviderOperationTransitionKind::CommitStarted),
                ProviderOperationTransitionKind::ProviderHandleRecorded
            )
            | (
                Some(ProviderOperationTransitionKind::ProviderHandleRecorded),
                ProviderOperationTransitionKind::Ready
            )
    );
    if !legal {
        return Err(CoreError::ProviderOperationTransitionConflict);
    }

    operation
        .transitions
        .push(ProviderOperationTransitionRecord {
            sequence: operation.transitions.len() as u32,
            transition,
            recorded_at: recorded_at.to_string(),
        });
    Ok(ProviderOperationEnvelope::V1(operation))
}

pub(crate) fn provider_operation_at_runtime_boundary(
    existing: Option<&ProviderOperationEnvelope>,
    provider_runtime_handle: Option<&ProviderRuntimeHandleEnvelope>,
    ready: bool,
    recorded_at: &str,
) -> CoreResult<Option<ProviderOperationEnvelope>> {
    let Some(existing) = existing else {
        return Ok(None);
    };
    let operation = existing.v1();
    let handle = provider_runtime_handle
        .cloned()
        .ok_or(CoreError::ProviderOperationBoundaryNotReached)?;
    let mut updated = append_provider_operation_transition(
        Some(existing),
        &operation.agent_creation_request_id,
        &operation.correlation_id,
        operation.placement,
        ProviderOperationTransition::ProviderHandleRecorded {
            provider_runtime_handle: handle,
        },
        recorded_at,
    )?;
    if ready {
        updated = append_provider_operation_transition(
            Some(&updated),
            &operation.agent_creation_request_id,
            &operation.correlation_id,
            operation.placement,
            ProviderOperationTransition::Ready,
            recorded_at,
        )?;
    }
    Ok(Some(updated))
}

/// Preserve an explicit Core backfill when an N-1 worker omits the new field,
/// but never allow a current worker to change its advertisement between
/// registration retries or final completion.
pub(crate) fn merge_runtime_capabilities(
    existing: Option<&AgentRuntime>,
    incoming: Option<RuntimeCapabilitiesEnvelope>,
) -> CoreResult<Option<RuntimeCapabilitiesEnvelope>> {
    let current = existing.and_then(|runtime| runtime.runtime_capabilities.clone());
    match (current, incoming) {
        (Some(current), Some(incoming)) if current != incoming => {
            Err(CoreError::RuntimeCapabilitiesMismatch)
        }
        (Some(current), _) => Ok(Some(current)),
        (None, incoming) => Ok(incoming),
    }
}

/// Bound worker claims to product authority this Core generation has actually
/// accepted. A route-scoped worker credential is not permission to expose a
/// misleading recovery control or a destructive retirement transition.
pub(crate) fn validate_runtime_capabilities_policy(
    capabilities: Option<&RuntimeCapabilitiesEnvelope>,
    placement: Option<RuntimePlacement>,
) -> CoreResult<()> {
    let Some(capabilities) = capabilities else {
        return Ok(());
    };
    let capabilities = capabilities.v1();
    if (capabilities.recover_known_good_chat || capabilities.runtime_retirement)
        && placement.is_none_or(|placement| placement.runner_class != RunnerClass::Kata)
    {
        return Err(CoreError::RuntimeCapabilitiesNotAuthorized);
    }
    if capabilities.runtime_upgrade
        && placement.is_none_or(|placement| placement.runner_class != RunnerClass::Kata)
    {
        return Err(CoreError::RuntimeCapabilitiesNotAuthorized);
    }
    Ok(())
}

pub(crate) fn validate_runtime_capabilities_artifact_policy(
    capabilities: Option<&RuntimeCapabilitiesEnvelope>,
    placement: Option<RuntimePlacement>,
    artifact: &RuntimeArtifact,
) -> CoreResult<()> {
    validate_runtime_capabilities_policy(capabilities, placement)?;
    if capabilities.is_some_and(|capabilities| capabilities.v1().recover_known_good_chat)
        && !artifact.recover_known_good_chat
    {
        return Err(CoreError::RuntimeCapabilitiesNotAuthorized);
    }
    Ok(())
}

pub(crate) fn bound_runtime_capabilities_to_artifact(
    capabilities: Option<RuntimeCapabilitiesEnvelope>,
    artifact: &RuntimeArtifact,
) -> Option<RuntimeCapabilitiesEnvelope> {
    capabilities.map(|mut envelope| {
        let RuntimeCapabilitiesEnvelope::V1(capabilities) = &mut envelope;
        capabilities.recover_known_good_chat &= artifact.recover_known_good_chat;
        envelope
    })
}

pub(crate) fn provider_operation_allows_generic_failure(
    operation: &ProviderOperationEnvelope,
) -> bool {
    matches!(
        operation
            .v1()
            .transitions
            .last()
            .map(|record| &record.transition),
        Some(ProviderOperationTransition::CorrelationReserved)
    )
}

fn current_time_iso() -> CoreResult<String> {
    // Truncate to microseconds: TIMESTAMPTZ stores exactly six fractional
    // digits, so a nanosecond-precision stamp would round on write and stop
    // round-tripping byte-for-byte (macOS clocks tick in microseconds, which
    // hid this; Linux exposes it).
    let now = OffsetDateTime::now_utc();
    let now = now
        .replace_nanosecond(now.nanosecond() / 1_000 * 1_000)
        .expect("truncating nanoseconds cannot leave the valid range");
    Ok(now.format(&Rfc3339)?)
}

fn parse_time(value: &str) -> CoreResult<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| CoreError::InvalidTimestamp)
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

/// Mint the stable, collision-resistant Finite VIP address shown to humans.
/// The readable prefix comes from the chosen agent name; the opaque project
/// suffix keeps duplicate names from competing for the same global identity.
pub fn canonical_agent_email(display_name: &str, project_id: &str) -> String {
    let mut slug = String::with_capacity(display_name.len());
    let mut previous_was_separator = false;
    for character in display_name.trim().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !slug.is_empty() && !previous_was_separator {
            slug.push('-');
            previous_was_separator = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("agent");
    }
    slug.truncate(40);
    while slug.ends_with('-') {
        slug.pop();
    }

    let mut suffix = project_id
        .strip_prefix("project_")
        .unwrap_or(project_id)
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(16)
        .collect::<String>()
        .to_ascii_lowercase();
    if suffix.len() < 16 {
        suffix = Sha256::digest(project_id.as_bytes())
            .iter()
            .take(8)
            .map(|byte| format!("{byte:02x}"))
            .collect();
    }
    format!("{slug}-{suffix}@finite.vip")
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
        && existing.recover_known_good_chat == candidate.recover_known_good_chat
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

pub(crate) fn finite_private_begins_new_epoch(
    grant: &FinitePrivateGrant,
    projected_window_started_at: &str,
) -> CoreResult<bool> {
    let Some(current_window_started_at) = grant.current_window_started_at.as_deref() else {
        return Ok(false);
    };
    Ok(parse_time(current_window_started_at)? != parse_time(projected_window_started_at)?)
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

fn finite_private_limit_reached_message(
    window_label: &str,
    reset_at: &str,
    retry_after_seconds: i64,
) -> String {
    format!(
        "Finite Private {window_label} limit reached. Your usage resets at {reset_at} ({}).",
        finite_private_retry_after_label(retry_after_seconds)
    )
}

fn finite_private_retry_after_label(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds == 0 {
        return "resetting now".to_string();
    }
    let total_minutes = (seconds + 59) / 60;
    let days = total_minutes / (24 * 60);
    let hours = (total_minutes % (24 * 60)) / 60;
    let minutes = total_minutes % 60;
    if days > 0 && hours > 0 {
        format!("in {days}d {hours}h")
    } else if days > 0 {
        format!("in {days}d")
    } else if hours > 0 && minutes > 0 {
        format!("in {hours}h {minutes}m")
    } else if hours > 0 {
        format!("in {hours}h")
    } else {
        format!("in {minutes}m")
    }
}

fn finite_private_next_daily_reset_at(now: OffsetDateTime) -> CoreResult<String> {
    let next_midnight = (now.unix_timestamp().div_euclid(86_400) + 1) * 86_400;
    OffsetDateTime::from_unix_timestamp(next_midnight)
        .map_err(|_| CoreError::InvalidTimestamp)?
        .format(&Rfc3339)
        .map_err(CoreError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TestDb, with_isolated_postgres};
    use serde_json::json;

    const NOW: &str = "2026-05-25T12:00:00Z";
    const LATER: &str = "2026-05-25T13:00:00Z";

    /// Assert the three independent encodings of one enum agree.
    macro_rules! assert_wire_encodings_agree {
        ($parse:path, $($variant:expr),+ $(,)?) => {
            $({
                let value = $variant;
                let encoded = serde_json::to_value(value).unwrap();
                let encoded = encoded
                    .as_str()
                    .unwrap_or_else(|| panic!("{value:?} does not serialize to a JSON string"));
                assert_eq!(
                    encoded,
                    value.as_str(),
                    "serde and as_str disagree for {value:?}",
                );
                assert_eq!(
                    $parse(value.as_str()),
                    Some(value),
                    "{} rejects the as_str it must round-trip for {value:?}",
                    stringify!($parse),
                );
            })+
        };
    }

    /// TIMESTAMPTZ stores six fractional digits, so any stamp Core generates
    /// with more would round on write and no longer round-trip byte-for-byte.
    /// On macOS the clock ticks in microseconds and this can never fire; on
    /// Linux (CI, production) nanosecond stamps are real.
    #[test]
    fn current_time_iso_never_exceeds_postgres_microsecond_precision() {
        for _ in 0..1_000 {
            let stamp = current_time_iso().unwrap();
            let parsed = parse_time(&stamp).unwrap();
            assert_eq!(parsed.nanosecond() % 1_000, 0, "{stamp}");
        }
    }

    fn stored_health(ready: bool, reported_at: &str, interval_seconds: i64) -> StoredRuntimeHealth {
        StoredRuntimeHealth {
            reported_at: Some(reported_at.to_string()),
            observed_at: Some(reported_at.to_string()),
            ready: Some(ready),
            reason: None,
            report_interval_seconds: Some(interval_seconds),
            reporting_npub: None,
        }
    }

    #[test]
    fn runtime_health_projection_is_ready_only_for_a_fresh_ready_report() {
        let now = "2026-08-24T12:00:00Z";
        let fresh = stored_health(true, "2026-08-24T11:59:00Z", 60);
        let projected = project_runtime_health(RuntimeSummaryStatus::Online, &fresh, now).unwrap();
        assert_eq!(projected.status, RuntimeHealthStatus::Ready);

        // A not-ready report stays not_ready inside the freshness window, and
        // its reason rides along.
        let mut not_ready = stored_health(false, "2026-08-24T11:59:00Z", 60);
        not_ready.reason = Some("unreachable".to_string());
        let projected =
            project_runtime_health(RuntimeSummaryStatus::Online, &not_ready, now).unwrap();
        assert_eq!(projected.status, RuntimeHealthStatus::NotReady);
        assert_eq!(projected.reason.as_deref(), Some("unreachable"));
    }

    #[test]
    fn runtime_health_projection_names_stale_and_missing_reports_unknown() {
        let now = "2026-08-24T12:00:00Z";
        // 181s old at a 60s cadence is past the 3x staleness deadline: the
        // "died at 3am, shows ready forever" gap closes as `unknown`.
        let stale = stored_health(true, "2026-08-24T11:56:59Z", 60);
        let projected = project_runtime_health(RuntimeSummaryStatus::Online, &stale, now).unwrap();
        assert_eq!(projected.status, RuntimeHealthStatus::Unknown);

        // Just inside the deadline still projects the report.
        let edge = stored_health(true, "2026-08-24T11:57:00Z", 60);
        let projected = project_runtime_health(RuntimeSummaryStatus::Online, &edge, now).unwrap();
        assert_eq!(projected.status, RuntimeHealthStatus::Ready);

        // A slower reporter gets its own deadline: 10m cadence, 20m old is
        // fresh for it but would be stale at the default cadence.
        let slow = stored_health(true, "2026-08-24T11:40:00Z", 600);
        let projected = project_runtime_health(RuntimeSummaryStatus::Online, &slow, now).unwrap();
        assert_eq!(projected.status, RuntimeHealthStatus::Ready);

        // No report at all is unknown.
        let projected = project_runtime_health(
            RuntimeSummaryStatus::Online,
            &StoredRuntimeHealth::default(),
            now,
        )
        .unwrap();
        assert_eq!(projected.status, RuntimeHealthStatus::Unknown);
    }

    #[test]
    fn runtime_health_projection_only_answers_for_online_runtimes() {
        let now = "2026-08-24T12:00:00Z";
        let fresh = stored_health(true, "2026-08-24T11:59:30Z", 60);
        for status in [
            RuntimeSummaryStatus::Offline,
            RuntimeSummaryStatus::Stale,
            RuntimeSummaryStatus::Unknown,
        ] {
            let projected = project_runtime_health(status, &fresh, now).unwrap();
            assert_eq!(
                projected.status,
                RuntimeHealthStatus::Unknown,
                "an intentionally not-online runtime carries no standing readiness claim"
            );
        }
    }

    /// `wire_enum!` now generates serde, `as_str`, and `parse_*` from one
    /// variant list, so the three cannot drift by construction. This keeps
    /// checking them because the guarantee depends on serde's `rename`
    /// behaving as assumed, and because an enum added outside the macro would
    /// otherwise reintroduce the three hand-written surfaces unnoticed.
    #[test]
    fn enum_serde_as_str_and_parse_encodings_agree() {
        use BillingClass::*;
        assert_wire_encodings_agree!(parse_billing_class, Grandfathered, Sponsored, Standard);

        use BillingSubscriptionStatus::*;
        assert_wire_encodings_agree!(
            parse_billing_subscription_status,
            Incomplete,
            IncompleteExpired,
            Trialing,
            BillingSubscriptionStatus::Active,
            PastDue,
            Canceled,
            Unpaid,
            Paused,
        );

        assert_wire_encodings_agree!(
            parse_user_link_status,
            UserLinkStatus::Pending,
            UserLinkStatus::Linked,
        );

        assert_wire_encodings_agree!(
            parse_project_membership_role,
            ProjectMembershipRole::Owner,
            ProjectMembershipRole::Admin,
            ProjectMembershipRole::Member,
        );

        assert_wire_encodings_agree!(
            parse_runtime_summary_status,
            RuntimeSummaryStatus::Online,
            RuntimeSummaryStatus::Offline,
            RuntimeSummaryStatus::Stale,
            RuntimeSummaryStatus::Unknown,
        );

        assert_wire_encodings_agree!(
            parse_offboarding_phase,
            OffboardingPhase::RetirementRequested,
            OffboardingPhase::ReceiptVerified,
            OffboardingPhase::ComputeRemoved,
            OffboardingPhase::LinkDeactivated,
            OffboardingPhase::Archived,
        );

        assert_wire_encodings_agree!(
            parse_runtime_health_status,
            RuntimeHealthStatus::Ready,
            RuntimeHealthStatus::NotReady,
            RuntimeHealthStatus::Unknown,
        );

        assert_wire_encodings_agree!(parse_runtime_artifact_kind, RuntimeArtifactKind::OciImage);

        assert_wire_encodings_agree!(
            parse_hosting_tier,
            HostingTier::Standard,
            HostingTier::Confidential,
        );

        assert_wire_encodings_agree!(
            parse_runtime_resource_class,
            RuntimeResourceClass::Vcpu4Memory8Gib,
            RuntimeResourceClass::Vcpu2Memory4Gib,
        );

        assert_wire_encodings_agree!(
            parse_runner_class,
            RunnerClass::LocalDocker,
            RunnerClass::AppleContainer,
            RunnerClass::Kata,
            RunnerClass::Phala,
            RunnerClass::Enclavia,
        );

        assert_wire_encodings_agree!(
            parse_runtime_control_kind,
            RuntimeControlKind::Restart,
            RuntimeControlKind::RecoverKnownGoodChatRuntime,
            RuntimeControlKind::Upgrade,
            RuntimeControlKind::Stop,
            RuntimeControlKind::Destroy,
        );

        assert_wire_encodings_agree!(
            parse_runtime_control_request_status,
            RuntimeControlRequestStatus::Requested,
            RuntimeControlRequestStatus::Launching,
            RuntimeControlRequestStatus::ComputeUp,
            RuntimeControlRequestStatus::Ready,
            RuntimeControlRequestStatus::Succeeded,
            RuntimeControlRequestStatus::Stopped,
            RuntimeControlRequestStatus::Failed,
        );

        assert_wire_encodings_agree!(
            parse_runtime_lifecycle_stage,
            RuntimeLifecycleStage::Launch,
            RuntimeLifecycleStage::Compute,
            RuntimeLifecycleStage::Readiness,
            RuntimeLifecycleStage::Retirement,
            RuntimeLifecycleStage::Unknown,
        );

        assert_wire_encodings_agree!(
            parse_agent_creation_request_status,
            AgentCreationRequestStatus::Requested,
            AgentCreationRequestStatus::Launching,
            AgentCreationRequestStatus::Running,
            AgentCreationRequestStatus::Failed,
            AgentCreationRequestStatus::Cancelled,
        );

        assert_wire_encodings_agree!(
            parse_finite_private_grant_status,
            FinitePrivateGrantStatus::Active,
            FinitePrivateGrantStatus::Revoked,
        );

        assert_wire_encodings_agree!(
            parse_finite_private_api_key_status,
            FinitePrivateApiKeyStatus::Active,
            FinitePrivateApiKeyStatus::Revoked,
        );

        assert_wire_encodings_agree!(
            parse_finite_private_reservation_status,
            FinitePrivateReservationStatus::Reserved,
            FinitePrivateReservationStatus::Settled,
            FinitePrivateReservationStatus::Denied,
        );

        assert_wire_encodings_agree!(
            parse_finite_private_settlement_kind,
            FinitePrivateSettlementKind::Actual,
            FinitePrivateSettlementKind::Estimate,
        );
    }

    /// Every phase pair is classified exactly by rank: forward moves and
    /// same-phase restatements are allowed, any backward move is refused.
    #[test]
    fn offboarding_phase_transitions_are_forward_only() {
        let ordered = [
            OffboardingPhase::RetirementRequested,
            OffboardingPhase::ReceiptVerified,
            OffboardingPhase::ComputeRemoved,
            OffboardingPhase::LinkDeactivated,
            OffboardingPhase::Archived,
        ];
        assert!(OffboardingPhase::transition_allowed(None, ordered[0]));
        assert!(OffboardingPhase::transition_allowed(
            None,
            *ordered.last().unwrap()
        ));
        for (from_index, current) in ordered.iter().enumerate() {
            for (to_index, attempted) in ordered.iter().enumerate() {
                assert_eq!(
                    OffboardingPhase::transition_allowed(Some(*current), *attempted),
                    from_index <= to_index,
                    "{current} -> {attempted}",
                );
                assert_eq!(current.reached(*attempted), from_index >= to_index);
            }
        }
    }

    /// The 0020 backfill mapping, mirrored by `from_legacy_facts`, over every
    /// legacy flag combination. A verified receipt dominates (the destroy
    /// completed, so compute is gone); an inactive link with no receipt and no
    /// surviving project link is the archived-unrecoverable shape; an inactive
    /// link superseded by another active link of the same project is a
    /// relocation leftover, not an offboarding.
    #[test]
    fn offboarding_phase_maps_every_legacy_flag_combination() {
        use OffboardingPhase::*;
        let expected = |has_verified_receipt,
                        destroy_request_active,
                        link_active,
                        any_link_exists,
                        project_has_active_link| {
            OffboardingPhase::from_legacy_facts(
                has_verified_receipt,
                destroy_request_active,
                link_active,
                any_link_exists,
                project_has_active_link,
            )
        };
        for destroy_request_active in [false, true] {
            for any_link_exists in [false, true] {
                for project_has_active_link in [false, true] {
                    // The half-retired ghost: receipt stored, link still active.
                    assert_eq!(
                        expected(
                            true,
                            destroy_request_active,
                            true,
                            any_link_exists,
                            project_has_active_link
                        ),
                        Some(ComputeRemoved),
                    );
                    // Completed retirement: receipt stored, link deactivated.
                    assert_eq!(
                        expected(
                            true,
                            destroy_request_active,
                            false,
                            any_link_exists,
                            project_has_active_link
                        ),
                        Some(Archived),
                    );
                    // Live runtime, with or without an in-flight destroy.
                    assert_eq!(
                        expected(false, false, true, any_link_exists, project_has_active_link),
                        None,
                    );
                    assert_eq!(
                        expected(false, true, true, any_link_exists, project_has_active_link),
                        Some(RetirementRequested),
                    );
                    // Never linked: no offboarding evidence at all.
                    assert_eq!(
                        expected(
                            false,
                            destroy_request_active,
                            false,
                            false,
                            project_has_active_link
                        ),
                        None,
                    );
                    // Inactive link but the project has another active
                    // runtime: superseded by relocation, not offboarded.
                    assert_eq!(
                        expected(false, destroy_request_active, false, true, true),
                        None,
                    );
                    // Inactive link, no receipt, no surviving project link:
                    // unrecoverable archive or legacy offboard.
                    assert_eq!(
                        expected(false, destroy_request_active, false, true, false),
                        Some(Archived),
                    );
                }
            }
        }
    }

    fn completion_input(
        artifact: Option<&str>,
        receipt: Option<RuntimeRetirementSnapshotReceipt>,
    ) -> CompleteRuntimeControlRequestInput {
        CompleteRuntimeControlRequestInput {
            request_id: "request_1".to_string(),
            runner_id: "runner-1".to_string(),
            lease_token: "lease-1".to_string(),
            runtime_artifact_id: artifact.map(str::to_string),
            state_schema_version: artifact.map(|_| "state-v1".to_string()),
            runtime_capabilities: None,
            runtime_host: artifact.map(|_| "https://runtime.example".to_string()),
            published_app_urls: artifact.map(|_| vec!["https://app.example".to_string()]),
            retirement_snapshot: receipt,
            now: None,
        }
    }

    fn retirement_receipt() -> RuntimeRetirementSnapshotReceipt {
        RuntimeRetirementSnapshotReceipt {
            schema: RUNTIME_RETIREMENT_SNAPSHOT_SCHEMA.to_string(),
            request_id: "request_1".to_string(),
            project_id: "project_1".to_string(),
            agent_runtime_id: "runtime_1".to_string(),
            durable_state_id: "runtime_1".to_string(),
            runtime_artifact_id: "artifact_1".to_string(),
            backend: RUNTIME_RETIREMENT_BACKEND_BORG.to_string(),
            locator: "retirement-request_1".to_string(),
            zip_bytes: 1,
            zip_sha256: "a".repeat(64),
            manifest_sha256: "b".repeat(64),
            created_at: NOW.to_string(),
            verified_at: NOW.to_string(),
            recovery_authority_id: "finite-assisted-test".to_string(),
            retention_policy: RUNTIME_RETIREMENT_RETENTION_INDEFINITE.to_string(),
        }
    }

    #[test]
    fn runtime_control_completion_parse_pins_the_three_shapes() {
        // Restart/Recover/Stop complete plainly; any facts are a mismatch.
        for kind in [
            RuntimeControlKind::Restart,
            RuntimeControlKind::RecoverKnownGoodChatRuntime,
            RuntimeControlKind::Stop,
        ] {
            assert_eq!(
                RuntimeControlCompletion::parse(kind, &completion_input(None, None)).unwrap(),
                RuntimeControlCompletion::Plain
            );
            assert!(matches!(
                RuntimeControlCompletion::parse(kind, &completion_input(Some("artifact_1"), None)),
                Err(CoreError::RuntimeUpgradeCompletionMismatch)
            ));
            assert!(matches!(
                RuntimeControlCompletion::parse(
                    kind,
                    &completion_input(None, Some(retirement_receipt()))
                ),
                Err(CoreError::RuntimeRetirementSnapshotMismatch)
            ));
        }

        // Upgrade requires the full fact set and rejects the receipt.
        assert!(matches!(
            RuntimeControlCompletion::parse(
                RuntimeControlKind::Upgrade,
                &completion_input(None, None)
            ),
            Err(CoreError::RuntimeUpgradeCompletionMismatch)
        ));
        assert!(matches!(
            RuntimeControlCompletion::parse(
                RuntimeControlKind::Upgrade,
                &completion_input(Some("artifact_1"), Some(retirement_receipt()))
            ),
            Err(CoreError::RuntimeRetirementSnapshotMismatch)
        ));
        assert!(matches!(
            RuntimeControlCompletion::parse(
                RuntimeControlKind::Upgrade,
                &completion_input(Some("artifact_1"), None)
            ),
            Ok(RuntimeControlCompletion::Upgrade(_))
        ));

        // Destroy requires the receipt and rejects upgrade facts.
        assert!(matches!(
            RuntimeControlCompletion::parse(
                RuntimeControlKind::Destroy,
                &completion_input(None, None)
            ),
            Err(CoreError::RuntimeRetirementSnapshotMismatch)
        ));
        assert!(matches!(
            RuntimeControlCompletion::parse(
                RuntimeControlKind::Destroy,
                &completion_input(Some("artifact_1"), Some(retirement_receipt()))
            ),
            Err(CoreError::RuntimeUpgradeCompletionMismatch)
        ));
        assert_eq!(
            RuntimeControlCompletion::parse(
                RuntimeControlKind::Destroy,
                &completion_input(None, Some(retirement_receipt()))
            )
            .unwrap(),
            RuntimeControlCompletion::Destroy(Box::new(retirement_receipt()))
        );
    }

    #[test]
    fn lifecycle_machine_legal_chains_reach_their_terminals() {
        use crate::runtime_lifecycle::{RuntimeLifecycle, phase};

        // The up-bound chain: every up-bound operation passes through Ready
        // before it may be recorded Succeeded.
        let lifecycle = RuntimeLifecycle::<phase::Requested>::enqueue();
        assert_eq!(lifecycle.status(), RuntimeControlRequestStatus::Requested);
        let lifecycle = lifecycle.lease();
        assert_eq!(lifecycle.status(), RuntimeControlRequestStatus::Launching);
        let lifecycle = lifecycle.compute_up(&RuntimeControlCompletion::Plain);
        assert_eq!(lifecycle.status(), RuntimeControlRequestStatus::ComputeUp);
        let lifecycle = lifecycle.ready();
        assert_eq!(lifecycle.status(), RuntimeControlRequestStatus::Ready);
        let terminal = lifecycle.succeed();
        assert_eq!(terminal.status(), RuntimeControlRequestStatus::Succeeded);

        // The down-bound chain: Stop/Destroy confirm straight into Stopped.
        let terminal = RuntimeLifecycle::<phase::Requested>::enqueue()
            .lease()
            .confirm_stopped(&RuntimeControlCompletion::Plain);
        assert_eq!(terminal.status(), RuntimeControlRequestStatus::Stopped);

        // Retirement requeues from Launching back to Requested.
        let retried = RuntimeLifecycle::<phase::Requested>::enqueue()
            .lease()
            .retry();
        assert_eq!(retried.status(), RuntimeControlRequestStatus::Requested);
    }

    #[test]
    fn lifecycle_machine_failure_is_named_from_every_non_terminal_state() {
        use crate::runtime_lifecycle::{RuntimeLifecycle, phase};

        let stages = [
            RuntimeLifecycleStage::Launch,
            RuntimeLifecycleStage::Compute,
            RuntimeLifecycleStage::Readiness,
            RuntimeLifecycleStage::Retirement,
            RuntimeLifecycleStage::Unknown,
        ];
        for stage in stages {
            let failed = RuntimeLifecycle::<phase::Requested>::enqueue().fail(stage);
            assert_eq!(failed.status(), RuntimeControlRequestStatus::Failed);
            assert_eq!(failed.stage(), stage);

            let failed = RuntimeLifecycle::<phase::Requested>::enqueue()
                .lease()
                .fail(stage);
            assert_eq!(failed.stage(), stage);

            let failed = RuntimeLifecycle::<phase::Requested>::enqueue()
                .lease()
                .compute_up(&RuntimeControlCompletion::Plain)
                .fail(stage);
            assert_eq!(failed.stage(), stage);

            let failed = RuntimeLifecycle::<phase::Requested>::enqueue()
                .lease()
                .compute_up(&RuntimeControlCompletion::Plain)
                .ready()
                .fail(stage);
            assert_eq!(failed.stage(), stage);
        }
    }

    #[test]
    fn lifecycle_machine_rehydration_only_accepts_the_exact_phase() {
        use crate::runtime_lifecycle::{RuntimeLifecycle, phase};

        for status in [
            RuntimeControlRequestStatus::Requested,
            RuntimeControlRequestStatus::Launching,
            RuntimeControlRequestStatus::ComputeUp,
            RuntimeControlRequestStatus::Ready,
            RuntimeControlRequestStatus::Succeeded,
            RuntimeControlRequestStatus::Stopped,
            RuntimeControlRequestStatus::Failed,
        ] {
            assert_eq!(
                RuntimeLifecycle::<phase::Requested>::from_status(status).is_some(),
                status == RuntimeControlRequestStatus::Requested
            );
            assert_eq!(
                RuntimeLifecycle::<phase::Launching>::from_status(status).is_some(),
                status == RuntimeControlRequestStatus::Launching
            );
            assert_eq!(
                RuntimeLifecycle::<phase::ComputeUp>::from_status(status).is_some(),
                status == RuntimeControlRequestStatus::ComputeUp
            );
            assert_eq!(
                RuntimeLifecycle::<phase::Ready>::from_status(status).is_some(),
                status == RuntimeControlRequestStatus::Ready
            );
        }
        // Terminal states have no outgoing transitions, so they expose no
        // `from_status` rehydration into a continuing machine at all.
    }

    #[test]
    fn lifecycle_status_terminal_and_active_sets_are_partitioned() {
        for (status, terminal) in [
            (RuntimeControlRequestStatus::Requested, false),
            (RuntimeControlRequestStatus::Launching, false),
            (RuntimeControlRequestStatus::ComputeUp, false),
            (RuntimeControlRequestStatus::Ready, false),
            (RuntimeControlRequestStatus::Succeeded, true),
            (RuntimeControlRequestStatus::Stopped, true),
            (RuntimeControlRequestStatus::Failed, true),
        ] {
            assert_eq!(status.is_terminal(), terminal, "{status:?}");
            assert_eq!(status.is_active(), !terminal, "{status:?}");
        }
        // The N-1 deploy bridge: legacy "running" parses as Launching, and
        // serialization only ever emits the canonical value.
        assert_eq!(
            parse_runtime_control_request_status("running"),
            Some(RuntimeControlRequestStatus::Launching)
        );
        assert_eq!(
            serde_json::to_value(RuntimeControlRequestStatus::Launching).unwrap(),
            serde_json::Value::String("launching".to_string())
        );
    }

    fn phala_runner_capacity(provider_inventory_count: u32) -> RunnerLeaseCapacity {
        RunnerLeaseCapacity {
            runner_classes: vec![RunnerClass::Phala],
            max_sandbox_count: Some(1),
            active_sandbox_count: Some(provider_inventory_count),
            ..RunnerLeaseCapacity::default()
        }
    }

    /// A second handle on the same database with different runtime
    /// configuration.
    ///
    /// The in-memory store took environment/secret references as call
    /// arguments; the real store carries them on the handle. Tests that prove a
    /// persisted runtime spec is reused rather than recomputed lease twice
    /// through differently-configured handles.
    fn with_runtime_config(
        db: &TestDb,
        environment: &BTreeMap<String, String>,
        secret_references: &[String],
    ) -> crate::store::CoreStore {
        db.store
            .clone()
            .with_runtime_environment(environment.clone())
            .unwrap()
            .with_runtime_secret_references(secret_references.to_vec())
            .unwrap()
    }

    /// One Stripe subscription sync for the `cus_order` fixture.
    ///
    /// A plain closure cannot hold `.await`, so the repeated call is a helper.
    async fn sync_order_subscription(
        db: &TestDb,
        org_id: &str,
        status: BillingSubscriptionStatus,
        event: &str,
        created: i64,
    ) -> CustomerBillingAccount {
        db.sync_stripe_subscription(SyncStripeSubscriptionInput {
            customer_org_id: Some(org_id.to_string()),
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
        .await
        .unwrap()
    }

    async fn issue_test_launch_code(db: &TestDb) -> String {
        issue_launch_code(db, None).await
    }

    /// Issue one real launch code batch and return its single plaintext code.
    ///
    /// Staging rows directly is no longer possible (and was never how a code
    /// reaches production), so tests redeem codes the store actually issued.
    async fn issue_launch_code(db: &TestDb, hosting_tier: Option<HostingTier>) -> String {
        db.issue_launch_code_batch(launch_codes::IssueLaunchCodeBatchInput {
            name: "Test batch".to_string(),
            code_count: 1,
            expires_in_hours: Some(launch_codes::MAX_LAUNCH_CODE_BATCH_HOURS),
            hosting_tier,
            created_by_workos_user_id: "workos-test-operator".to_string(),
            now: Some(NOW.to_string()),
        })
        .await
        .unwrap()
        .codes[0]
            .code
            .clone()
    }

    async fn issued_launch_code_id(db: &TestDb, plaintext: &str) -> String {
        let hash = launch_codes::hash_launch_code(plaintext).unwrap();
        db.query_json(
            "SELECT to_jsonb(t) FROM launch_codes t WHERE t.code_hash = $1",
            &[&hash],
        )
        .await
        .first()
        .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// LEGACY-ROW CONTRACT. The existing-host import bridge is deleted, but
    /// production may still hold rows from its 2026-07 near-ship test run.
    /// This test plants those rows the way the bridge left them (raw SQL —
    /// the writing machinery is gone; see git history for the original
    /// reconcile/claim code) and pins the two behaviors that keep them inert:
    ///
    /// 1. A project linked to an import candidate stays out of user-facing
    ///    project lists (`public_visible_projects` filters on
    ///    `import_candidate_id`).
    /// 2. Its capability-less runtime refuses every runtime control
    ///    (`supports_runtime_control` fails closed on NULL capabilities).
    ///
    /// A future importer must define its own linkage and lifecycle rather
    /// than resurrecting these rows' semantics.
    #[tokio::test]
    async fn legacy_import_rows_stay_hidden_and_refuse_runtime_controls() {
        with_isolated_postgres(|db| async move {
            let owner = db
                .link_verified_user(LinkVerifiedUserInput {
                    verified_email: "paul@finite.vip".to_string(),
                    workos_user_id: "user_workos_paul".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            let owner_id = owner.id.clone();
            // link_verified_user already provisioned the personal org; the
            // bridge reused it the same way.
            let org_id = db
                .query_json(
                    "SELECT to_jsonb(t.id) FROM customer_orgs t WHERE t.owner_user_id = $1",
                    &[&owner_id],
                )
                .await[0]
                .as_str()
                .unwrap()
                .to_string();
            for statement in [
                format!(
                    "INSERT INTO project_import_candidates \
                     (id, source_host_id, source_machine_id, source_import_key, owner_email, \
                      pending_user_id, customer_org_id, status, project_id, agent_runtime_id, \
                      claimed_by_user_id, host_facts, created_at, updated_at) \
                     VALUES ('candidate-legacy', 'box1', 'paul-smoke', 'box1:paul-smoke', \
                      'paul@finite.vip', '{owner_id}', '{org_id}', 'claimed', 'project-legacy', \
                      'runtime-legacy', '{owner_id}', \
                      '{{\"display_name\": \"Paul Smoke\", \"hostname\": null, \"runtime_host\": \"box1\", \
                        \"runtime_status\": \"online\", \"active_inference_profile\": null, \
                        \"hermes_available\": null, \"published_app_urls\": []}}', '{NOW}', '{NOW}')"
                ),
                format!(
                    "INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, \
                      import_candidate_id, created_at, updated_at) \
                     VALUES ('project-legacy', '{org_id}', '{owner_id}', 'Paul Smoke', \
                      'candidate-legacy', '{NOW}', '{NOW}')"
                ),
                format!(
                    "INSERT INTO agent_runtimes (id, project_id, source_host_id, source_machine_id, \
                      source_import_key, host_facts, created_at, updated_at) \
                     VALUES ('runtime-legacy', 'project-legacy', 'box1', 'paul-smoke', \
                      'box1:paul-smoke', \
                      '{{\"display_name\": \"Paul Smoke\", \"hostname\": null, \"runtime_host\": \"box1\", \
                        \"runtime_status\": \"online\", \"active_inference_profile\": null, \
                        \"hermes_available\": null, \"published_app_urls\": []}}', '{NOW}', '{NOW}')"
                ),
                format!(
                    "INSERT INTO project_runtime_links (id, project_id, agent_runtime_id, active, created_at) \
                     VALUES ('link-legacy', 'project-legacy', 'runtime-legacy', TRUE, '{NOW}')"
                ),
                // The bridge granted the claiming user a hosted-web owner
                // membership; visibility reads flow through these rows.
                format!(
                    "INSERT INTO chat_identities (id, user_id, kind, device_id, created_at) \
                     VALUES ('identity-legacy', '{owner_id}', 'hosted_web', 'dashboard-bridge-v1', '{NOW}')"
                ),
                format!(
                    "INSERT INTO project_room_memberships (id, project_id, chat_identity_id, role, created_at) \
                     VALUES ('membership-legacy', 'project-legacy', 'identity-legacy', 'owner', '{NOW}')"
                ),
            ] {
                db.exec(&statement).await;
            }

            let visible = db
                .visible_projects_for_workos_user("user_workos_paul")
                .await
                .unwrap();
            let legacy = visible
                .iter()
                .find(|candidate| candidate.project.id == "project-legacy")
                .expect("the legacy project row is still readable internally");
            assert_eq!(
                legacy.project.import_candidate_id.as_deref(),
                Some("candidate-legacy"),
                "the import linkage that keeps this row hidden must survive reads"
            );

            let error = db
                .request_runtime_restart(RequestRuntimeRestartInput {
                    verified_email: "paul@finite.vip".to_string(),
                    workos_user_id: "user_workos_paul".to_string(),
                    project_id: "project-legacy".to_string(),
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(error, CoreError::RuntimeControlUnsupported));
        })
        .await;
    }

    #[tokio::test]
    async fn launch_code_creates_one_self_serve_agent_request_and_visible_project() {
        with_isolated_postgres(|db| async move {
            let launch_code = issue_test_launch_code(&db).await;

            let first = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    display_name: "Oslo Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "first-submit".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            let second = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    display_name: "Oslo Agent duplicate submit".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "first-submit".to_string(),
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap();

            assert!(!first.reused);
            assert!(second.reused);
            assert_eq!(first.request.id, second.request.id);
            assert_eq!(first.project.id, second.project.id);
            assert_eq!(db.table_len("projects").await, 1);
            assert_eq!(db.table_len("agent_runtimes").await, 0);
            assert_eq!(db.table_len("agent_creation_requests").await, 1);
            assert_eq!(first.project.hosting_tier, Some(HostingTier::Standard));
            assert_eq!(
                first.project.placement,
                Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard))
            );
            assert_eq!(first.request.runner_class, RunnerClass::Kata);
            assert_eq!(first.request.hosting_tier, Some(HostingTier::Standard));
            let user = db.all_users().await.into_iter().next().unwrap();
            let org = db.all_customer_orgs().await.into_iter().next().unwrap();
            assert_eq!(org.billing_class, BillingClass::Sponsored);
            assert_eq!(
                db.visible_projects_for_user(&user.id)
                    .await
                    .into_iter()
                    .map(|visible| visible.project)
                    .collect::<Vec<_>>(),
                vec![first.project]
            );
        })
        .await;
    }

    #[tokio::test]
    async fn confidential_launch_code_resolves_phala_placement_inside_core() {
        with_isolated_postgres(|db| async move {
            let launch_code = issue_launch_code(&db, Some(HostingTier::Confidential)).await;
            promote_runtime_artifact(&db).await;

            let requested = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "confidential@finite.vip".to_string(),
                    workos_user_id: "user_workos_confidential".to_string(),
                    display_name: "Confidential Agent".to_string(),
                    launch_code,
                    idempotency_key: "confidential-submit".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();

            assert_eq!(
                requested.project.hosting_tier,
                Some(HostingTier::Confidential)
            );
            assert_eq!(
                requested.project.placement,
                Some(RuntimePlacement::for_hosting_tier(
                    HostingTier::Confidential
                ))
            );
            assert_eq!(requested.request.runner_class, RunnerClass::Phala);

            let lease = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "phala-runner".to_string(),
                    source_host_id: Some("phala-host".to_string()),
                    lease_token: "phala-lease".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                            RuntimeCapabilitiesV1 {
                                restart: true,
                                stop: true,
                                ..RuntimeCapabilitiesV1::default()
                            },
                        )),
                        ..phala_runner_capacity(0)
                    }),
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap()
                .unwrap();
            let error = db
                .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                    request_id: lease.request.id,
                    runner_id: "phala-runner".to_string(),
                    lease_token: "phala-lease".to_string(),
                    source_host_id: "phala-host".to_string(),
                    source_machine_id: "phala-cvm".to_string(),
                    runtime_artifact_id: Some("artifact-v1".to_string()),
                    state_schema_version: Some("db-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: None,
                    runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                        RuntimeCapabilitiesV1 {
                            restart: true,
                            runtime_upgrade: true,
                            stop: true,
                            ..RuntimeCapabilitiesV1::default()
                        },
                    )),
                    display_name: None,
                    hostname: None,
                    runtime_host: None,
                    runtime_status: Some(RuntimeSummaryStatus::Unknown),
                    active_inference_profile: None,
                    hermes_available: None,
                    published_app_urls: Vec::new(),
                    now: Some("2026-05-25T13:01:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(error, CoreError::RuntimeCapabilitiesNotAuthorized));
        })
        .await;
    }

    #[tokio::test]
    async fn selected_hosting_tier_must_match_launch_code_before_code_redemption() {
        with_isolated_postgres(|db| async move {
            let launch_code = issue_test_launch_code(&db).await;
            let input = RequestAgentCreationInput {
                verified_email: "tier-check@finite.vip".to_string(),
                workos_user_id: "user_workos_tier_check".to_string(),
                display_name: "Tier Check".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: "tier-check-submit".to_string(),
                now: Some(NOW.to_string()),
            };

            let denied = db
                .request_agent_creation_configured(
                    input.clone(),
                    AgentCreationConfiguration {
                        requested_hosting_tier: Some(HostingTier::Confidential),
                        ..AgentCreationConfiguration::default()
                    },
                )
                .await
                .unwrap_err();
            assert!(matches!(denied, CoreError::HostingTierNotAuthorized));
            assert!(db.all_users().await.is_empty());
            assert!(db.all_projects().await.is_empty());
            assert!(db.all_agent_creation_requests().await.is_empty());

            let created = db
                .request_agent_creation_configured(
                    input,
                    AgentCreationConfiguration {
                        requested_hosting_tier: Some(HostingTier::Standard),
                        ..AgentCreationConfiguration::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(created.request.runner_class, RunnerClass::Kata);
        })
        .await;
    }

    #[tokio::test]
    async fn phala_capacity_reservation_is_atomic_and_releases_only_the_existing_in_flight_request()
    {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let mut request_ids = Vec::new();
            for index in 0..2 {
                let launch_code = issue_launch_code(&db, Some(HostingTier::Confidential)).await;
                let requested = db
                    .request_agent_creation(RequestAgentCreationInput {
                        verified_email: format!("confidential-{index}@finite.vip"),
                        workos_user_id: format!("user_workos_confidential_{index}"),
                        display_name: format!("Confidential Agent {index}"),
                        launch_code,
                        idempotency_key: format!("confidential-submit-{index}"),
                        now: Some(NOW.to_string()),
                    })
                    .await
                    .unwrap();
                request_ids.push(requested.request.id);
            }

            let first = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "phala-runner-a".to_string(),
                    source_host_id: Some("phala-host".to_string()),
                    lease_token: "phala-lease-a".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(phala_runner_capacity(0)),
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap()
                .unwrap();
            assert!(request_ids.contains(&first.request.id));
            let waiting_request_id = request_ids
                .iter()
                .find(|request_id| request_id.as_str() != first.request.id)
                .unwrap();
            let reservation = first.in_flight_capacity_reservation.as_ref().unwrap().v1();
            assert_eq!(reservation.request_id, first.request.id);
            assert_eq!(
                reservation.placement,
                RuntimePlacement::for_hosting_tier(HostingTier::Confidential)
            );
            assert_eq!(reservation.provider_inventory_count, 0);
            assert_eq!(reservation.core_in_flight_count, 1);
            assert_eq!(reservation.max_sandbox_count, 1);

            let second = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "phala-runner-b".to_string(),
                    source_host_id: Some("phala-host".to_string()),
                    lease_token: "phala-lease-b".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(phala_runner_capacity(0)),
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap();
            assert!(second.is_none());

            let resumed = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "phala-runner-c".to_string(),
                    source_host_id: Some("phala-host".to_string()),
                    lease_token: "phala-lease-c".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(phala_runner_capacity(1)),
                    now: Some("2026-05-25T14:00:00Z".to_string()),
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(resumed.request.id, first.request.id);
            let reservation = resumed
                .in_flight_capacity_reservation
                .as_ref()
                .unwrap()
                .v1();
            assert_eq!(reservation.provider_inventory_count, 1);
            assert_eq!(reservation.core_in_flight_count, 1);
            assert_eq!(
                db.agent_creation_request(waiting_request_id)
                    .await
                    .unwrap()
                    .status,
                AgentCreationRequestStatus::Requested
            );
        })
        .await;
    }

    #[tokio::test]
    async fn project_selected_runner_class_routes_to_a_matching_worker() {
        with_isolated_postgres(|db| async move {
            let launch_code = issue_test_launch_code(&db).await;
            promote_runtime_artifact(&db).await;
            let requested = db
                .request_agent_creation_configured(
                    RequestAgentCreationInput {
                        verified_email: "kata@finite.vip".to_string(),
                        workos_user_id: "user_workos_kata".to_string(),
                        display_name: "Kata Agent".to_string(),
                        launch_code: launch_code.clone(),
                        idempotency_key: "kata-submit".to_string(),
                        now: Some(NOW.to_string()),
                    },
                    AgentCreationConfiguration {
                        placement: Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
                        requested_hosting_tier: None,
                        profile_picture_url: Some(
                            "https://chat.finite.computer/v1/blobs/profile".to_string(),
                        ),
                        owner_chat_account_id: None,
                    },
                )
                .await
                .unwrap();
            assert_eq!(requested.request.runner_class, RunnerClass::Kata);

            let draining_kata = RunnerLeaseCapacity {
                draining: true,
                runner_classes: vec![RunnerClass::Kata],
                runtime_capabilities: Some(kata_runtime_capabilities()),
                ..RunnerLeaseCapacity::default()
            };
            assert!(!draining_kata.accepts_agent_creation());
            assert!(draining_kata.accepts_runtime_control());

            let phala = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "phala-worker".to_string(),
                    source_host_id: None,
                    lease_token: "phala-lease".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(phala_runner_capacity(0)),
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap();
            assert!(phala.is_none());

            let unspecified = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "unspecified-worker".to_string(),
                    source_host_id: None,
                    lease_token: "unspecified-lease".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(RunnerLeaseCapacity::default()),
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap();
            assert!(unspecified.is_none());

            let kata = db
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
                .await
                .unwrap()
                .expect("Kata worker should claim Kata placement");
            assert_eq!(kata.request.id, requested.request.id);
        })
        .await;
    }

    #[tokio::test]
    async fn creation_retry_reuses_the_persisted_complete_runtime_spec() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let launch_code = issue_test_launch_code(&db).await;
            let requested = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "retry@finite.vip".to_string(),
                    workos_user_id: "user_workos_retry".to_string(),
                    display_name: "Retry Agent".to_string(),
                    launch_code,
                    idempotency_key: "retry-submit".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            let original_environment = BTreeMap::from([(
                "FINITE_SITES_API".to_string(),
                "https://api.finite.chat".to_string(),
            )]);
            let original_secret_references = vec![
                "FAL_KEY".to_string(),
                "FIRECRAWL_API_KEY".to_string(),
                "XAI_API_KEY".to_string(),
            ];
            let first =
                with_runtime_config(&db, &original_environment, &original_secret_references)
                    .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                        runner_id: "kata-worker-1".to_string(),
                        source_host_id: None,
                        lease_token: "lease-one".to_string(),
                        lease_seconds: Some(300),
                        runner_capacity: Some(RunnerLeaseCapacity {
                            runner_classes: vec![RunnerClass::Kata],
                            ..RunnerLeaseCapacity::default()
                        }),
                        now: Some(LATER.to_string()),
                    })
                    .await
                    .unwrap()
                    .unwrap();
            let first_spec = first.request.runtime_spec.clone().unwrap();
            let first_runtime_id = first.request.agent_runtime_id.clone().unwrap();
            let first_spec_v1 = runtime_spec_v1(&first_spec);
            assert_eq!(first_spec_v1.operation_id, requested.request.id);
            assert_eq!(first_spec_v1.agent_runtime_id, first_runtime_id);
            assert_eq!(first_spec_v1.durable_state_id, first_runtime_id);
            assert_eq!(first_spec_v1.environment, original_environment);
            assert_eq!(
                first_spec_v1.secret_references,
                vec![
                    FINITE_PRIVATE_SECRET_REFERENCE.to_string(),
                    "FAL_KEY".to_string(),
                    "FIRECRAWL_API_KEY".to_string(),
                    "XAI_API_KEY".to_string(),
                ]
            );

            promote_runtime_artifact_version(
                &db,
                "artifact-v2",
                &format!(
                    "ghcr.io/finitecomputer/agent-runtime:v2@sha256:{}",
                    "b".repeat(64)
                ),
                "v2",
                "db-v1",
                "2026-05-25T13:05:00Z",
            )
            .await;
            let second = with_runtime_config(
                &db,
                &BTreeMap::from([(
                    "FINITE_SITES_API".to_string(),
                    "https://changed.example.test".to_string(),
                )]),
                &["PERPLEXITY_API_KEY".to_string()],
            )
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "kata-worker-2".to_string(),
                source_host_id: None,
                lease_token: "lease-two".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some("2026-05-25T13:06:00Z".to_string()),
            })
            .await
            .unwrap()
            .unwrap();

            assert_eq!(second.request.runtime_spec.as_ref(), Some(&first_spec));
            assert_eq!(
                second.request.desired_runtime_artifact_id.as_deref(),
                Some("artifact-v1")
            );
            assert_eq!(
                second.request.agent_runtime_id.as_deref(),
                Some(first_runtime_id.as_str())
            );
        })
        .await;
    }

    #[test]
    fn configured_runtime_secret_references_are_bounded_unique_and_cannot_override_inference() {
        assert!(
            runtime_spec_secret_references(&[
                "FAL_KEY".to_string(),
                "X_API_BEARER_TOKEN".to_string(),
            ])
            .is_ok()
        );
        for invalid in [
            vec!["OPENAI_API_KEY".to_string()],
            vec!["FAL_KEY".to_string(), "FAL_KEY".to_string()],
            vec!["FINITE_SITES_API".to_string()],
            vec![FINITE_PRIVATE_SECRET_REFERENCE.to_string()],
        ] {
            assert!(runtime_spec_secret_references(&invalid).is_err());
        }
    }

    /// A launch key provisioned before a provider failure stays usable until
    /// the request is finally cancelled.
    ///
    /// Split from the ledger fencing test: failing and cancelling terminates the
    /// request, so it cannot share a database with the assertions that continue
    /// to drive the same request.
    #[tokio::test]
    async fn abandoned_launch_key_survives_failure_and_is_revoked_by_cancellation() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let launch_code = issue_test_launch_code(&db).await;
            let requested = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "abandoned@finite.vip".to_string(),
                    workos_user_id: "workos-abandoned".to_string(),
                    display_name: "Abandoned Agent".to_string(),
                    launch_code,
                    idempotency_key: "abandoned-submit".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            let request_id = requested.request.id;
            db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-a".to_string(),
                lease_token: "token-a".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                source_host_id: None,
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap()
            .unwrap();
            db.exec(&format!(
                "UPDATE agent_creation_requests SET lease_expires_at = '2099-01-01T00:00:00Z' \
                 WHERE id = '{request_id}'"
            ))
            .await;
            let reserved = db
                .record_provider_operation_transition(RecordProviderOperationTransitionInput {
                    request_id: request_id.clone(),
                    runner_id: "runner-a".to_string(),
                    lease_token: "token-a".to_string(),
                    correlation_id: "provider-correlation-1".to_string(),
                    placement: RuntimePlacement::for_hosting_tier(HostingTier::Standard),
                    transition: ProviderOperationTransition::CorrelationReserved,
                })
                .await
                .unwrap();

            let abandoned_key = db
                .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                    request_id: request_id.clone(),
                    runner_id: "runner-a".to_string(),
                    lease_token: "token-a".to_string(),
                    source_host_id: None,
                    source_machine_id: None,
                    now: Some("2098-01-01T00:00:20Z".to_string()),
                })
                .await
                .unwrap();
            let failed = db
                .fail_agent_creation_request(FailAgentCreationRequestInput {
                    request_id: request_id.clone(),
                    runner_id: "runner-a".to_string(),
                    lease_token: "token-a".to_string(),
                    failure_message: "provider launch failed".to_string(),
                    provisioned_finite_private_api_key_id: None,
                    now: Some("2098-01-01T00:00:30Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(failed.status, AgentCreationRequestStatus::Failed);
            assert_eq!(
                db.finite_private_api_key(&abandoned_key.api_key.id)
                    .await
                    .unwrap()
                    .status,
                FinitePrivateApiKeyStatus::Active,
                "failure cannot revoke a launch key the runner failed to identify"
            );
            assert_eq!(
                db.provider_operation(&request_id).await.as_ref(),
                Some(&reserved),
                "the accepted pre-provider failure keeps its audit journal"
            );

            let cancelled = db
                .cancel_agent_creation_request(CancelAgentCreationRequestInput {
                    request_id: request_id.clone(),
                    now: Some("2098-01-01T00:00:31Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(cancelled.status, AgentCreationRequestStatus::Cancelled);
            assert_eq!(
                db.finite_private_api_key(&abandoned_key.api_key.id)
                    .await
                    .unwrap()
                    .status,
                FinitePrivateApiKeyStatus::Revoked,
                "final cancellation revokes an otherwise abandoned project launch key"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn provider_operation_ledger_is_fenced_monotonic_and_survives_re_lease() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let launch_code = issue_test_launch_code(&db).await;
            let requested = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "ledger@finite.vip".to_string(),
                    workos_user_id: "workos-ledger".to_string(),
                    display_name: "Ledger Agent".to_string(),
                    launch_code,
                    idempotency_key: "ledger-submit".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            let request_id = requested.request.id;
            db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-a".to_string(),
                lease_token: "token-a".to_string(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    ..RunnerLeaseCapacity::default()
                }),
                source_host_id: None,
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap()
            .unwrap();
            db.exec(&format!(
                "UPDATE agent_creation_requests SET lease_expires_at = '2099-01-01T00:00:00Z' \
                 WHERE id = '{request_id}'"
            ))
            .await;
            let placement = RuntimePlacement::for_hosting_tier(HostingTier::Standard);
            let input = |runner: &str,
                         token: &str,
                         correlation: &str,
                         transition: ProviderOperationTransition| {
                RecordProviderOperationTransitionInput {
                    request_id: request_id.clone(),
                    runner_id: runner.to_string(),
                    lease_token: token.to_string(),
                    correlation_id: correlation.to_string(),
                    placement,
                    transition,
                }
            };
            let fail_input =
                |runner: &str, token: &str, key_id: Option<String>| FailAgentCreationRequestInput {
                    request_id: request_id.clone(),
                    runner_id: runner.to_string(),
                    lease_token: token.to_string(),
                    failure_message: "provider launch failed".to_string(),
                    provisioned_finite_private_api_key_id: key_id,
                    now: Some("2098-01-01T00:00:30Z".to_string()),
                };

            let reserved = db
                .record_provider_operation_transition(input(
                    "runner-a",
                    "token-a",
                    "provider-correlation-1",
                    ProviderOperationTransition::CorrelationReserved,
                ))
                .await
                .unwrap();
            let replay = db
                .record_provider_operation_transition(input(
                    "runner-a",
                    "token-a",
                    "provider-correlation-1",
                    ProviderOperationTransition::CorrelationReserved,
                ))
                .await
                .unwrap();
            assert_eq!(replay, reserved, "replay returns the exact persisted ack");
            db.exec(&format!(
                "UPDATE agent_creation_requests SET lease_expires_at = '2020-01-01T00:00:00Z' \
                 WHERE id = '{request_id}'"
            ))
            .await;
            assert!(matches!(
                db.fail_agent_creation_request(fail_input("runner-a", "token-a", None))
                    .await,
                Err(CoreError::AgentCreationRequestLeaseConflict)
            ));
            assert_eq!(
                db.provider_operation(&request_id).await.as_ref(),
                Some(&reserved)
            );
            assert_eq!(
                db.agent_creation_request(&request_id).await.unwrap().status,
                AgentCreationRequestStatus::Launching
            );
            db.exec(&format!(
                "UPDATE agent_creation_requests SET lease_expires_at = '2099-01-01T00:00:00Z' \
                 WHERE id = '{request_id}'"
            ))
            .await;
            assert!(matches!(
                db.record_provider_operation_transition(input(
                    "runner-a",
                    "wrong-token",
                    "provider-correlation-1",
                    ProviderOperationTransition::CorrelationReserved,
                ))
                .await,
                Err(CoreError::AgentCreationRequestLeaseConflict)
            ));
            assert!(matches!(
                db.record_provider_operation_transition(input(
                    "runner-a",
                    "token-a",
                    "provider-correlation-1",
                    ProviderOperationTransition::Provisioned {
                        provider_facts: json!({"api_token": "must-not-persist"}),
                    },
                ))
                .await,
                Err(CoreError::InvalidProviderOperationFacts)
            ));
            assert!(matches!(
                db.record_provider_operation_transition(input(
                    "runner-a",
                    "token-a",
                    "provider-correlation-1",
                    ProviderOperationTransition::Provisioned {
                        provider_facts: json!({"provider_id": "must-not-skip-start"}),
                    },
                ))
                .await,
                Err(CoreError::ProviderOperationTransitionConflict)
            ));

            let provision_started = db
                .record_provider_operation_transition(input(
                    "runner-a",
                    "token-a",
                    "provider-correlation-1",
                    ProviderOperationTransition::ProvisionStarted,
                ))
                .await
                .unwrap();
            assert!(matches!(
                db.fail_agent_creation_request(fail_input("runner-a", "token-a", None))
                    .await,
                Err(CoreError::ProviderOperationBoundaryNotReached)
            ));
            assert!(matches!(
                db.cancel_agent_creation_request(CancelAgentCreationRequestInput {
                    request_id: request_id.clone(),
                    now: Some("2098-01-01T00:00:32Z".to_string()),
                })
                .await,
                Err(CoreError::ProviderOperationBoundaryNotReached)
            ));
            assert_eq!(
                db.provider_operation(&request_id).await.as_ref(),
                Some(&provision_started),
                "a crash after the pre-mutation fence remains resumable"
            );

            let provision_unknown = db
                .record_provider_operation_transition(input(
                    "runner-a",
                    "token-a",
                    "provider-correlation-1",
                    ProviderOperationTransition::ProvisionUnknown {
                        provider_facts: json!({"attempt": "timed_out"}),
                    },
                ))
                .await
                .unwrap();
            assert!(matches!(
                db.fail_agent_creation_request(fail_input("runner-a", "token-a", None))
                    .await,
                Err(CoreError::ProviderOperationBoundaryNotReached)
            ));
            assert_eq!(
                db.provider_operation(&request_id).await.as_ref(),
                Some(&provision_unknown)
            );
            assert!(matches!(
                db.record_provider_operation_transition(input(
                    "runner-a",
                    "token-a",
                    "provider-correlation-1",
                    ProviderOperationTransition::CommitStarted,
                ))
                .await,
                Err(CoreError::ProviderOperationTransitionConflict)
            ));
            let provisioned = db
                .record_provider_operation_transition(input(
                    "runner-a",
                    "token-a",
                    "provider-correlation-1",
                    ProviderOperationTransition::Provisioned {
                        provider_facts: json!({"provider_id": "opaque-123", "region": "test"}),
                    },
                ))
                .await
                .unwrap();
            let provisioned_key = db
                .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                    request_id: request_id.clone(),
                    runner_id: "runner-a".to_string(),
                    lease_token: "token-a".to_string(),
                    source_host_id: Some("ledger-host".to_string()),
                    source_machine_id: Some("ledger-machine".to_string()),
                    now: Some("2098-01-01T00:00:40Z".to_string()),
                })
                .await
                .unwrap();
            assert!(matches!(
                db.fail_agent_creation_request(fail_input(
                    "runner-a",
                    "token-a",
                    Some(provisioned_key.api_key.id.clone()),
                ))
                .await,
                Err(CoreError::ProviderOperationBoundaryNotReached)
            ));
            assert_eq!(
                db.provider_operation(&request_id).await.as_ref(),
                Some(&provisioned)
            );
            assert_eq!(
                db.finite_private_api_key(&provisioned_key.api_key.id)
                    .await
                    .unwrap()
                    .status,
                FinitePrivateApiKeyStatus::Active
            );
            assert!(matches!(
                db.cancel_agent_creation_request(CancelAgentCreationRequestInput {
                    request_id: request_id.clone(),
                    now: Some("2098-01-01T00:00:41Z".to_string()),
                })
                .await,
                Err(CoreError::ProviderOperationBoundaryNotReached)
            ));
            let committed = db
                .record_provider_operation_transition(input(
                    "runner-a",
                    "token-a",
                    "provider-correlation-1",
                    ProviderOperationTransition::CommitStarted,
                ))
                .await
                .unwrap();
            assert!(matches!(
                db.fail_agent_creation_request(fail_input(
                    "runner-a",
                    "token-a",
                    Some(provisioned_key.api_key.id.clone()),
                ))
                .await,
                Err(CoreError::ProviderOperationBoundaryNotReached)
            ));
            assert_eq!(
                db.provider_operation(&request_id).await.as_ref(),
                Some(&committed)
            );

            db.exec(&format!(
                "UPDATE agent_creation_requests SET lease_expires_at = '2097-01-01T00:00:00Z' \
                 WHERE id = '{request_id}'"
            ))
            .await;
            let second = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-b".to_string(),
                    lease_token: "token-b".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        ..RunnerLeaseCapacity::default()
                    }),
                    source_host_id: None,
                    now: Some("2098-01-01T00:00:00Z".to_string()),
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(second.provider_operation.as_ref(), Some(&committed));
            assert!(matches!(
                db.record_provider_operation_transition(input(
                    "runner-a",
                    "token-a",
                    "provider-correlation-1",
                    ProviderOperationTransition::CommitStarted,
                ))
                .await,
                Err(CoreError::AgentCreationRequestLeaseConflict)
            ));
            assert!(matches!(
                db.record_provider_operation_transition(input(
                    "runner-b",
                    "token-b",
                    "different-correlation",
                    ProviderOperationTransition::CommitStarted,
                ))
                .await,
                Err(CoreError::ProviderOperationIdentityMismatch)
            ));
            let replay_after_crash = db
                .record_provider_operation_transition(input(
                    "runner-b",
                    "token-b",
                    "provider-correlation-1",
                    ProviderOperationTransition::CommitStarted,
                ))
                .await
                .unwrap();
            assert_eq!(replay_after_crash, committed);

            let handle = ProviderRuntimeHandleEnvelope::V1(ProviderRuntimeHandleV1 {
                runner_class: RunnerClass::Kata,
                opaque: json!({"sandbox_id": "opaque-123"}),
            });
            let registered = db
                .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                    request_id: request_id.clone(),
                    runner_id: "runner-b".to_string(),
                    lease_token: "token-b".to_string(),
                    source_host_id: "ledger-host".to_string(),
                    source_machine_id: "ledger-machine".to_string(),
                    runtime_artifact_id: Some("artifact-v1".to_string()),
                    state_schema_version: None,
                    provider_runtime_handle: Some(handle.clone()),
                    contact_endpoint: None,
                    runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                        RuntimeCapabilitiesV1 {
                            recover_known_good_chat: true,
                            ..*kata_runtime_capabilities().v1()
                        },
                    )),
                    display_name: None,
                    hostname: None,
                    runtime_host: None,
                    runtime_status: None,
                    active_inference_profile: None,
                    hermes_available: None,
                    published_app_urls: Vec::new(),
                    now: Some("2098-01-01T00:01:00Z".to_string()),
                })
                .await
                .unwrap();
            assert!(
                !db.agent_runtime(registered.request.agent_runtime_id.as_ref().unwrap())
                    .await
                    .unwrap()
                    .runtime_capabilities
                    .as_ref()
                    .unwrap()
                    .v1()
                    .recover_known_good_chat,
                "an old artifact bounds the worker's process-wide recovery maximum"
            );
            assert!(matches!(
                registered
                    .provider_operation
                    .as_ref()
                    .unwrap()
                    .v1()
                    .transitions
                    .last()
                    .unwrap()
                    .transition,
                ProviderOperationTransition::ProviderHandleRecorded { .. }
            ));
            let runtime_id = registered.request.agent_runtime_id.clone().unwrap();
            let handle_recorded = registered.provider_operation.clone().unwrap();
            assert!(matches!(
                db.fail_agent_creation_request(fail_input(
                    "runner-b",
                    "token-b",
                    Some(provisioned_key.api_key.id.clone()),
                ))
                .await,
                Err(CoreError::ProviderOperationBoundaryNotReached)
            ));
            assert_eq!(
                db.provider_operation(&request_id).await.as_ref(),
                Some(&handle_recorded)
            );
            assert!(db.agent_runtime(&runtime_id).await.is_some());
            assert_eq!(
                db.finite_private_api_key(&provisioned_key.api_key.id)
                    .await
                    .unwrap()
                    .status,
                FinitePrivateApiKeyStatus::Active
            );
            assert!(matches!(
                db.cancel_agent_creation_request(CancelAgentCreationRequestInput {
                    request_id: request_id.clone(),
                    now: Some("2098-01-01T00:01:01Z".to_string()),
                })
                .await,
                Err(CoreError::ProviderOperationBoundaryNotReached)
            ));
            assert!(db.agent_runtime(&runtime_id).await.is_some());
            let completed = db
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id,
                    runner_id: "runner-b".to_string(),
                    lease_token: "token-b".to_string(),
                    source_host_id: "ledger-host".to_string(),
                    source_machine_id: "ledger-machine".to_string(),
                    runtime_artifact_id: Some("artifact-v1".to_string()),
                    state_schema_version: None,
                    provider_runtime_handle: Some(handle),
                    contact_endpoint: None,
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: None,
                    hostname: None,
                    runtime_host: None,
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: None,
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: Some("2098-01-01T00:02:00Z".to_string()),
                })
                .await
                .unwrap();
            assert!(matches!(
                completed
                    .provider_operation
                    .unwrap()
                    .v1()
                    .transitions
                    .last()
                    .unwrap()
                    .transition,
                ProviderOperationTransition::Ready
            ));
        })
        .await;
    }

    #[tokio::test]
    async fn kata_is_the_only_runtime_recovery_capability_boundary() {
        with_isolated_postgres(|db| async move {
            let recover = RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                recover_known_good_chat: true,
                ..RuntimeCapabilitiesV1::default()
            });
            assert!(
                RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(recover.clone()),
                    ..RunnerLeaseCapacity::default()
                }
                .validate_runtime_capability_policy()
                .is_ok()
            );
            for runner_classes in [
                Vec::new(),
                vec![RunnerClass::Phala],
                vec![RunnerClass::Kata, RunnerClass::Phala],
            ] {
                assert!(matches!(
                    (RunnerLeaseCapacity {
                        runner_classes,
                        runtime_capabilities: Some(recover.clone()),
                        ..RunnerLeaseCapacity::default()
                    })
                    .validate_runtime_capability_policy(),
                    Err(CoreError::RuntimeCapabilitiesNotAuthorized)
                ));
            }
            assert!(
                validate_runtime_capabilities_policy(
                    Some(&recover),
                    Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard))
                )
                .is_ok()
            );
            assert!(matches!(
                validate_runtime_capabilities_policy(
                    Some(&recover),
                    Some(RuntimePlacement::for_hosting_tier(
                        HostingTier::Confidential
                    ))
                ),
                Err(CoreError::RuntimeCapabilitiesNotAuthorized)
            ));
            promote_runtime_artifact(&db).await;
            let legacy_artifact = db
                .runtime_artifact_row("artifact-v1")
                .await
                .unwrap()
                .clone();
            assert!(matches!(
                validate_runtime_capabilities_artifact_policy(
                    Some(&recover),
                    Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
                    &legacy_artifact,
                ),
                Err(CoreError::RuntimeCapabilitiesNotAuthorized)
            ));
            let capable_artifact = RuntimeArtifact {
                recover_known_good_chat: true,
                ..legacy_artifact.clone()
            };
            assert!(
                validate_runtime_capabilities_artifact_policy(
                    Some(&recover),
                    Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
                    &capable_artifact,
                )
                .is_ok()
            );
            assert!(
                !runtime_artifact_material_matches(&legacy_artifact, &capable_artifact),
                "artifact recovery support is immutable release material"
            );
            for key in ["FINITE_AGENT_BOOT_INTENT_JSON", "FINITE_AGENT_STATE_ROOT"] {
                assert!(matches!(
                    validate_runtime_spec_environment(&BTreeMap::from([(
                        key.to_string(),
                        "caller-owned".to_string()
                    )])),
                    Err(CoreError::RuntimeSpecMismatch)
                ));
            }
        })
        .await;
    }

    #[tokio::test]
    async fn runner_leases_and_completes_self_serve_agent_request() {
        with_isolated_postgres(|db| async move {
            let launch_code = issue_test_launch_code(&db).await;
            promote_runtime_artifact(&db).await;
            db.exec("UPDATE runtime_artifacts SET recover_known_good_chat = true WHERE id = 'artifact-v1'")
                .await;
            let requested = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    display_name: "Oslo Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "first-submit".to_string(),
                    now: Some(NOW.to_string()),
                }).await
                .unwrap();

            let lease = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    source_host_id: None,
                    lease_token: "lease-token-1".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: Some(LATER.to_string()),
                }).await
                .unwrap()
                .expect("pending request should be leased");
            assert_eq!(lease.project.id, requested.project.id);
            assert_eq!(lease.request.status, AgentCreationRequestStatus::Launching);
            assert_eq!(lease.request.runner_id.as_deref(), Some("runner-oslo-1"));
            assert!(lease.request.lease_expires_at.is_some());

            let none = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-oslo-2".to_string(),
                    source_host_id: None,
                    lease_token: "lease-token-2".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: Some("2026-05-25T13:01:00Z".to_string()),
                }).await
                .unwrap();
            assert!(none.is_none());

            let completed = db
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: lease.request.id.clone(),
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "lease-token-1".to_string(),
                    source_host_id: "oslo-host-1".to_string(),
                    source_machine_id: "oslo-agent-001".to_string(),
                    runtime_artifact_id: Some("artifact-v1".to_string()),
                    state_schema_version: None,
                    provider_runtime_handle: Some(ProviderRuntimeHandleEnvelope::V1(
                        ProviderRuntimeHandleV1 {
                            runner_class: RunnerClass::Kata,
                            opaque: json!({"container": "finite-kata-oslo-001"}),
                        },
                    )),
                    contact_endpoint: Some("https://oslo-agent.example.com/contact/".to_string()),
                    runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                        RuntimeCapabilitiesV1 {
                            recover_known_good_chat: true,
                            ..*kata_runtime_capabilities().v1()
                        },
                    )),
                    display_name: None,
                    hostname: Some("oslo-agent-001.finite.computer".to_string()),
                    runtime_host: Some("oslo-host-1".to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: Some("2026-05-25T13:02:00Z".to_string()),
                }).await
                .unwrap();

            assert_eq!(
                completed.request.status,
                AgentCreationRequestStatus::Running
            );
            assert!(completed.request.lease_token.is_none());
            let runtime_id = completed.request.agent_runtime_id.unwrap();
            let runtime = db.agent_runtime(&runtime_id).await.unwrap();
            assert!(
                runtime
                    .runtime_capabilities
                    .as_ref()
                    .unwrap()
                    .v1()
                    .recover_known_good_chat
            );
            assert_eq!(runtime.project_id, requested.project.id);
            assert_eq!(runtime.runtime_artifact_id.as_deref(), Some("artifact-v1"));
            assert_eq!(runtime.state_schema_version.as_deref(), Some("db-v1"));
            assert_eq!(runtime.source_host_id, "oslo-host-1");
            assert_eq!(runtime.source_machine_id, "oslo-agent-001");
            assert_eq!(
                runtime.host_facts.runtime_status,
                RuntimeSummaryStatus::Online
            );
            assert_eq!(
                db.all("project_runtime_links").await.iter()
                    .filter(|link| link["project_id"] == requested.project.id.as_str() && link["active"] == true)
                    .count(),
                1
            );
        })
        .await;
    }

    #[tokio::test]
    async fn runtime_artifact_promotion_does_not_mutate_healthy_running_agent() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_a = complete_self_serve_agent(
                &db,
                "a@finite.vip",
                "user_workos_a",
                "agent-a",
                "oslo-agent-a",
                "artifact-v1",
                "2026-05-25T13:02:00Z",
            )
            .await;
            let runtime_a_before = db.agent_runtime(&runtime_a).await.unwrap().clone();

            promote_runtime_artifact_version(
                &db,
                "artifact-v2",
                &format!(
                    "ghcr.io/finitecomputer/agent-runtime:v2@sha256:{}",
                    "b".repeat(64)
                ),
                "v2",
                "db-v1",
                "2026-05-25T14:00:00Z",
            )
            .await;

            assert_eq!(
                db.agent_runtime(&runtime_a).await.unwrap(),
                runtime_a_before
            );
            assert_eq!(
                db.agent_runtime(&runtime_a)
                    .await
                    .unwrap()
                    .runtime_artifact_id
                    .as_deref(),
                Some("artifact-v1")
            );

            let runtime_b = complete_self_serve_agent(
                &db,
                "b@finite.vip",
                "user_workos_b",
                "agent-b",
                "oslo-agent-b",
                "artifact-v2",
                "2026-05-25T14:05:00Z",
            )
            .await;
            assert_eq!(
                db.agent_runtime(&runtime_b)
                    .await
                    .unwrap()
                    .runtime_artifact_id
                    .as_deref(),
                Some("artifact-v2")
            );
            assert_eq!(
                db.agent_runtime(&runtime_a)
                    .await
                    .unwrap()
                    .runtime_artifact_id
                    .as_deref(),
                Some("artifact-v1")
            );
        })
        .await;
    }

    #[tokio::test]
    async fn promoted_or_runtime_referenced_artifact_material_is_immutable() {
        with_isolated_postgres(|db| async move {
            let input = UpsertRuntimeArtifactInput {
                id: "artifact-immutable".to_string(),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!("ghcr.io/finite/runtime@sha256:{}", "a".repeat(64)),
                version_label: "v1".to_string(),
                source_git_sha: Some("git-v1".to_string()),
                finitec_version: Some("finitec-v1".to_string()),
                hermes_source_ref: Some("hermes-v1".to_string()),
                finite_platform_plugin_ref: Some("plugin-v1".to_string()),
                state_schema_version: "db-v1".to_string(),
                base_image: Some("base-v1".to_string()),
                recover_known_good_chat: false,
                promoted: false,
                now: Some(NOW.to_string()),
            };
            db.upsert_runtime_artifact(input.clone()).await.unwrap();

            let mut before_promotion = input.clone();
            before_promotion.version_label = "v1-corrected".to_string();
            db.upsert_runtime_artifact(before_promotion.clone())
                .await
                .unwrap();
            before_promotion.promoted = true;
            db.upsert_runtime_artifact(before_promotion.clone())
                .await
                .unwrap();

            let mut exact_retry = before_promotion.clone();
            exact_retry.now = Some(LATER.to_string());
            db.upsert_runtime_artifact(exact_retry).await.unwrap();
            let mut mutation = before_promotion;
            mutation.reference = format!("ghcr.io/finite/runtime@sha256:{}", "b".repeat(64));
            assert!(matches!(
                db.upsert_runtime_artifact(mutation).await.unwrap_err(),
                CoreError::RuntimeArtifactImmutable
            ));

            // Same invariant for an UNPROMOTED artifact that a Runtime
            // references. Create a real Runtime and repoint it, rather than
            // fabricating a row: `agent_runtimes.runtime_artifact_id` is a
            // foreign key, and the invariant is about the reference existing,
            // not about how it got there.
            // The agent leases the most recently promoted artifact, which is
            // `artifact-immutable`, so the Runtime references it directly.
            complete_self_serve_agent(
                &db,
                "immutable@finite.vip",
                "workos_immutable",
                "immutable-key",
                "immutable-machine",
                "artifact-immutable",
                NOW,
            )
            .await;
            db.exec(
                "UPDATE runtime_artifacts SET promoted_at = NULL WHERE id = 'artifact-immutable'",
            )
            .await;

            let mut referenced_mutation = input;
            referenced_mutation.version_label = "mutated".to_string();
            assert!(matches!(
                db.upsert_runtime_artifact(referenced_mutation)
                    .await
                    .unwrap_err(),
                CoreError::RuntimeArtifactImmutable
            ));
        })
        .await;
    }

    #[tokio::test]
    async fn self_serve_agent_creation_requires_promoted_runtime_artifact() {
        with_isolated_postgres(|db| async move {
            // The shared harness seeds one promoted artifact so creation tests
            // can lease. This test is about having NO launchable artifact.
            db.exec("UPDATE runtime_artifacts SET promoted_at = NULL")
                .await;
            let launch_code = issue_test_launch_code(&db).await;
            db.upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: "artifact-v1".to_string(),
                kind: RuntimeArtifactKind::OciImage,
                reference: "ghcr.io/finitecomputer/finite-agent-runtime:v1".to_string(),
                version_label: "v1".to_string(),
                source_git_sha: None,
                finitec_version: None,
                hermes_source_ref: None,
                finite_platform_plugin_ref: None,
                state_schema_version: "db-v1".to_string(),
                base_image: Some("python:3.11-trixie".to_string()),
                recover_known_good_chat: false,
                promoted: false,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
            let requested = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    display_name: "Oslo Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "first-submit".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            let error = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    source_host_id: None,
                    lease_token: "lease-token-1".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap_err();

            assert!(matches!(error, CoreError::RuntimeArtifactUnavailable));
            assert!(db.all_agent_runtimes().await.is_empty());
            assert_eq!(
                db.agent_creation_request(&requested.request.id)
                    .await
                    .unwrap()
                    .status,
                AgentCreationRequestStatus::Requested
            );
        })
        .await;
    }

    #[tokio::test]
    async fn self_serve_registration_launches_then_completion_marks_running() {
        with_isolated_postgres(|db| async move {
            let launch_code = issue_test_launch_code(&db).await;
            promote_runtime_artifact(&db).await;
            let requested = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    display_name: "Oslo Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "first-submit".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            let lease = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    source_host_id: None,
                    lease_token: "lease-token-1".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap()
                .unwrap();
            let register_input = RegisterAgentCreationRuntimeInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "oslo-agent-001".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
                provider_runtime_handle: Some(ProviderRuntimeHandleEnvelope::V1(
                    ProviderRuntimeHandleV1 {
                        runner_class: RunnerClass::Kata,
                        opaque: json!({"container": "finite-kata-oslo-001"}),
                    },
                )),
                contact_endpoint: Some("https://oslo-agent.example.com/contact/".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: None,
                hostname: None,
                runtime_host: Some("oslo-host-1".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Unknown),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: None,
                published_app_urls: Vec::new(),
                now: Some("2026-05-25T13:01:30Z".to_string()),
            };
            let registered = db
                .register_agent_creation_runtime(register_input)
                .await
                .unwrap();

            assert_eq!(
                registered.request.status,
                AgentCreationRequestStatus::Launching
            );
            assert!(registered.request.agent_runtime_id.is_some());
            let runtime = &db
                .agent_runtime(registered.request.agent_runtime_id.as_ref().unwrap())
                .await
                .unwrap();
            assert_eq!(
                runtime.contact_endpoint.as_deref(),
                Some("https://oslo-agent.example.com/contact")
            );
            assert_eq!(runtime.provider_runtime_handle_history.len(), 1);

            let completion_input = CompleteAgentCreationRequestInput {
                request_id: lease.request.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "oslo-agent-001".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
                provider_runtime_handle: Some(ProviderRuntimeHandleEnvelope::V1(
                    ProviderRuntimeHandleV1 {
                        runner_class: RunnerClass::Kata,
                        opaque: json!({"container": "finite-kata-oslo-001"}),
                    },
                )),
                contact_endpoint: Some("https://oslo-agent.example.com/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: None,
                hostname: None,
                runtime_host: Some("oslo-host-1".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                now: Some("2026-05-25T13:02:00Z".to_string()),
            };
            let mut mismatched_completion = completion_input.clone();
            mismatched_completion.runtime_capabilities =
                Some(RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                    runtime_upgrade: false,
                    ..*kata_runtime_capabilities().v1()
                }));
            assert!(matches!(
                db.complete_agent_creation_request(mismatched_completion)
                    .await,
                Err(CoreError::RuntimeCapabilitiesMismatch)
            ));
            let completed = db
                .complete_agent_creation_request(completion_input)
                .await
                .unwrap();

            assert_eq!(
                completed.request.status,
                AgentCreationRequestStatus::Running
            );
            assert_eq!(completed.project.id, requested.project.id);
        })
        .await;
    }

    #[tokio::test]
    async fn user_can_request_and_runner_can_complete_oci_runtime_restart() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_id = complete_self_serve_agent(
                &db,
                "new@finite.vip",
                "user_workos_new",
                "first-submit",
                "oslo-agent-001",
                "artifact-v1",
                "2026-05-25T13:02:00Z",
            )
            .await;
            let project_id = db
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();

            let restart = db
                .request_runtime_restart(RequestRuntimeRestartInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    project_id,
                    now: Some("2026-05-25T13:03:00Z".to_string()),
                })
                .await
                .unwrap();

            assert_eq!(restart.agent_runtime_id, runtime_id);
            assert_eq!(restart.source_host_id, "oslo-host-1");
            assert_eq!(restart.source_machine_id, "oslo-agent-001");
            assert_eq!(restart.kind, RuntimeControlKind::Restart);
            assert_eq!(restart.status, RuntimeControlRequestStatus::Requested);

            let duplicate = db
                .request_runtime_restart(RequestRuntimeRestartInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    project_id: restart.project_id.clone(),
                    now: Some("2026-05-25T13:04:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(duplicate.id, restart.id);

            let lease = db
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "restart-lease-1".to_string(),
                    lease_seconds: Some(60),
                    source_host_id: Some("oslo-host-1".to_string()),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: Some("2026-05-25T13:04:00Z".to_string()),
                })
                .await
                .unwrap()
                .expect("restart request should lease");

            assert_eq!(lease.request.id, restart.id);
            assert_eq!(lease.request.status, RuntimeControlRequestStatus::Launching);
            assert_eq!(lease.runtime.source_machine_id, "oslo-agent-001");

            let stale_complete = db
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: restart.id.clone(),
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "wrong-token".to_string(),
                    runtime_artifact_id: None,
                    state_schema_version: None,
                    runtime_capabilities: None,
                    runtime_host: None,
                    published_app_urls: None,
                    retirement_snapshot: None,
                    now: Some("2026-05-25T13:04:30Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(
                stale_complete,
                CoreError::RuntimeControlRequestLeaseConflict
            ));

            let forbidden_refresh = db
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: restart.id.clone(),
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "restart-lease-1".to_string(),
                    runtime_artifact_id: None,
                    state_schema_version: None,
                    runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                        RuntimeCapabilitiesV1 {
                            recover_known_good_chat: true,
                            ..*kata_runtime_capabilities().v1()
                        },
                    )),
                    runtime_host: None,
                    published_app_urls: None,
                    retirement_snapshot: None,
                    now: Some("2026-05-25T13:04:45Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(
                forbidden_refresh,
                CoreError::RuntimeUpgradeCompletionMismatch
            ));

            let completed = db
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: restart.id,
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "restart-lease-1".to_string(),
                    runtime_artifact_id: None,
                    state_schema_version: None,
                    runtime_capabilities: None,
                    runtime_host: None,
                    published_app_urls: None,
                    retirement_snapshot: None,
                    now: Some("2026-05-25T13:05:00Z".to_string()),
                })
                .await
                .unwrap();

            assert_eq!(completed.status, RuntimeControlRequestStatus::Succeeded);
            assert!(completed.lease_token.is_none());
            assert_eq!(
                db.agent_runtime(&runtime_id)
                    .await
                    .unwrap()
                    .host_facts
                    .runtime_status,
                RuntimeSummaryStatus::Online
            );
            assert!(
                db.lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "restart-lease-2".to_string(),
                    lease_seconds: Some(60),
                    source_host_id: Some("oslo-host-1".to_string()),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: Some("2026-05-25T13:06:00Z".to_string()),
                })
                .await
                .unwrap()
                .is_none()
            );
        })
        .await;
    }

    #[tokio::test]
    async fn restart_readiness_failure_is_named_and_never_reads_succeeded() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_id = complete_self_serve_agent(
                &db,
                "new@finite.vip",
                "user_workos_new",
                "first-submit",
                "oslo-agent-001",
                "artifact-v1",
                "2026-05-25T13:02:00Z",
            )
            .await;
            let project_id = db
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();

            // The 2026-08-18 postmortem shape: a restart whose readiness wait
            // expires must end in a named failed state, never in succeeded.
            let restart = db
                .request_runtime_restart(RequestRuntimeRestartInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    project_id: project_id.clone(),
                    now: Some("2026-05-25T13:03:00Z".to_string()),
                })
                .await
                .unwrap();
            db.lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "restart-lease-1".to_string(),
                lease_seconds: Some(60),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .await
            .unwrap()
            .expect("restart request should lease");
            let failed = db
                .fail_runtime_control_request(FailRuntimeControlRequestInput {
                    request_id: restart.id,
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "restart-lease-1".to_string(),
                    failure_message: "runtime /healthz did not become ready within 180s"
                        .to_string(),
                    failure_stage: Some(RuntimeLifecycleStage::Readiness),
                    now: Some("2026-05-25T13:07:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(failed.status, RuntimeControlRequestStatus::Failed);
            assert_eq!(failed.failure_stage, Some(RuntimeLifecycleStage::Readiness));
            assert!(failed.completed_at.is_some());
            assert_eq!(
                db.agent_runtime(&runtime_id)
                    .await
                    .unwrap()
                    .host_facts
                    .runtime_status,
                RuntimeSummaryStatus::Stale
            );
            // The terminal row leaves the one-active index: a fresh request
            // is a new row, and the failed one cannot be leased again.
            let retry_restart = db
                .request_runtime_restart(RequestRuntimeRestartInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    project_id,
                    now: Some("2026-05-25T13:08:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_ne!(retry_restart.id, failed.id);

            // An N-1 Runner names no stage; the failure still lands, marked
            // unknown rather than silently laundered into a real stage.
            db.lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "restart-lease-2".to_string(),
                lease_seconds: Some(60),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some("2026-05-25T13:08:30Z".to_string()),
            })
            .await
            .unwrap()
            .expect("the fresh restart should lease");
            let legacy_failed = db
                .fail_runtime_control_request(FailRuntimeControlRequestInput {
                    request_id: retry_restart.id,
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "restart-lease-2".to_string(),
                    failure_message: "n-1 runner failure".to_string(),
                    failure_stage: None,
                    now: Some("2026-05-25T13:09:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(legacy_failed.status, RuntimeControlRequestStatus::Failed);
            assert_eq!(
                legacy_failed.failure_stage,
                Some(RuntimeLifecycleStage::Unknown)
            );
        })
        .await;
    }

    #[tokio::test]
    async fn stop_confirms_into_the_stopped_terminal() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_id = complete_self_serve_agent(
                &db,
                "new@finite.vip",
                "user_workos_new",
                "first-submit",
                "oslo-agent-001",
                "artifact-v1",
                "2026-05-25T13:02:00Z",
            )
            .await;
            let project_id = db
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();

            let stop = db
                .request_runtime_stop(RequestRuntimeStopInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    project_id,
                    now: Some("2026-05-25T13:03:00Z".to_string()),
                })
                .await
                .unwrap();
            db.lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "stop-lease-1".to_string(),
                lease_seconds: Some(60),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .await
            .unwrap()
            .expect("stop request should lease");
            let stopped = db
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: stop.id.clone(),
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "stop-lease-1".to_string(),
                    runtime_artifact_id: None,
                    state_schema_version: None,
                    runtime_capabilities: None,
                    runtime_host: None,
                    published_app_urls: None,
                    retirement_snapshot: None,
                    now: Some("2026-05-25T13:05:00Z".to_string()),
                })
                .await
                .unwrap();
            // A stopped runtime never displays as ready/succeeded: its
            // terminal is Stopped and its host facts read offline.
            assert_eq!(stopped.status, RuntimeControlRequestStatus::Stopped);
            assert_eq!(stopped.failure_stage, None);
            assert_eq!(
                db.agent_runtime(&runtime_id)
                    .await
                    .unwrap()
                    .host_facts
                    .runtime_status,
                RuntimeSummaryStatus::Offline
            );
            // A replayed completion against the terminal row is refused.
            let replay = db
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: stop.id,
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "stop-lease-1".to_string(),
                    runtime_artifact_id: None,
                    state_schema_version: None,
                    runtime_capabilities: None,
                    runtime_host: None,
                    published_app_urls: None,
                    retirement_snapshot: None,
                    now: Some("2026-05-25T13:06:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(
                replay,
                CoreError::RuntimeRetirementSnapshotConflict
            ));
        })
        .await;
    }

    #[tokio::test]
    async fn known_good_chat_recovery_is_fail_closed_until_a_real_recovery_path_exists() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_id = complete_self_serve_agent(
                &db,
                "new@finite.vip",
                "user_workos_new",
                "first-submit",
                "oslo-agent-001",
                "artifact-v1",
                "2026-05-25T13:02:00Z",
            )
            .await;
            let project_id = db
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();

            let error = db
                .request_runtime_recover_known_good_chat(RequestRuntimeRecoverKnownGoodChatInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    project_id,
                    now: Some("2026-05-25T13:03:00Z".to_string()),
                })
                .await
                .unwrap_err();

            assert!(matches!(error, CoreError::RuntimeControlUnsupported));
            assert!(db.all_runtime_control_requests().await.is_empty());
        })
        .await;
    }

    #[tokio::test]
    async fn stop_is_supported_but_runtime_retirement_is_fail_closed() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_id = complete_self_serve_agent(
                &db,
                "new@finite.vip",
                "user_workos_new",
                "first-submit",
                "oslo-agent-001",
                "artifact-v1",
                "2026-05-25T13:02:00Z",
            )
            .await;
            let project_id = db
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();
            let unrelated_runtime_id = complete_self_serve_agent(
                &db,
                "new@finite.vip",
                "user_workos_new",
                "second-submit",
                "oslo-agent-002",
                "artifact-v1",
                "2026-05-25T13:02:10Z",
            )
            .await;
            let unrelated_project_id = db
                .agent_runtime(&unrelated_runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();
            let user_id = db
                .all_users()
                .await
                .iter()
                .find(|user| user.workos_user_id.as_deref() == Some("user_workos_new"))
                .unwrap()
                .id
                .clone();
            assert_eq!(db.visible_projects_for_user(&user_id).await.len(), 2);
            // A legacy relay credential row: nothing writes these anymore, but
            // destroy still clears any left behind by earlier Core generations.
            let relay_hash = "ab".repeat(32);
            db.exec(&format!(
                "INSERT INTO runtime_relay_credentials \
                 (agent_runtime_id, token_hash, created_at, updated_at) \
                 VALUES ('{runtime_id}', '{relay_hash}', \
                 '2026-05-25T13:02:15Z', '2026-05-25T13:02:15Z')"
            ))
            .await;
            assert!(
                !db.query_json(
                    "SELECT to_jsonb(t) FROM runtime_relay_credentials t \
                 WHERE t.agent_runtime_id = $1",
                    &[&runtime_id],
                )
                .await
                .is_empty()
            );
            let grant = db
                .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: Some("user_workos_new".to_string()),
                    limit_profile_id: None,
                    now: Some("2026-05-25T13:02:30Z".to_string()),
                })
                .await
                .unwrap();
            let runtime_key = db
                .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                    grant_id: grant.id,
                    raw_key: "fpk_live_destroy_test".to_string(),
                    project_id: Some(project_id.clone()),
                    agent_runtime_id: Some(runtime_id.clone()),
                    now: Some("2026-05-25T13:02:31Z".to_string()),
                })
                .await
                .unwrap();
            db.exec(&format!(
                "UPDATE agent_runtimes SET host_facts = jsonb_set(host_facts, \
                 '{{published_app_urls}}', \
                 '[\"https://oslo-agent.example.com/contact\"]'::jsonb) \
                 WHERE id = '{runtime_id}'"
            ))
            .await;

            let stop = db
                .request_runtime_stop(RequestRuntimeStopInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    project_id: project_id.clone(),
                    now: Some("2026-05-25T13:03:00Z".to_string()),
                })
                .await
                .unwrap();
            let stop_lease = db
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "stop-lease-1".to_string(),
                    lease_seconds: Some(60),
                    source_host_id: Some("oslo-host-1".to_string()),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: Some("2026-05-25T13:04:00Z".to_string()),
                })
                .await
                .unwrap()
                .expect("stop request should lease");
            assert_eq!(stop_lease.request.kind, RuntimeControlKind::Stop);
            db.complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: stop.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "stop-lease-1".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: None,
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: None,
                now: Some("2026-05-25T13:05:00Z".to_string()),
            })
            .await
            .unwrap();
            let stopped_runtime = &db.agent_runtime(&runtime_id).await.unwrap();
            assert_eq!(
                stopped_runtime.host_facts.runtime_status,
                RuntimeSummaryStatus::Offline
            );
            assert_eq!(
                stopped_runtime.host_facts.published_app_urls,
                vec!["https://oslo-agent.example.com/contact".to_string()]
            );

            let destroy_error = db
                .request_runtime_destroy(RequestRuntimeDestroyInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    project_id: project_id.clone(),
                    now: Some("2026-05-25T13:06:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(
                destroy_error,
                CoreError::RuntimeControlUnsupported
            ));

            // A stale N-1 request cannot bypass the persisted-runtime and worker
            // capability intersection at lease time.
            let stale_destroy_id = "runtime_ctl_stale_destroy".to_string();
            db.exec(&format!(
                "INSERT INTO runtime_control_requests \
                 (id, project_id, agent_runtime_id, source_host_id, source_machine_id, \
                  requested_by_user_id, kind, status, created_at, updated_at) \
                 VALUES ('{stale_destroy_id}', '{project_id}', '{runtime_id}', \
                 'oslo-host-1', 'oslo-agent-001', '{user_id}', 'destroy', 'requested', \
                 '2026-05-25T13:06:30Z', '2026-05-25T13:06:30Z')"
            ))
            .await;
            let stale_destroy_lease = db
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "destroy-lease-1".to_string(),
                    lease_seconds: Some(60),
                    source_host_id: Some("oslo-host-1".to_string()),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: Some("2026-05-25T13:07:00Z".to_string()),
                })
                .await
                .unwrap();
            assert!(stale_destroy_lease.is_none());
            assert_eq!(
                db.runtime_control_request(&stale_destroy_id)
                    .await
                    .unwrap()
                    .status,
                RuntimeControlRequestStatus::Requested
            );
            assert!(
                !db.query_json(
                    "SELECT to_jsonb(t) FROM runtime_relay_credentials t \
                 WHERE t.agent_runtime_id = $1",
                    &[&runtime_id],
                )
                .await
                .is_empty()
            );
            assert!(
                db.all("project_runtime_links")
                    .await
                    .iter()
                    .any(|link| link["agent_runtime_id"] == runtime_id.as_str()
                        && link["active"] == true)
            );
            assert_eq!(
                db.finite_private_api_key(&runtime_key.id)
                    .await
                    .unwrap()
                    .status,
                FinitePrivateApiKeyStatus::Active
            );
            assert!(
                !db.finite_private_admin_audit_events()
                    .await
                    .unwrap()
                    .iter()
                    .any(|event| event.action == "finite_private.runtime.destroy_revoke_keys")
            );
            let visible_project_ids = db
                .visible_projects_for_user(&user_id)
                .await
                .into_iter()
                .map(|visible| visible.project.id)
                .collect::<BTreeSet<_>>();
            assert_eq!(
                visible_project_ids,
                BTreeSet::from([project_id.clone(), unrelated_project_id.clone()]),
                "unsupported retirement cannot hide either project"
            );
            assert!(
                db.project(&project_id).await.is_some(),
                "destroy retains the project row"
            );
            assert!(
                db.agent_runtime(&runtime_id).await.is_some(),
                "destroy retains the runtime row"
            );
            assert!(
                db.all("project_room_memberships")
                    .await
                    .iter()
                    .find(|membership| membership["project_id"] == project_id.as_str())
                    .unwrap()["archived_at"]
                    .is_null()
            );
            assert!(
                db.all("project_room_memberships")
                    .await
                    .iter()
                    .find(|membership| membership["project_id"] == unrelated_project_id.as_str())
                    .unwrap()["archived_at"]
                    .is_null(),
                "unrelated membership remains active"
            );
            assert!(db.agent_runtime(&unrelated_runtime_id).await.is_some());
        })
        .await;
    }

    #[tokio::test]
    async fn retirement_requires_exact_immutable_receipt_and_retries_same_request() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_id = complete_self_serve_agent(
                &db,
                "new@finite.vip",
                "user_workos_new",
                "retirement-submit",
                "oslo-agent-retire",
                "artifact-v1",
                "2026-05-25T13:02:00Z",
            )
            .await;
            let project_id = db
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();
            let retirement_capable =
                serde_json::to_string(&RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                    runtime_retirement: true,
                    ..*kata_runtime_capabilities().v1()
                }))
                .unwrap();
            db.exec(&format!(
                "UPDATE agent_runtimes SET runtime_capabilities = '{retirement_capable}'::jsonb \
                 WHERE id = '{runtime_id}'"
            ))
            .await;
            let request = db
                .request_runtime_destroy(RequestRuntimeDestroyInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    project_id: project_id.clone(),
                    now: Some("2026-05-25T13:03:00Z".to_string()),
                })
                .await
                .unwrap();
            let capacity = RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Kata],
                runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                    RuntimeCapabilitiesV1 {
                        runtime_retirement: true,
                        ..*kata_runtime_capabilities().v1()
                    },
                )),
                ..RunnerLeaseCapacity::default()
            };
            let first = db
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "destroy-lease-1".to_string(),
                    lease_seconds: Some(60),
                    source_host_id: Some("oslo-host-1".to_string()),
                    runner_capacity: Some(capacity.clone()),
                    now: Some("2026-05-25T13:04:00Z".to_string()),
                })
                .await
                .unwrap()
                .unwrap();
            let spec = runtime_spec_v1(first.runtime_spec.as_ref().unwrap());
            let receipt = RuntimeRetirementSnapshotReceipt {
                schema: RUNTIME_RETIREMENT_SNAPSHOT_SCHEMA.to_string(),
                request_id: request.id.clone(),
                project_id: project_id.clone(),
                agent_runtime_id: runtime_id.clone(),
                durable_state_id: spec.durable_state_id.clone(),
                runtime_artifact_id: spec.runtime_artifact_id.clone(),
                backend: RUNTIME_RETIREMENT_BACKEND_BORG.to_string(),
                locator: runtime_retirement_archive_locator(&request.id),
                zip_bytes: 4096,
                zip_sha256: "a".repeat(64),
                manifest_sha256: "b".repeat(64),
                created_at: "2026-05-25T13:04:10Z".to_string(),
                verified_at: "2026-05-25T13:04:20Z".to_string(),
                recovery_authority_id: "finite-assisted-v1".to_string(),
                retention_policy: RUNTIME_RETIREMENT_RETENTION_INDEFINITE.to_string(),
            };

            let bare = db
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: request.id.clone(),
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "destroy-lease-1".to_string(),
                    runtime_artifact_id: None,
                    state_schema_version: None,
                    runtime_capabilities: None,
                    runtime_host: None,
                    published_app_urls: None,
                    retirement_snapshot: None,
                    now: Some("2026-05-25T13:04:25Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(bare, CoreError::RuntimeRetirementSnapshotMismatch));
            assert!(db.all("runtime_retirement_snapshots").await.is_empty());
            assert!(db.active_runtime_for_project(&project_id).await.is_some());

            db.renew_runtime_control_request(RenewRuntimeControlRequestInput {
                request_id: request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "destroy-lease-1".to_string(),
                lease_seconds: Some(60),
                now: Some("2026-05-25T13:04:30Z".to_string()),
            })
            .await
            .unwrap();
            let retry = db
                .retry_runtime_control_request(RetryRuntimeControlRequestInput {
                    request_id: request.id.clone(),
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "destroy-lease-1".to_string(),
                    failure_message: "synthetic upload interruption".to_string(),
                    now: Some("2026-05-25T13:04:40Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(retry.id, request.id);
            assert_eq!(retry.status, RuntimeControlRequestStatus::Requested);
            let second = db
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "destroy-lease-2".to_string(),
                    lease_seconds: Some(60),
                    source_host_id: Some("oslo-host-1".to_string()),
                    runner_capacity: Some(capacity),
                    now: Some("2026-05-25T13:04:45Z".to_string()),
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(second.request.id, request.id);

            let completion = CompleteRuntimeControlRequestInput {
                request_id: request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "destroy-lease-2".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: None,
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: Some(receipt.clone()),
                now: Some("2026-05-25T13:05:00Z".to_string()),
            };
            let completed = db
                .complete_runtime_control_request(completion.clone())
                .await
                .unwrap();
            // Retirement confirms into the Stopped terminal, never Succeeded:
            // a stopped runtime must not read as a ready one.
            assert_eq!(completed.status, RuntimeControlRequestStatus::Stopped);
            let snapshot = db
                .row("runtime_retirement_snapshots", &request.id)
                .await
                .expect("a completed retirement persists its snapshot");
            assert_eq!(snapshot["zip_sha256"], receipt.zip_sha256);
            assert_eq!(snapshot["manifest_sha256"], receipt.manifest_sha256);
            assert_eq!(snapshot["locator"], receipt.locator);
            assert_eq!(snapshot["backend"], receipt.backend);
            assert!(db.active_runtime_for_project(&project_id).await.is_none());
            assert_eq!(
                db.visible_projects_for_user(&completed.requested_by_user_id)
                    .await
                    .len(),
                0
            );

            let replay = db
                .complete_runtime_control_request(completion)
                .await
                .expect("identical completion replay is idempotent");
            assert_eq!(replay.id, request.id);
            let mut conflicting = receipt;
            conflicting.zip_sha256 = "c".repeat(64);
            let conflict = db
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: request.id.clone(),
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "destroy-lease-2".to_string(),
                    runtime_artifact_id: None,
                    state_schema_version: None,
                    runtime_capabilities: None,
                    runtime_host: None,
                    published_app_urls: None,
                    retirement_snapshot: Some(conflicting),
                    now: Some("2026-05-25T13:05:01Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(
                conflict,
                CoreError::RuntimeRetirementSnapshotConflict
            ));
            assert_eq!(
                db.row("runtime_retirement_snapshots", &request.id)
                    .await
                    .unwrap()["zip_sha256"],
                "a".repeat(64),
                "a conflicting replay must not replace the immutable receipt"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn oci_runtime_artifacts_support_hosted_runtime_control() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_id = complete_self_serve_agent(
                &db,
                "new@finite.vip",
                "user_workos_new",
                "first-submit",
                "docker-agent-001",
                "artifact-v1",
                "2026-05-25T13:02:00Z",
            )
            .await;
            let project_id = db
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();

            let restart = db
                .request_runtime_restart(RequestRuntimeRestartInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    project_id,
                    now: Some("2026-05-25T13:03:00Z".to_string()),
                })
                .await
                .unwrap();

            assert_eq!(restart.agent_runtime_id, runtime_id);
            assert_eq!(restart.kind, RuntimeControlKind::Restart);
        })
        .await;
    }

    #[tokio::test]
    async fn runner_lease_can_expire_and_reassign_but_completion_requires_current_token() {
        with_isolated_postgres(|db| async move {
            let launch_code = issue_test_launch_code(&db).await;
            promote_runtime_artifact(&db).await;
            let requested = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    display_name: "Oslo Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "first-submit".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            let first_lease = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-a".to_string(),
                    source_host_id: None,
                    lease_token: "lease-a".to_string(),
                    lease_seconds: Some(60),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(first_lease.request.project_id, requested.project.id);
            let second_lease = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-b".to_string(),
                    source_host_id: None,
                    lease_token: "lease-b".to_string(),
                    lease_seconds: Some(60),
                    runner_capacity: None,
                    now: Some("2026-05-25T13:02:00Z".to_string()),
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(second_lease.request.runner_id.as_deref(), Some("runner-b"));

            let stale_complete = db
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: requested.request.id,
                    runner_id: "runner-a".to_string(),
                    lease_token: "lease-a".to_string(),
                    source_host_id: "oslo-host-1".to_string(),
                    source_machine_id: "oslo-agent-001".to_string(),
                    runtime_artifact_id: Some("artifact-v1".to_string()),
                    state_schema_version: None,
                    provider_runtime_handle: None,
                    contact_endpoint: None,
                    runtime_capabilities: None,
                    display_name: None,
                    hostname: None,
                    runtime_host: None,
                    runtime_status: None,
                    active_inference_profile: None,
                    hermes_available: None,
                    published_app_urls: Vec::new(),
                    now: Some("2026-05-25T13:03:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(
                stale_complete,
                CoreError::AgentCreationRequestLeaseConflict
            ));
        })
        .await;
    }

    #[tokio::test]
    async fn runner_can_mark_agent_creation_request_failed_without_runtime() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let launch_code = issue_test_launch_code(&db).await;
            let requested = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    display_name: "Oslo Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "first-submit".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                source_host_id: None,
                lease_token: "lease-token-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap();

            let failed = db
                .fail_agent_creation_request(FailAgentCreationRequestInput {
                    request_id: requested.request.id,
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "lease-token-1".to_string(),
                    failure_message: "runner capacity unavailable".to_string(),
                    provisioned_finite_private_api_key_id: None,
                    now: Some("2026-05-25T13:02:00Z".to_string()),
                })
                .await
                .unwrap();

            assert_eq!(failed.status, AgentCreationRequestStatus::Failed);
            assert_eq!(
                failed.failure_message.as_deref(),
                Some("runner capacity unavailable")
            );
            assert!(failed.agent_runtime_id.is_none());
            assert!(db.all_agent_runtimes().await.is_empty());
        })
        .await;
    }

    #[tokio::test]
    async fn cancelled_request_does_not_make_a_redeemed_launch_code_reusable() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let launch_code = issue_test_launch_code(&db).await;
            let requested = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    display_name: "Oslo Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "first-submit".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                source_host_id: None,
                lease_token: "lease-token-1".to_string(),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap();
            db.fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id: requested.request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                failure_message: "runner capacity unavailable".to_string(),
                provisioned_finite_private_api_key_id: None,
                now: Some("2026-05-25T13:02:00Z".to_string()),
            })
            .await
            .unwrap();

            let cancelled = db
                .cancel_agent_creation_request(CancelAgentCreationRequestInput {
                    request_id: requested.request.id,
                    now: Some("2026-05-25T13:03:00Z".to_string()),
                })
                .await
                .unwrap();

            assert_eq!(cancelled.status, AgentCreationRequestStatus::Cancelled);
            assert!(cancelled.agent_runtime_id.is_none());
            assert!(
                db.visible_projects_for_user(&requested.project.owner_user_id)
                    .await
                    .is_empty()
            );

            let retry = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    display_name: "Retry Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "second-submit".to_string(),
                    now: Some("2026-05-25T13:04:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(retry, CoreError::InvalidLaunchCode));
        })
        .await;
    }

    #[tokio::test]
    async fn failed_self_serve_launch_removes_provisional_runtime() {
        with_isolated_postgres(|db| async move {
            let launch_code = issue_test_launch_code(&db).await;
            promote_runtime_artifact(&db).await;
            let requested = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    display_name: "Oslo Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "first-submit".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            let lease = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    source_host_id: None,
                    lease_token: "lease-token-1".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap()
                .unwrap();
            db.register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "lease-token-1".to_string(),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: "oslo-agent-001".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: None,
                provider_runtime_handle: None,
                contact_endpoint: None,
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: None,
                hostname: None,
                runtime_host: Some("oslo-host-1".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Unknown),
                active_inference_profile: None,
                hermes_available: None,
                published_app_urls: Vec::new(),
                now: Some("2026-05-25T13:01:30Z".to_string()),
            })
            .await
            .unwrap();

            assert_eq!(db.table_len("agent_runtimes").await, 1);
            assert_eq!(db.table_len("project_runtime_links").await, 1);

            let failed = db
                .fail_agent_creation_request(FailAgentCreationRequestInput {
                    request_id: requested.request.id,
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "lease-token-1".to_string(),
                    failure_message: "runtime did not publish a relay heartbeat".to_string(),
                    provisioned_finite_private_api_key_id: None,
                    now: Some("2026-05-25T13:03:00Z".to_string()),
                })
                .await
                .unwrap();

            assert_eq!(failed.status, AgentCreationRequestStatus::Failed);
            assert!(failed.agent_runtime_id.is_none());
            assert!(db.all_agent_runtimes().await.is_empty());
            assert!(db.all("project_runtime_links").await.is_empty());
        })
        .await;
    }

    #[tokio::test]
    async fn fresh_launch_code_adds_one_creation_to_an_exhausted_entitlement() {
        with_isolated_postgres(|db| async move {
            let launch_code = issue_test_launch_code(&db).await;

            let bad = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    display_name: "Oslo Agent".to_string(),
                    launch_code: "wrong".to_string(),
                    idempotency_key: "bad-submit".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(bad, CoreError::InvalidLaunchCode));
            assert!(db.all_users().await.is_empty());
            assert!(db.all_customer_orgs().await.is_empty());
            assert!(db.all("agent_creation_entitlements").await.is_empty());

            db.request_agent_creation(RequestAgentCreationInput {
                verified_email: "new@finite.vip".to_string(),
                workos_user_id: "user_workos_new".to_string(),
                display_name: "Oslo Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: "first-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
            let unused_launch_code = issue_test_launch_code(&db).await;
            let unused_launch_code_id = issued_launch_code_id(&db, &unused_launch_code).await;
            let second = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    display_name: "Second Agent".to_string(),
                    launch_code: unused_launch_code.clone(),
                    idempotency_key: "second-submit".to_string(),
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap();
            assert!(!second.reused);
            assert!(
                !db.row("launch_codes", &unused_launch_code_id)
                    .await
                    .unwrap()["redeemed_at"]
                    .is_null(),
                "the top-up code must be consumed"
            );
            let entitlement = db
                .all("agent_creation_entitlements")
                .await
                .iter()
                .find(|entitlement| {
                    entitlement["customer_org_id"] == second.project.customer_org_id.as_str()
                })
                .unwrap()
                .clone();
            assert_eq!(entitlement["allowed_new_agent_runtimes"], 2);
            let entitlement_org_id = entitlement["customer_org_id"].as_str().unwrap().to_string();

            let retry = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    display_name: "Second Agent".to_string(),
                    launch_code: unused_launch_code,
                    idempotency_key: "second-submit".to_string(),
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap();
            assert!(retry.reused);
            assert_eq!(
                db.agent_creation_entitlement(&entitlement_org_id)
                    .await
                    .unwrap()
                    .allowed_new_agent_runtimes,
                2,
                "an identical retry must not apply the top-up twice"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn paid_self_serve_agent_creation_requires_active_stripe_billing() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;

            let unpaid = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "paid@finite.vip".to_string(),
                    workos_user_id: "user_workos_paid".to_string(),
                    display_name: "Paid Agent".to_string(),
                    launch_code: String::new(),
                    idempotency_key: "paid-submit-before-billing".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(unpaid, CoreError::BillingRequired));
            assert!(db.all_users().await.is_empty());
            assert!(db.all_customer_orgs().await.is_empty());

            db.link_stripe_customer(LinkStripeCustomerInput {
                verified_email: "paid@finite.vip".to_string(),
                workos_user_id: "user_workos_paid".to_string(),
                stripe_customer_id: "cus_paid".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
            let org_id = db
                .personal_org_by_owner(&db.user_by_email("paid@finite.vip").await.unwrap().id)
                .await
                .unwrap()
                .id;
            db.sync_stripe_subscription(SyncStripeSubscriptionInput {
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
            .await
            .unwrap();

            let overview = db
                .billing_overview(LinkVerifiedUserInput {
                    verified_email: "paid@finite.vip".to_string(),
                    workos_user_id: "user_workos_paid".to_string(),
                    now: Some(LATER.to_string()),
                })
                .await
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

            let created = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "paid@finite.vip".to_string(),
                    workos_user_id: "user_workos_paid".to_string(),
                    display_name: "Paid Agent".to_string(),
                    launch_code: String::new(),
                    idempotency_key: "paid-submit".to_string(),
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap();
            assert_eq!(created.request.requested_launch_code, None);
            assert_eq!(
                db.customer_org(&org_id).await.unwrap().billing_class,
                BillingClass::Standard
            );
            let lease = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-paid-1".to_string(),
                    source_host_id: None,
                    lease_token: "paid-lease-1".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: Some("2026-05-25T13:01:00Z".to_string()),
                })
                .await
                .unwrap()
                .expect("paid request should be leased");
            let provisioned = db
                .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                    request_id: lease.request.id.clone(),
                    runner_id: "runner-paid-1".to_string(),
                    lease_token: "paid-lease-1".to_string(),
                    source_host_id: Some("paid-host-1".to_string()),
                    source_machine_id: Some("paid-agent-001".to_string()),
                    now: Some("2026-05-25T13:02:00Z".to_string()),
                })
                .await
                .unwrap();
            let completed = db
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: lease.request.id.clone(),
                    runner_id: "runner-paid-1".to_string(),
                    lease_token: "paid-lease-1".to_string(),
                    source_host_id: "paid-host-1".to_string(),
                    source_machine_id: "paid-agent-001".to_string(),
                    runtime_artifact_id: Some("artifact-v1".to_string()),
                    state_schema_version: None,
                    provider_runtime_handle: None,
                    contact_endpoint: None,
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: None,
                    hostname: None,
                    runtime_host: Some("paid-host-1".to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: vec!["https://paid-agent.example.com/contact".to_string()],
                    now: Some("2026-05-25T13:03:00Z".to_string()),
                })
                .await
                .unwrap();
            let runtime_id = completed.request.agent_runtime_id.unwrap();
            assert!(db.agent_runtime(&runtime_id).await.is_some());

            db.sync_stripe_subscription(SyncStripeSubscriptionInput {
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
            .await
            .unwrap();
            let blocked_after_past_due = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "paid@finite.vip".to_string(),
                    workos_user_id: "user_workos_paid".to_string(),
                    display_name: "Second Paid Agent".to_string(),
                    launch_code: String::new(),
                    idempotency_key: "paid-submit-2".to_string(),
                    now: Some("2026-05-25T14:01:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(blocked_after_past_due, CoreError::BillingRequired));
            assert!(db.agent_runtime(&runtime_id).await.is_some());
            assert!(
                db.all("project_runtime_links")
                    .await
                    .iter()
                    .any(|link| link["agent_runtime_id"] == runtime_id.as_str()
                        && link["active"] == true)
            );
            assert_eq!(
                db.finite_private_api_key(&provisioned.api_key.id)
                    .await
                    .unwrap()
                    .status,
                FinitePrivateApiKeyStatus::Active
            );
        })
        .await;
    }

    #[tokio::test]
    async fn stripe_subscription_sync_ignores_non_current_subscription_events() {
        with_isolated_postgres(|db| async move {
            db.link_stripe_customer(LinkStripeCustomerInput {
                verified_email: "paid@finite.vip".to_string(),
                workos_user_id: "user_workos_paid".to_string(),
                stripe_customer_id: "cus_paid".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
            let org_id = db
                .personal_org_by_owner(&db.user_by_email("paid@finite.vip").await.unwrap().id)
                .await
                .unwrap()
                .id;
            let current = db
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
                .await
                .unwrap();
            assert_eq!(
                current.stripe_subscription_id.as_deref(),
                Some("sub_current")
            );

            let ignored = db
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
                .await
                .unwrap();
            assert_eq!(
                ignored.stripe_subscription_id.as_deref(),
                Some("sub_current")
            );
            assert_eq!(
                db.customer_billing_account(&org_id)
                    .await
                    .unwrap()
                    .last_stripe_event_id
                    .as_deref(),
                Some("evt_current_active")
            );

            db.sync_stripe_subscription(SyncStripeSubscriptionInput {
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
            .await
            .unwrap();

            let replacement = db
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
                .await
                .unwrap();
            assert_eq!(
                replacement.stripe_subscription_id.as_deref(),
                Some("sub_replacement")
            );

            let old_event = db
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
                .await
                .unwrap();
            assert_eq!(
                old_event.stripe_subscription_id.as_deref(),
                Some("sub_replacement")
            );
            assert_eq!(
                db.customer_billing_account(&org_id)
                    .await
                    .unwrap()
                    .subscription_status
                    .unwrap(),
                BillingSubscriptionStatus::Active
            );
        })
        .await;
    }

    #[tokio::test]
    async fn stripe_subscription_sync_ignores_stale_out_of_order_event() {
        with_isolated_postgres(|db| async move {
            // Event-ordering guard: for the SAME subscription, a webhook whose Stripe
            // `event.created` predates the last applied event must be ignored, so a
            // stale `active` delivered after `canceled` cannot resurrect billing.
            db.link_stripe_customer(LinkStripeCustomerInput {
                verified_email: "order@finite.vip".to_string(),
                workos_user_id: "user_workos_order".to_string(),
                stripe_customer_id: "cus_order".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
            let org_id = db
                .personal_org_by_owner(&db.user_by_email("order@finite.vip").await.unwrap().id)
                .await
                .unwrap()
                .id;

            sync_order_subscription(
                &db,
                &org_id,
                BillingSubscriptionStatus::Active,
                "evt_active",
                1_000,
            )
            .await;
            let canceled = sync_order_subscription(
                &db,
                &org_id,
                BillingSubscriptionStatus::Canceled,
                "evt_canceled",
                2_000,
            )
            .await;
            assert_eq!(
                canceled.subscription_status,
                Some(BillingSubscriptionStatus::Canceled)
            );

            // Stale `active` (created BEFORE the canceled event) arrives last.
            let stale = sync_order_subscription(
                &db,
                &org_id,
                BillingSubscriptionStatus::Active,
                "evt_active_stale",
                1_500,
            )
            .await;
            assert_eq!(
                stale.subscription_status,
                Some(BillingSubscriptionStatus::Canceled),
                "stale out-of-order webhook must be ignored; billing stays canceled"
            );
            assert_eq!(stale.last_stripe_event_id.as_deref(), Some("evt_canceled"));
        })
        .await;
    }

    #[tokio::test]
    async fn stripe_subscription_sync_requires_standard_price_before_entitlement() {
        with_isolated_postgres(|db| async move {
            db.link_stripe_customer(LinkStripeCustomerInput {
                verified_email: "paid@finite.vip".to_string(),
                workos_user_id: "user_workos_paid".to_string(),
                stripe_customer_id: "cus_paid".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
            let org_id = db
                .personal_org_by_owner(&db.user_by_email("paid@finite.vip").await.unwrap().id)
                .await
                .unwrap()
                .id;

            let wrong_price = db
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
                .await
                .unwrap_err();
            assert!(matches!(
                wrong_price,
                CoreError::StripeSubscriptionPriceMismatch
            ));
            assert!(db.all("agent_creation_entitlements").await.is_empty());

            let missing_expected_price = db
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
                .await
                .unwrap_err();
            assert!(matches!(
                missing_expected_price,
                CoreError::MissingStripeStandardPriceId
            ));
            assert!(db.all("agent_creation_entitlements").await.is_empty());
        })
        .await;
    }

    #[tokio::test]
    async fn stripe_subscription_lapse_preserves_launch_code_entitlement() {
        with_isolated_postgres(|db| async move {
            let launch_code = issue_test_launch_code(&db).await;
            let launch_code_id = issued_launch_code_id(&db, &launch_code).await;
            db.request_agent_creation(RequestAgentCreationInput {
                verified_email: "bridge@finite.vip".to_string(),
                workos_user_id: "user_workos_bridge".to_string(),
                display_name: "Bridge Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: "bridge-submit".to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
            let org_id = db
                .personal_org_by_owner(&db.user_by_email("bridge@finite.vip").await.unwrap().id)
                .await
                .unwrap()
                .id;
            assert_eq!(
                db.all("agent_creation_entitlements")
                    .await
                    .iter()
                    .find(|entitlement| entitlement["customer_org_id"] == org_id.as_str())
                    .and_then(|entitlement| entitlement["launch_code"].as_str()),
                Some(launch_code_id.as_str())
            );

            db.sync_stripe_subscription(SyncStripeSubscriptionInput {
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
            .await
            .unwrap();
            assert_eq!(
                db.agent_creation_entitlement(&org_id)
                    .await
                    .unwrap()
                    .launch_code
                    .as_deref(),
                Some(launch_code_id.as_str())
            );

            db.sync_stripe_subscription(SyncStripeSubscriptionInput {
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
            .await
            .unwrap();
            let entitlement = &db.agent_creation_entitlement(&org_id).await.unwrap();
            assert_eq!(
                entitlement.launch_code.as_deref(),
                Some(launch_code_id.as_str())
            );
            assert_eq!(entitlement.allowed_new_agent_runtimes, 1);
        })
        .await;
    }

    #[tokio::test]
    async fn finite_private_runtime_key_provisioning_is_bound_to_launching_request() {
        with_isolated_postgres(|db| async move {
            let launch_code = issue_test_launch_code(&db).await;
            promote_runtime_artifact(&db).await;
            let requested = db
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "new@finite.vip".to_string(),
                    workos_user_id: "user_workos_new".to_string(),
                    display_name: "Oslo Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "first-submit".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            let lease = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    source_host_id: None,
                    lease_token: "lease-token-1".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap()
                .expect("request should be leased");

            let provisioned = db
                .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                    request_id: lease.request.id.clone(),
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "lease-token-1".to_string(),
                    source_host_id: Some("oslo-host-1".to_string()),
                    source_machine_id: Some("finite-agent_123".to_string()),
                    now: Some("2026-05-25T13:01:00Z".to_string()),
                })
                .await
                .unwrap();

            assert!(provisioned.raw_api_key.starts_with("fpk_live_"));
            assert_eq!(provisioned.grant.status, FinitePrivateGrantStatus::Active);
            assert_eq!(
                provisioned.api_key.project_id.as_deref(),
                Some(requested.project.id.as_str())
            );
            assert!(provisioned.api_key.agent_runtime_id.is_none());
            assert!(
                !serde_json::to_string(&db.all("finite_private_api_keys").await)
                    .unwrap()
                    .contains(&provisioned.raw_api_key)
            );

            let wrong_lease = db
                .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                    request_id: lease.request.id.clone(),
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "wrong-token".to_string(),
                    source_host_id: Some("oslo-host-1".to_string()),
                    source_machine_id: Some("finite-agent_123".to_string()),
                    now: Some("2026-05-25T13:01:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(
                wrong_lease,
                CoreError::AgentCreationRequestLeaseConflict
            ));

            let unrelated_key = db
                .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                    grant_id: provisioned.grant.id.clone(),
                    raw_key: "fpk_live_unrelated_project_key".to_string(),
                    project_id: None,
                    agent_runtime_id: None,
                    now: Some("2026-05-25T13:01:30Z".to_string()),
                })
                .await
                .unwrap();
            let mismatched = db
                .fail_agent_creation_request(FailAgentCreationRequestInput {
                    request_id: lease.request.id.clone(),
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "lease-token-1".to_string(),
                    failure_message: "runtime failed".to_string(),
                    provisioned_finite_private_api_key_id: Some(unrelated_key.id.clone()),
                    now: Some("2026-05-25T13:02:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(mismatched, CoreError::InvalidFinitePrivateApiKey));
            assert_eq!(
                db.agent_creation_request(&lease.request.id)
                    .await
                    .unwrap()
                    .status,
                AgentCreationRequestStatus::Launching
            );
            assert_eq!(
                db.finite_private_api_key(&unrelated_key.id)
                    .await
                    .unwrap()
                    .status,
                FinitePrivateApiKeyStatus::Active
            );

            let failed = db
                .fail_agent_creation_request(FailAgentCreationRequestInput {
                    request_id: lease.request.id,
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "lease-token-1".to_string(),
                    failure_message: "runtime failed".to_string(),
                    provisioned_finite_private_api_key_id: Some(provisioned.api_key.id.clone()),
                    now: Some("2026-05-25T13:02:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(failed.status, AgentCreationRequestStatus::Failed);
            assert_eq!(
                db.finite_private_api_key(&provisioned.api_key.id)
                    .await
                    .unwrap()
                    .status,
                FinitePrivateApiKeyStatus::Revoked
            );
            assert_eq!(
                db.finite_private_api_key(&unrelated_key.id)
                    .await
                    .unwrap()
                    .status,
                FinitePrivateApiKeyStatus::Active
            );
        })
        .await;
    }

    #[tokio::test]
    async fn finite_private_reserve_and_settle_keeps_core_as_usage_authority() {
        with_isolated_postgres(|db| async move {
            let grant = db
                .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                    verified_email: "private@finite.vip".to_string(),
                    workos_user_id: Some("user_workos_private".to_string()),
                    limit_profile_id: None,
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            let key = db
                .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                    grant_id: grant.id.clone(),
                    raw_key: "fpk_live_secret".to_string(),
                    project_id: None,
                    agent_runtime_id: None,
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            assert_ne!(key.key_hash, "fpk_live_secret");
            assert!(
                !serde_json::to_string(&db.all("finite_private_api_keys").await)
                    .unwrap()
                    .contains("fpk_live_secret")
            );

            let reserved = db
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
                .await
                .unwrap();

            assert_eq!(reserved.decision, "allow");
            assert_eq!(reserved.burst_limit_units, Some(100_000_000));
            assert_eq!(reserved.burst_remaining_units, Some(99_750_000));
            assert_eq!(reserved.weekly_limit_units, None);
            assert_eq!(reserved.weekly_remaining_units, None);
            let reservation_id = reserved.reservation_id.clone().unwrap();
            assert_eq!(
                db.finite_private_grant(&grant.id)
                    .await
                    .unwrap()
                    .current_window_used_units,
                250_000
            );

            let settled = db
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
                .await
                .unwrap();

            assert!(settled.settled);
            assert_eq!(
                db.finite_private_grant(&grant.id)
                    .await
                    .unwrap()
                    .current_window_used_units,
                160_000
            );
            let reservation = &db
                .finite_private_reservation(&reservation_id)
                .await
                .unwrap();
            assert_eq!(reservation.status, FinitePrivateReservationStatus::Settled);
            assert_eq!(
                reservation.settlement_kind,
                Some(FinitePrivateSettlementKind::Actual)
            );
            assert_eq!(reservation.settled_usage_units, Some(160_000));
        })
        .await;
    }

    #[tokio::test]
    async fn finite_private_grant_can_start_as_pending_email_and_later_link_workos() {
        with_isolated_postgres(|db| async move {
            let pending_grant = db
                .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                    verified_email: "friend@finite.vip".to_string(),
                    workos_user_id: None,
                    limit_profile_id: None,
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            let pending_user = db.user(&pending_grant.user_id).await.unwrap();
            assert_eq!(pending_user.email, "friend@finite.vip");
            assert_eq!(pending_user.status, UserLinkStatus::Pending);
            assert_eq!(pending_user.workos_user_id, None);

            let linked_grant = db
                .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                    verified_email: "friend@finite.vip".to_string(),
                    workos_user_id: Some("user_workos_friend".to_string()),
                    limit_profile_id: None,
                    now: Some("2026-05-26T13:00:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(linked_grant.id, pending_grant.id);
            let linked_user = db.user(&linked_grant.user_id).await.unwrap();
            assert_eq!(linked_user.status, UserLinkStatus::Linked);
            assert_eq!(
                linked_user.workos_user_id.as_deref(),
                Some("user_workos_friend")
            );
        })
        .await;
    }

    #[tokio::test]
    async fn finite_private_admin_operations_write_audit_events_without_raw_keys() {
        with_isolated_postgres(|db| async move {
            let grant = db
                .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                    verified_email: "friend@finite.vip".to_string(),
                    workos_user_id: None,
                    limit_profile_id: None,
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            let key = db
                .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                    grant_id: grant.id.clone(),
                    raw_key: "fpk_live_first_secret".to_string(),
                    project_id: None,
                    agent_runtime_id: None,
                    now: Some("2026-05-26T12:01:00Z".to_string()),
                })
                .await
                .unwrap();
            db.reset_finite_private_usage_window(ResetFinitePrivateUsageWindowInput {
                grant_id: grant.id.clone(),
                now: Some("2026-05-26T12:02:00Z".to_string()),
            })
            .await
            .unwrap();
            let rotated = db
                .rotate_finite_private_api_key(RotateFinitePrivateApiKeyInput {
                    key_id: key.id.clone(),
                    raw_key: "fpk_live_second_secret".to_string(),
                    now: Some("2026-05-26T12:03:00Z".to_string()),
                })
                .await
                .unwrap();
            db.revoke_finite_private_grant(RevokeFinitePrivateGrantInput {
                grant_id: grant.id.clone(),
                now: Some("2026-05-26T12:04:00Z".to_string()),
            })
            .await
            .unwrap();

            let events = db.finite_private_admin_audit_events().await.unwrap();
            let actions = events
                .iter()
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
                db.finite_private_admin_audit_events()
                    .await
                    .unwrap()
                    .iter()
                    .filter(|event| event.grant_id.as_deref() == Some(grant.id.as_str()))
                    .count(),
                db.table_len("finite_private_admin_audit_events").await
            );
            assert_eq!(
                db.finite_private_api_key(&rotated.id).await.unwrap().status,
                FinitePrivateApiKeyStatus::Revoked
            );
            let audit_json =
                serde_json::to_string(&db.finite_private_admin_audit_events().await.unwrap())
                    .unwrap();
            assert!(!audit_json.contains("fpk_live_first_secret"));
            assert!(!audit_json.contains("fpk_live_second_secret"));
        })
        .await;
    }

    #[tokio::test]
    async fn finite_private_reserve_denies_unknown_key_and_over_limit_without_upstream_work() {
        with_isolated_postgres(|db| async move {
            let unknown = db
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
                .await
                .unwrap();
            assert_eq!(unknown.decision, "deny");
            assert_eq!(
                unknown.error.as_ref().map(|error| error.code.as_str()),
                Some("invalid_api_key")
            );
            assert!(db.all_finite_private_reservations().await.is_empty());

            let grant = db
                .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                    verified_email: "private@finite.vip".to_string(),
                    workos_user_id: Some("user_workos_private".to_string()),
                    limit_profile_id: None,
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            db.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id.clone(),
                raw_key: "fpk_live_secret".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();

            let denied = db
                .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                    request_id: "req-private-over".to_string(),
                    presented_api_key: "fpk_live_secret".to_string(),
                    endpoint: "/v1/chat/completions".to_string(),
                    model: "kimi-k2-6".to_string(),
                    estimated_prompt_tokens: DEFAULT_FINITE_PRIVATE_BURST_LIMIT_UNITS + 1,
                    estimated_completion_tokens: 0,
                    estimated_usage_units: DEFAULT_FINITE_PRIVATE_BURST_LIMIT_UNITS + 1,
                    usage_formula_version: "2026-05-26.v1".to_string(),
                    dashboard_url: "https://finite.computer/dashboard".to_string(),
                    now: Some("2026-05-25T13:00:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(denied.decision, "deny");
            assert_eq!(
                denied.error.as_ref().map(|error| error.code.as_str()),
                Some("burst_window_limit_exceeded")
            );
            let denied_error = denied.error.as_ref().unwrap();
            assert!(denied_error.message.contains("2026-05-25T18:00:00Z"));
            assert!(denied_error.message.contains("(in 5h)"));
            assert_eq!(
                db.finite_private_grant(&grant.id)
                    .await
                    .unwrap()
                    .current_window_used_units,
                0
            );
            assert!(db.all_finite_private_reservations().await.is_empty());
        })
        .await;
    }

    #[tokio::test]
    async fn finite_private_weekly_limit_denies_without_upstream_work() {
        with_isolated_postgres(|db| async move {
            db.exec(
                "INSERT INTO finite_private_limit_profiles \
                 (id, burst_window_seconds, burst_limit_units, weekly_limit_units, \
                  created_at, updated_at) \
                 VALUES ('weekly-small', 3600, 10000000, 1000, \
                 '2026-05-25T12:00:00Z', '2026-05-25T12:00:00Z')",
            )
            .await;
            let grant = db
                .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                    verified_email: "private@finite.vip".to_string(),
                    workos_user_id: Some("user_workos_private".to_string()),
                    limit_profile_id: Some("weekly-small".to_string()),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            db.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id.clone(),
                raw_key: "fpk_live_secret".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();

            let allowed = db
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
                .await
                .unwrap();
            assert_eq!(allowed.decision, "allow");
            assert_eq!(allowed.weekly_remaining_units, Some(200));

            let denied = db
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
                .await
                .unwrap();
            assert_eq!(denied.decision, "deny");
            assert_eq!(
                denied.error.as_ref().map(|error| error.code.as_str()),
                Some("weekly_limit_exceeded")
            );
            let denied_error = denied.error.as_ref().unwrap();
            assert!(denied_error.message.contains("2026-06-01T13:00:00Z"));
            assert!(denied_error.message.contains("(in 6d)"));
            assert_eq!(db.table_len("finite_private_reservations").await, 1);
        })
        .await;
    }

    #[tokio::test]
    async fn finite_private_status_does_not_start_or_roll_the_usage_window() {
        with_isolated_postgres(|db| async move {
            let grant = db
                .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                    verified_email: "status-read@finite.vip".to_string(),
                    workos_user_id: None,
                    limit_profile_id: None,
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            db.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id.clone(),
                raw_key: "fpk_live_status_read".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();

            let status = db
                .finite_private_usage_status_for_api_key(
                    "fpk_live_status_read",
                    false,
                    Some("2026-05-26T13:00:00Z".to_string()),
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(status.burst_used_units, 0);
            assert_eq!(status.burst_reset_at, "2026-05-26T18:00:00Z");
            let after_unstarted_read = &db.finite_private_grant(&grant.id).await.unwrap();
            assert!(after_unstarted_read.current_window_started_at.is_none());
            assert_eq!(
                after_unstarted_read.burst_window_epoch,
                grant.burst_window_epoch
            );

            db.reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: "req-status-window".to_string(),
                presented_api_key: "fpk_live_status_read".to_string(),
                endpoint: "/v1/chat/completions".to_string(),
                model: "glm-5.2".to_string(),
                estimated_prompt_tokens: 10,
                estimated_completion_tokens: 0,
                estimated_usage_units: 10,
                usage_formula_version: "v1".to_string(),
                dashboard_url: "https://finite.computer/dashboard".to_string(),
                now: Some("2026-05-26T14:00:00Z".to_string()),
            })
            .await
            .unwrap();
            let started = db.finite_private_grant(&grant.id).await.unwrap().clone();
            assert_eq!(
                started.current_window_started_at.as_deref(),
                Some("2026-05-26T14:00:00Z")
            );

            let expired_status = db
                .finite_private_usage_status_for_api_key(
                    "fpk_live_status_read",
                    false,
                    Some("2026-05-26T20:00:00Z".to_string()),
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(expired_status.burst_used_units, 0);
            assert_eq!(expired_status.burst_reset_at, "2026-05-27T01:00:00Z");
            assert_eq!(db.finite_private_grant(&grant.id).await.unwrap(), started);
        })
        .await;
    }

    #[tokio::test]
    async fn finite_private_daily_reset_is_once_per_utc_day_and_epoch_safe() {
        with_isolated_postgres(|db| async move {
            let grant = db
                .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                    verified_email: "reset@finite.vip".to_string(),
                    workos_user_id: Some("user_workos_reset".to_string()),
                    limit_profile_id: None,
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            db.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id.clone(),
                raw_key: "fpk_live_reset".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
            let reserved = db
                .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                    request_id: "req-before-reset".to_string(),
                    presented_api_key: "fpk_live_reset".to_string(),
                    endpoint: "/v1/chat/completions".to_string(),
                    model: "glm-5.2".to_string(),
                    estimated_prompt_tokens: 10,
                    estimated_completion_tokens: 0,
                    estimated_usage_units: 10,
                    usage_formula_version: "v1".to_string(),
                    dashboard_url: "https://finite.computer/dashboard".to_string(),
                    now: Some("2026-05-26T23:59:00Z".to_string()),
                })
                .await
                .unwrap();
            let old_epoch = db
                .finite_private_grant(&grant.id)
                .await
                .unwrap()
                .burst_window_epoch;

            let reset = db
                .claim_finite_private_daily_reset_for_api_key(
                    "fpk_live_reset",
                    Some("2026-05-26T23:59:30Z".to_string()),
                )
                .await
                .unwrap();
            assert!(reset.performed);
            assert_eq!(reset.status.burst_used_units, 0);
            assert_eq!(
                reset.status.free_daily_reset_available_again_at,
                "2026-05-27T00:00:00Z"
            );
            assert_eq!(
                db.finite_private_grant(&grant.id)
                    .await
                    .unwrap()
                    .burst_window_epoch,
                old_epoch + 1
            );

            db.settle_finite_private_reservation(SettleFinitePrivateReservationInput {
                reservation_id: reserved.reservation_id.unwrap(),
                request_id: "req-before-reset".to_string(),
                settlement: FinitePrivateSettlementKind::Actual,
                prompt_tokens: Some(5),
                completion_tokens: Some(0),
                usage_units: Some(5),
                usage_formula_version: "v1".to_string(),
                upstream_status: Some(200),
                upstream_error_class: None,
                now: Some("2026-05-26T23:59:40Z".to_string()),
            })
            .await
            .unwrap();
            assert_eq!(
                db.finite_private_grant(&grant.id)
                    .await
                    .unwrap()
                    .current_window_used_units,
                0
            );

            let repeated = db
                .claim_finite_private_daily_reset_for_api_key(
                    "fpk_live_reset",
                    Some("2026-05-26T23:59:50Z".to_string()),
                )
                .await
                .unwrap();
            assert!(!repeated.performed);
            let next_day = db
                .claim_finite_private_daily_reset_for_workos_user(
                    "user_workos_reset",
                    Some("2026-05-27T00:00:00Z".to_string()),
                )
                .await
                .unwrap()
                .unwrap();
            assert!(next_day.performed);
        })
        .await;
    }

    #[tokio::test]
    async fn finite_private_threshold_notices_are_strongest_once_per_epoch() {
        with_isolated_postgres(|db| async move {
            let grant = db
                .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                    verified_email: "notice@finite.vip".to_string(),
                    workos_user_id: Some("user_workos_notice".to_string()),
                    limit_profile_id: None,
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            db.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id,
                raw_key: "fpk_live_notice".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();

            for (request_id, units, at) in [
                ("req-notice-25", 76_000_000, "2026-05-26T13:00:00Z"),
                ("req-notice-10", 16_000_000, "2026-05-26T13:10:00Z"),
            ] {
                let reserved = db
                    .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                        request_id: request_id.to_string(),
                        presented_api_key: "fpk_live_notice".to_string(),
                        endpoint: "/v1/chat/completions".to_string(),
                        model: "glm-5.2".to_string(),
                        estimated_prompt_tokens: units,
                        estimated_completion_tokens: 0,
                        estimated_usage_units: units,
                        usage_formula_version: "v1".to_string(),
                        dashboard_url: "https://finite.computer/dashboard".to_string(),
                        now: Some(at.to_string()),
                    })
                    .await
                    .unwrap();
                db.settle_finite_private_reservation(SettleFinitePrivateReservationInput {
                    reservation_id: reserved.reservation_id.unwrap(),
                    request_id: request_id.to_string(),
                    settlement: FinitePrivateSettlementKind::Actual,
                    prompt_tokens: Some(units),
                    completion_tokens: Some(0),
                    usage_units: Some(units),
                    usage_formula_version: "v1".to_string(),
                    upstream_status: Some(200),
                    upstream_error_class: None,
                    now: Some(at.to_string()),
                })
                .await
                .unwrap();
                let status = db
                    .finite_private_usage_status_for_api_key(
                        "fpk_live_notice",
                        true,
                        Some(at.to_string()),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    status
                        .notice
                        .as_ref()
                        .map(|notice| notice.threshold_remaining_percent),
                    Some(if units == 76_000_000 { 25 } else { 10 })
                );
                assert!(
                    status
                        .notice
                        .unwrap()
                        .message
                        .contains(&status.burst_reset_at)
                );
            }
            let repeated = db
                .finite_private_usage_status_for_api_key(
                    "fpk_live_notice",
                    true,
                    Some("2026-05-26T13:20:00Z".to_string()),
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(repeated.notice, None);
        })
        .await;
    }

    #[tokio::test]
    async fn finite_private_settlement_retry_is_idempotent_but_mismatch_conflicts() {
        with_isolated_postgres(|db| async move {
            let grant = db
                .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                    verified_email: "settle@finite.vip".to_string(),
                    workos_user_id: None,
                    limit_profile_id: None,
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            db.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id,
                raw_key: "fpk_live_settle".to_string(),
                project_id: None,
                agent_runtime_id: None,
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
            let reserved = db
                .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                    request_id: "req-settle-retry".to_string(),
                    presented_api_key: "fpk_live_settle".to_string(),
                    endpoint: "/v1/chat/completions".to_string(),
                    model: "glm-5.2".to_string(),
                    estimated_prompt_tokens: 100,
                    estimated_completion_tokens: 0,
                    estimated_usage_units: 100,
                    usage_formula_version: "v1".to_string(),
                    dashboard_url: "https://finite.computer/dashboard".to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();
            let input = SettleFinitePrivateReservationInput {
                reservation_id: reserved.reservation_id.unwrap(),
                request_id: "req-settle-retry".to_string(),
                settlement: FinitePrivateSettlementKind::Actual,
                prompt_tokens: Some(80),
                completion_tokens: Some(0),
                usage_units: Some(80),
                usage_formula_version: "v1".to_string(),
                upstream_status: Some(200),
                upstream_error_class: None,
                now: Some(NOW.to_string()),
            };
            assert!(
                db.settle_finite_private_reservation(input.clone())
                    .await
                    .unwrap()
                    .settled
            );
            assert!(
                db.settle_finite_private_reservation(input.clone())
                    .await
                    .unwrap()
                    .settled
            );
            let mut mismatch = input;
            mismatch.usage_units = Some(81);
            assert!(matches!(
                db.settle_finite_private_reservation(mismatch).await,
                Err(CoreError::FinitePrivateReservationAlreadySettled)
            ));
        })
        .await;
    }

    #[test]
    fn schema_is_postgres_first_and_contains_first_bridge_tables() {
        for table in [
            "users",
            "customer_orgs",
            // Written only by the deleted existing-host import bridge; the
            // table stays because production may hold rows from its 2026-07
            // test run and dropping schema is a rollback boundary.
            "project_import_candidates",
            "projects",
            "runtime_artifacts",
            "agent_runtimes",
            "runtime_relay_credentials",
            "project_runtime_links",
            "chat_identities",
            "project_room_memberships",
            // Writer removed; the table stays because production may hold rows
            // and dropping schema is a rollback boundary (separate gated
            // migration).
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
            "finite_private_daily_resets",
            "finite_private_notice_claims",
            "runner_capacity_fences",
        ] {
            assert!(CORE_SCHEMA_SQL.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")));
        }

        assert!(CORE_SCHEMA_SQL.contains("JSONB"));
        assert!(CORE_SCHEMA_SQL.contains("TIMESTAMPTZ"));
        assert!(CORE_SCHEMA_SQL.contains("finite-private-generous-v2"));
        assert!(CORE_SCHEMA_SQL.contains("100000000"));
        assert!(CORE_SCHEMA_SQL.contains(FINITE_PRIVATE_5X_LIMIT_PROFILE));
        assert!(CORE_SCHEMA_SQL.contains("500000000"));
        assert!(CORE_SCHEMA_SQL.contains("weekly_limit_units = NULL"));
        assert!(!CORE_SCHEMA_SQL.to_lowercase().contains("sqlite"));
        // Operator-rescue scripts stay out of CORE_SCHEMA_SQL by construction:
        // CORE_SCHEMA_SQL is an explicit concat! allowlist and each rescue
        // lives in its own const, so Core startup can never mutate user state
        // outside the migration ladder. Keyed on each rescue's audit action,
        // which appears nowhere else in the schema.
        assert!(!CORE_SCHEMA_SQL.contains("runtime.upgrade.rollback_rescue"));
        assert!(!CORE_SCHEMA_SQL.contains("runtime.lifecycle.reverse_remap"));
    }

    #[test]
    fn expand_domain_reads_old_rows_and_n_minus_one_ignores_new_fields() {
        let old_project: Project = serde_json::from_value(json!({
            "id": "project-old",
            "customer_org_id": "org-old",
            "owner_user_id": "user-old",
            "display_name": "Old Agent",
            "import_candidate_id": null,
            "created_at": NOW,
            "updated_at": NOW
        }))
        .unwrap();
        assert_eq!(old_project.hosting_tier, None);
        assert_eq!(old_project.placement, None);

        let new_project = Project {
            hosting_tier: Some(HostingTier::Standard),
            placement: Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
            ..old_project
        };
        #[derive(Deserialize)]
        struct NMinusOneProject {
            id: String,
            display_name: String,
        }
        let legacy: NMinusOneProject =
            serde_json::from_value(serde_json::to_value(&new_project).unwrap()).unwrap();
        assert_eq!(legacy.id, "project-old");
        assert_eq!(legacy.display_name, "Old Agent");

        let old_runtime: AgentRuntime = serde_json::from_value(json!({
            "id": "runtime-old",
            "project_id": "project-old",
            "source_host_id": "legacy-host",
            "source_machine_id": "legacy-machine",
            "source_import_key": "legacy-host:legacy-machine",
            "runtime_artifact_id": null,
            "state_schema_version": null,
            "host_facts": {
                "display_name": "Old Agent",
                "hostname": null,
                "runtime_host": "legacy-host",
                "runtime_status": "online",
                "active_inference_profile": null,
                "hermes_available": true,
                "published_app_urls": []
            },
            "created_at": NOW,
            "updated_at": NOW
        }))
        .unwrap();
        assert_eq!(old_runtime.placement, None);
        assert_eq!(old_runtime.provider_runtime_handle, None);
        assert!(old_runtime.provider_runtime_handle_history.is_empty());

        let new_runtime = AgentRuntime {
            placement: Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
            provider_runtime_handle: Some(ProviderRuntimeHandleEnvelope::V1(
                ProviderRuntimeHandleV1 {
                    runner_class: RunnerClass::Kata,
                    opaque: json!({"container": "finite-kata-old"}),
                },
            )),
            provider_runtime_handle_history: vec![ProviderRuntimeHandleEnvelope::V1(
                ProviderRuntimeHandleV1 {
                    runner_class: RunnerClass::Kata,
                    opaque: json!({"container": "finite-kata-old"}),
                },
            )],
            contact_endpoint: Some("https://old.example.test/contact".to_string()),
            ..old_runtime
        };
        #[derive(Deserialize)]
        struct NMinusOneRuntime {
            id: String,
            source_host_id: String,
            source_machine_id: String,
        }
        let legacy_runtime: NMinusOneRuntime =
            serde_json::from_value(serde_json::to_value(new_runtime).unwrap()).unwrap();
        assert_eq!(legacy_runtime.id, "runtime-old");
        assert_eq!(legacy_runtime.source_host_id, "legacy-host");
        assert_eq!(legacy_runtime.source_machine_id, "legacy-machine");
    }

    #[test]
    fn versioned_runtime_identity_envelopes_fail_closed_on_unknown_schema() {
        let unknown_spec = serde_json::from_value::<RuntimeSpecEnvelope>(json!({
            "schema": "runtime_spec.v2",
            "spec": {}
        }));
        assert!(unknown_spec.is_err());

        let n_minus_one_spec = serde_json::from_value::<RuntimeSpecEnvelope>(json!({
            "schema": "runtime_spec.v1",
            "spec": {
                "operationId": "agent-request-old",
                "projectId": "project-old",
                "agentRuntimeId": "runtime-old",
                "placement": {
                    "runnerClass": "kata",
                    "runtimeResourceClass": "vcpu4_memory8_gib"
                },
                "runtimeArtifactId": "artifact-v1",
                "runtimeImageDigest": format!(
                    "ghcr.io/finitecomputer/agent-runtime:v1@sha256:{}",
                    "a".repeat(64)
                ),
                "stateSchemaVersion": "state-v1",
                "durableStateId": "runtime-old",
                "endpoints": {
                    "servicePort": 8080,
                    "healthPath": "/healthz",
                    "contactPath": "/contact"
                },
                "environment": {},
                "secretReferences": ["FINITE_PRIVATE_API_KEY"]
            }
        }))
        .unwrap();
        assert_eq!(
            runtime_spec_v1(&n_minus_one_spec).boot_intent,
            RuntimeBootIntent::Normal
        );

        let unknown_handle = serde_json::from_value::<ProviderRuntimeHandleEnvelope>(json!({
            "schema": "provider_runtime_handle.v2",
            "handle": {"runnerClass": "phala", "opaque": {}}
        }));
        assert!(unknown_handle.is_err());
        assert!(matches!(
            normalize_runtime_contact_endpoint(Some("file:///tmp/contact")),
            Err(CoreError::InvalidRuntimeContactEndpoint)
        ));
    }

    #[tokio::test]
    async fn admin_runtime_control_skips_owner_check_and_matches_runner_lease_shape() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_id = complete_self_serve_agent(
                &db,
                "owner@finite.vip",
                "user_workos_owner",
                "first-submit",
                "oslo-agent-001",
                "artifact-v1",
                "2026-05-25T13:02:00Z",
            )
            .await;
            let project_id = db
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();

            // The owner-scoped path rejects non-owners outright.
            let denied = db
                .request_runtime_restart(RequestRuntimeRestartInput {
                    verified_email: "admin@finite.vip".to_string(),
                    workos_user_id: "user_workos_admin".to_string(),
                    project_id: project_id.clone(),
                    now: Some("2026-05-25T13:03:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(denied, CoreError::ProjectNotFound));

            // The admin path creates the request without owning the project.
            let restart = db
                .admin_request_runtime_restart(AdminRuntimeControlInput {
                    admin_verified_email: "Admin@Finite.VIP".to_string(),
                    admin_workos_user_id: "user_workos_admin".to_string(),
                    project_id: project_id.clone(),
                    now: Some("2026-05-25T13:03:30Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(restart.project_id, project_id);
            assert_eq!(restart.agent_runtime_id, runtime_id);
            assert_eq!(restart.source_host_id, "oslo-host-1");
            assert_eq!(restart.source_machine_id, "oslo-agent-001");
            assert_eq!(restart.kind, RuntimeControlKind::Restart);
            assert_eq!(restart.status, RuntimeControlRequestStatus::Requested);
            assert_eq!(
                restart.requested_by_user_id,
                db.user_by_email("admin@finite.vip").await.unwrap().id
            );

            // Idempotent while an equivalent request is pending, like the owner path.
            let duplicate = db
                .admin_request_runtime_restart(AdminRuntimeControlInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    admin_workos_user_id: "user_workos_admin".to_string(),
                    project_id: project_id.clone(),
                    now: Some("2026-05-25T13:04:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(duplicate.id, restart.id);

            // The runner consumes it through the exact same lease machinery.
            let lease = db
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "admin-restart-lease-1".to_string(),
                    lease_seconds: Some(60),
                    source_host_id: Some("oslo-host-1".to_string()),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: Some("2026-05-25T13:04:30Z".to_string()),
                })
                .await
                .unwrap()
                .expect("admin restart request should lease");
            assert_eq!(lease.request.id, restart.id);
            assert_eq!(lease.request.status, RuntimeControlRequestStatus::Launching);
            assert_eq!(lease.runtime.source_machine_id, "oslo-agent-001");
            let completed = db
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: restart.id.clone(),
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "admin-restart-lease-1".to_string(),
                    runtime_artifact_id: None,
                    state_schema_version: None,
                    runtime_capabilities: None,
                    runtime_host: None,
                    published_app_urls: None,
                    retirement_snapshot: None,
                    now: Some("2026-05-25T13:05:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(completed.status, RuntimeControlRequestStatus::Succeeded);

            // Recovery is not restart-by-another-name: until a genuine recovery
            // implementation exists, even the admin path is fail closed.
            let recover_error = db
                .admin_request_runtime_recover_known_good_chat(AdminRuntimeControlInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    admin_workos_user_id: "user_workos_admin".to_string(),
                    project_id,
                    now: Some("2026-05-25T13:06:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(
                recover_error,
                CoreError::RuntimeControlUnsupported
            ));

            let actions = db
                .finite_private_admin_audit_events()
                .await
                .unwrap()
                .iter()
                .map(|event| (event.action.clone(), event.actor.clone()))
                .collect::<Vec<_>>();
            assert!(actions.contains(&(
                "runtime.admin_restart".to_string(),
                "admin@finite.vip".to_string()
            )));
            assert!(
                !actions
                    .iter()
                    .any(|(action, _)| action == "runtime.admin_recover_known_good_chat")
            );
        })
        .await;
    }

    #[tokio::test]
    async fn admin_runtime_retirement_requires_the_exact_active_binding() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_id = complete_self_serve_agent(
                &db,
                "owner@finite.vip",
                "user_workos_owner_retire",
                "retire-submit",
                "oslo-agent-retire",
                "artifact-v1",
                "2026-07-22T15:00:00Z",
            )
            .await;
            let project_id = db
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();
            let retirement_capable =
                serde_json::to_string(&RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                    runtime_retirement: true,
                    ..*kata_runtime_capabilities().v1()
                }))
                .unwrap();
            db.exec(&format!(
                "UPDATE agent_runtimes SET runtime_capabilities = '{retirement_capable}'::jsonb \
                 WHERE id = '{runtime_id}'"
            ))
            .await;

            let changed_binding = db
                .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    admin_workos_user_id: "user_workos_admin_retire".to_string(),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: "runtime-replaced-after-review".to_string(),
                    expected_source_host_id: "oslo-host-1".to_string(),
                    expected_source_machine_id: "oslo-agent-retire".to_string(),
                    now: Some("2026-07-22T15:01:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(changed_binding, CoreError::RuntimeSpecMismatch));
            assert!(db.all_runtime_control_requests().await.is_empty());

            let retirement = db
                .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                    admin_verified_email: "Admin@Finite.VIP".to_string(),
                    admin_workos_user_id: "user_workos_admin_retire".to_string(),
                    project_id,
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: "oslo-host-1".to_string(),
                    expected_source_machine_id: "oslo-agent-retire".to_string(),
                    now: Some("2026-07-22T15:02:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(retirement.kind, RuntimeControlKind::Destroy);
            assert_eq!(retirement.agent_runtime_id, runtime_id);
            assert_eq!(retirement.status, RuntimeControlRequestStatus::Requested);
            assert!(
                db.finite_private_admin_audit_events()
                    .await
                    .unwrap()
                    .iter()
                    .any(|event| {
                        event.action == "runtime.admin_destroy"
                            && event.actor == "admin@finite.vip"
                            && event.target_id == retirement.agent_runtime_id
                    })
            );
        })
        .await;
    }

    /// Unrecoverable archive is fail-closed on every guard, and a successful
    /// archive retains history.
    ///
    /// Each rejected attempt leaves state untouched, so they run in sequence
    /// against one database instead of forking an in-memory snapshot.
    #[tokio::test]
    async fn cold_relocation_is_stopped_exact_targeted_and_failure_preserves_source_runtime() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_id = complete_self_serve_agent(
                &db,
                "canary@finite.vip",
                "workos-canary",
                "canary-create",
                "finite-kata-canary",
                "artifact-v1",
                "2026-05-25T13:00:00Z",
            )
            .await;
            let project_id = db
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();
            let running = db
                .admin_request_runtime_relocate_exact(AdminRuntimeRelocateExactInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    admin_workos_user_id: "workos-admin".to_string(),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: "oslo-host-1".to_string(),
                    expected_source_machine_id: "finite-kata-canary".to_string(),
                    target_source_host_id: "oslo-host-3".to_string(),
                    expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                    durable_state_manifest_sha256: "b".repeat(64),
                    operator_observed_compute_absent: false,
                    now: Some("2026-05-25T13:01:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(running, CoreError::RuntimeControlUnsupported));

            // Stopping is the precondition for relocation, not the subject of
            // this test. The owner path reaches the same state; the admin
            // variant was an in-memory-only helper.
            let stop = db
                .request_runtime_stop(RequestRuntimeStopInput {
                    verified_email: "canary@finite.vip".to_string(),
                    workos_user_id: "workos-canary".to_string(),
                    project_id: project_id.clone(),
                    now: Some("2026-05-25T13:02:00Z".to_string()),
                })
                .await
                .unwrap();
            let capacity = RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Kata],
                runtime_capabilities: Some(kata_runtime_capabilities()),
                ..RunnerLeaseCapacity::default()
            };
            let stop_lease = db
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "stop-lease".to_string(),
                    lease_seconds: Some(300),
                    source_host_id: Some("oslo-host-1".to_string()),
                    runner_capacity: Some(capacity.clone()),
                    now: Some("2026-05-25T13:03:00Z".to_string()),
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stop_lease.request.id, stop.id);
            db.complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id: stop.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "stop-lease".to_string(),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: None,
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: None,
                now: Some("2026-05-25T13:04:00Z".to_string()),
            })
            .await
            .unwrap();

            let relocation = db
                .admin_request_runtime_relocate_exact(AdminRuntimeRelocateExactInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    admin_workos_user_id: "workos-admin".to_string(),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: "oslo-host-1".to_string(),
                    expected_source_machine_id: "finite-kata-canary".to_string(),
                    target_source_host_id: "oslo-host-3".to_string(),
                    expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                    durable_state_manifest_sha256: "b".repeat(64),
                    operator_observed_compute_absent: false,
                    now: Some("2026-05-25T13:05:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(
                relocation.target_source_host_id.as_deref(),
                Some("oslo-host-3")
            );
            assert_eq!(
                relocation.agent_runtime_id.as_deref(),
                Some(runtime_id.as_str())
            );
            assert!(relocation.relocation.is_some());

            assert!(
                db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "wrong-host".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(capacity.clone()),
                    source_host_id: Some("oslo-host-1".to_string()),
                    now: Some("2026-05-25T13:06:00Z".to_string()),
                })
                .await
                .unwrap()
                .is_none()
            );
            let lease = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-oslo-3".to_string(),
                    lease_token: "relocate-lease".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(capacity),
                    source_host_id: Some("oslo-host-3".to_string()),
                    now: Some("2026-05-25T13:06:00Z".to_string()),
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(lease.request.id, relocation.id);
            db.register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                request_id: relocation.id.clone(),
                runner_id: "runner-oslo-3".to_string(),
                lease_token: "relocate-lease".to_string(),
                source_host_id: "oslo-host-3".to_string(),
                source_machine_id: "finite-kata-canary".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: Some("db-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://oslo-host-3:4201/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: None,
                hostname: None,
                runtime_host: Some("http://oslo-host-3:4201".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Unknown),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                now: Some("2026-05-25T13:06:30Z".to_string()),
            })
            .await
            .unwrap();
            let still_source = db.agent_runtime(&runtime_id).await.unwrap();
            assert_eq!(still_source.source_host_id, "oslo-host-1");
            assert_eq!(
                still_source.host_facts.runtime_status,
                RuntimeSummaryStatus::Offline
            );
            db.fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id: relocation.id.clone(),
                runner_id: "runner-oslo-3".to_string(),
                lease_token: "relocate-lease".to_string(),
                failure_message: "synthetic target launch failure".to_string(),
                provisioned_finite_private_api_key_id: None,
                now: Some("2026-05-25T13:07:00Z".to_string()),
            })
            .await
            .unwrap();
            let source = db.agent_runtime(&runtime_id).await.unwrap();
            assert_eq!(source.source_host_id, "oslo-host-1");
            assert_eq!(
                source.host_facts.runtime_status,
                RuntimeSummaryStatus::Offline
            );
            assert_eq!(
                db.agent_creation_request(&relocation.id)
                    .await
                    .unwrap()
                    .agent_runtime_id
                    .as_deref(),
                Some(runtime_id.as_str())
            );
            assert!(db.all("project_runtime_links").await.iter().any(|link| {
                link["project_id"] == project_id.as_str()
                    && link["agent_runtime_id"] == runtime_id.as_str()
                    && link["active"] == true
            }));
        })
        .await;
    }

    #[tokio::test]
    async fn cold_relocation_with_absent_compute_accepts_stale_and_waives_stop_receipt() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_id = complete_self_serve_agent(
                &db,
                "eddie-case@finite.vip",
                "workos-eddie-case",
                "absent-create",
                "finite-kata-absent",
                "artifact-v1",
                "2026-08-12T13:00:00Z",
            )
            .await;
            let project_id = db
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();
            let relocate_input = |absent: bool, now: &str| AdminRuntimeRelocateExactInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-admin".to_string(),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: "oslo-host-1".to_string(),
                expected_source_machine_id: "finite-kata-absent".to_string(),
                target_source_host_id: "oslo-host-3".to_string(),
                expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                durable_state_manifest_sha256: "c".repeat(64),
                operator_observed_compute_absent: absent,
                now: Some(now.to_string()),
            };

            // Without the attestation an online runtime may still be
            // running, so the exact relocation must refuse; the
            // attested-online acceptance and its survival of the
            // registration-time validation are pinned by the sibling
            // test below.
            let online = db
                .admin_request_runtime_relocate_exact(relocate_input(false, "2026-08-12T13:01:00Z"))
                .await
                .unwrap_err();
            assert!(matches!(online, CoreError::RuntimeControlUnsupported));

            // Reach `stale` the way absent compute does in production: a
            // control request that fails at the provider.
            let restart = db
                .request_runtime_restart(RequestRuntimeRestartInput {
                    verified_email: "eddie-case@finite.vip".to_string(),
                    workos_user_id: "workos-eddie-case".to_string(),
                    project_id: project_id.clone(),
                    now: Some("2026-08-12T13:02:00Z".to_string()),
                })
                .await
                .unwrap();
            let capacity = RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Kata],
                runtime_capabilities: Some(kata_runtime_capabilities()),
                ..RunnerLeaseCapacity::default()
            };
            let restart_lease = db
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "restart-lease".to_string(),
                    lease_seconds: Some(300),
                    source_host_id: Some("oslo-host-1".to_string()),
                    runner_capacity: Some(capacity),
                    now: Some("2026-08-12T13:03:00Z".to_string()),
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(restart_lease.request.id, restart.id);
            db.fail_runtime_control_request(FailRuntimeControlRequestInput {
                request_id: restart.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: "restart-lease".to_string(),
                failure_message: "no such object finite-kata-absent".to_string(),
                failure_stage: Some(RuntimeLifecycleStage::Compute),
                now: Some("2026-08-12T13:04:00Z".to_string()),
            })
            .await
            .unwrap();
            assert_eq!(
                db.agent_runtime(&runtime_id)
                    .await
                    .unwrap()
                    .host_facts
                    .runtime_status,
                RuntimeSummaryStatus::Stale
            );
            let stale_overview = db
                .admin_runtime_overviews()
                .await
                .unwrap()
                .into_iter()
                .find(|overview| overview.agent_runtime_id == runtime_id)
                .unwrap();
            assert_eq!(
                stale_overview.runtime_status,
                db.agent_runtime(&runtime_id)
                    .await
                    .unwrap()
                    .host_facts
                    .runtime_status
            );

            // Without the attestation, `stale` (and the missing stop
            // receipt) keep refusing — the existing posture is pinned.
            let unattested = db
                .admin_request_runtime_relocate_exact(relocate_input(false, "2026-08-12T13:05:00Z"))
                .await
                .unwrap_err();
            assert!(matches!(unattested, CoreError::RuntimeControlUnsupported));

            // With it, the enqueue succeeds despite `stale` and despite the
            // runtime never having a succeeded stop, and the attestation
            // rides the envelope for lease-time validation.
            let relocation = db
                .admin_request_runtime_relocate_exact(relocate_input(true, "2026-08-12T13:06:00Z"))
                .await
                .unwrap();
            assert_eq!(
                relocation.agent_runtime_id.as_deref(),
                Some(runtime_id.as_str())
            );
            assert_eq!(
                relocation.target_source_host_id.as_deref(),
                Some("oslo-host-3")
            );
            let envelope = relocation.relocation.as_ref().unwrap().v1();
            assert!(envelope.source_compute_absent);
            assert_eq!(envelope.source_machine_id, "finite-kata-absent");
        })
        .await;
    }

    #[tokio::test]
    async fn cold_relocation_with_absent_compute_accepts_attested_online_through_registration() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_id = complete_self_serve_agent(
                &db,
                "online-case@finite.vip",
                "workos-online-case",
                "absent-online-create",
                "finite-kata-absent-online",
                "artifact-v1",
                "2026-08-12T14:00:00Z",
            )
            .await;
            let project_id = db
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();
            let relocate_input = |absent: bool, now: &str| AdminRuntimeRelocateExactInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "workos-admin".to_string(),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: "oslo-host-1".to_string(),
                expected_source_machine_id: "finite-kata-absent-online".to_string(),
                target_source_host_id: "oslo-host-3".to_string(),
                expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                durable_state_manifest_sha256: "c".repeat(64),
                operator_observed_compute_absent: absent,
                now: Some(now.to_string()),
            };

            // Without the attestation the pre-death `online` report is not
            // frozen: the runtime may still be running, so refuse.
            let unattested = db
                .admin_request_runtime_relocate_exact(relocate_input(false, "2026-08-12T14:01:00Z"))
                .await
                .unwrap_err();
            assert!(matches!(unattested, CoreError::RuntimeControlUnsupported));

            // Under the attestation `online` is exactly as frozen as the
            // attested `stale` in the test above: the attestation rides the
            // envelope for the target runner's lease-time and
            // registration/completion-time validation.
            let relocation = db
                .admin_request_runtime_relocate_exact(relocate_input(true, "2026-08-12T14:02:00Z"))
                .await
                .unwrap();
            assert_eq!(
                relocation.agent_runtime_id.as_deref(),
                Some(runtime_id.as_str())
            );
            assert_eq!(
                relocation.target_source_host_id.as_deref(),
                Some("oslo-host-3")
            );
            let envelope = relocation.relocation.as_ref().unwrap().v1();
            assert!(envelope.source_compute_absent);
            assert_eq!(envelope.source_machine_id, "finite-kata-absent-online");

            let capacity = RunnerLeaseCapacity {
                runner_classes: vec![RunnerClass::Kata],
                runtime_capabilities: Some(kata_runtime_capabilities()),
                ..RunnerLeaseCapacity::default()
            };
            assert!(
                db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-oslo-1".to_string(),
                    lease_token: "wrong-host".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(capacity.clone()),
                    source_host_id: Some("oslo-host-1".to_string()),
                    now: Some("2026-08-12T14:03:00Z".to_string()),
                })
                .await
                .unwrap()
                .is_none()
            );
            let lease = db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-oslo-3".to_string(),
                    lease_token: "relocate-online-lease".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(capacity),
                    source_host_id: Some("oslo-host-3".to_string()),
                    now: Some("2026-08-12T14:03:30Z".to_string()),
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(lease.request.id, relocation.id);

            // Registration on the target re-validates the envelope while the
            // source is still `online`: only the recorded attestation keeps
            // that status frozen. Registration stays non-mutating, so the
            // source binding survives untouched.
            db.register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                request_id: relocation.id.clone(),
                runner_id: "runner-oslo-3".to_string(),
                lease_token: "relocate-online-lease".to_string(),
                source_host_id: "oslo-host-3".to_string(),
                source_machine_id: "finite-kata-absent-online".to_string(),
                runtime_artifact_id: Some("artifact-v1".to_string()),
                state_schema_version: Some("db-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://oslo-host-3:4201/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: None,
                hostname: None,
                runtime_host: Some("http://oslo-host-3:4201".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Unknown),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                now: Some("2026-08-12T14:04:00Z".to_string()),
            })
            .await
            .unwrap();
            let still_source = db.agent_runtime(&runtime_id).await.unwrap();
            assert_eq!(still_source.source_host_id, "oslo-host-1");
            assert_eq!(
                still_source.host_facts.runtime_status,
                RuntimeSummaryStatus::Online
            );

            // Completion re-validates the same envelope a second time and is
            // the single transaction that replaces the source binding.
            let completed = db
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: relocation.id.clone(),
                    runner_id: "runner-oslo-3".to_string(),
                    lease_token: "relocate-online-lease".to_string(),
                    source_host_id: "oslo-host-3".to_string(),
                    source_machine_id: "finite-kata-absent-online".to_string(),
                    runtime_artifact_id: Some("artifact-v1".to_string()),
                    state_schema_version: Some("db-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: Some("http://oslo-host-3:4201/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: None,
                    hostname: None,
                    runtime_host: Some("http://oslo-host-3:4201".to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: Some("2026-08-12T14:05:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(
                completed.request.status,
                AgentCreationRequestStatus::Running
            );
            let relocated = db.agent_runtime(&runtime_id).await.unwrap();
            assert_eq!(relocated.source_host_id, "oslo-host-3");
            assert_eq!(
                relocated.host_facts.runtime_status,
                RuntimeSummaryStatus::Online
            );
        })
        .await;
    }

    #[tokio::test]
    async fn unrecoverable_runtime_archive_is_exact_fail_closed_and_retains_history() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_id = complete_self_serve_agent(
                &db,
                "owner@finite.vip",
                "user_workos_owner_archive",
                "archive-submit",
                "legacy-agent-001",
                "artifact-v1",
                "2026-07-21T20:00:00Z",
            )
            .await;
            let project_id = db
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();
            let input = |compute_absent: bool| AdminArchiveUnrecoverableRuntimeInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "user_workos_admin_archive".to_string(),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: "oslo-host-1".to_string(),
                expected_source_machine_id: "legacy-agent-001".to_string(),
                expected_owner_email: "owner@finite.vip".to_string(),
                operator_observed_compute_absent: compute_absent,
                operator_observed_durable_state_absent: true,
                owner_acknowledged_unrecoverable: true,
                now: Some("2026-07-21T20:10:00Z".to_string()),
            };

            // Compute must be observed absent.
            assert!(matches!(
                db.admin_archive_unrecoverable_runtime(input(false)).await,
                Err(CoreError::UnrecoverableRuntimeArchiveAcknowledgementRequired)
            ));
            assert!(db.active_runtime_for_project(&project_id).await.is_some());

            // The binding must match exactly.
            let mut wrong_binding_input = input(true);
            wrong_binding_input.expected_source_machine_id = "replacement-agent".to_string();
            assert!(matches!(
                db.admin_archive_unrecoverable_runtime(wrong_binding_input)
                    .await,
                Err(CoreError::RuntimeSpecMismatch)
            ));

            // Provider metadata means the runtime is not actually unreachable.
            db.exec(&format!(
                "UPDATE agent_runtimes \
                 SET contact_endpoint = 'https://legacy-agent.example.test/contact' \
                 WHERE id = '{runtime_id}'"
            ))
            .await;
            assert!(matches!(
                db.admin_archive_unrecoverable_runtime(input(true)).await,
                Err(CoreError::UnrecoverableRuntimeArchiveProviderMetadataPresent)
            ));
            db.exec(&format!(
                "UPDATE agent_runtimes SET contact_endpoint = NULL WHERE id = '{runtime_id}'"
            ))
            .await;

            // An in-flight control operation blocks the archive until it settles.
            let in_flight = db
                .admin_request_runtime_restart(AdminRuntimeControlInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    admin_workos_user_id: "user_workos_admin_archive".to_string(),
                    project_id: project_id.clone(),
                    now: Some("2026-07-21T20:05:00Z".to_string()),
                })
                .await
                .unwrap();
            assert!(matches!(
                db.admin_archive_unrecoverable_runtime(input(true)).await,
                Err(CoreError::RuntimeControlOperationConflict)
            ));
            db.exec(&format!(
                "UPDATE runtime_control_requests SET status = 'succeeded' WHERE id = '{}'",
                in_flight.id
            ))
            .await;

            let receipt = db
                .admin_archive_unrecoverable_runtime(input(true))
                .await
                .unwrap();
            assert_eq!(receipt.project_id, project_id);
            assert_eq!(receipt.agent_runtime_id, runtime_id);
            assert_eq!(receipt.owner_email, "owner@finite.vip");
            assert_eq!(receipt.revoked_finite_private_key_count, 0);

            // History is retained: the Project and Runtime rows survive, the
            // room membership is archived, and the action is audited.
            assert!(db.active_runtime_for_project(&project_id).await.is_none());
            assert!(db.project(&project_id).await.is_some());
            assert!(db.agent_runtime(&runtime_id).await.is_some());
            assert!(
                db.all("project_room_memberships")
                    .await
                    .iter()
                    .any(|membership| {
                        membership["project_id"] == project_id.as_str()
                            && !membership["archived_at"].is_null()
                    })
            );
            let events = db.finite_private_admin_audit_events().await.unwrap();
            assert!(events.iter().any(|event| {
                event.action == "runtime.admin_archive_unrecoverable"
                    && event.target_id == runtime_id
                    && event.actor == "admin@finite.vip"
            }));
        })
        .await;
    }

    #[tokio::test]
    async fn offboard_retired_runtime_is_exact_fail_closed_and_keeps_the_receipt() {
        with_isolated_postgres(|db| async move {
            let (project_id, runtime_id, destroy_id) = stage_retired_offboard_anomaly(
                &db,
                "owner@finite.vip",
                "user_workos_owner_offboard",
                "offboard-submit",
                "retired-agent-001",
            )
            .await;
            let input = |compute_absent: bool| AdminOffboardRetiredRuntimeInput {
                admin_verified_email: "admin@finite.vip".to_string(),
                admin_workos_user_id: "user_workos_admin_offboard".to_string(),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: "oslo-host-1".to_string(),
                expected_source_machine_id: "retired-agent-001".to_string(),
                expected_owner_email: "owner@finite.vip".to_string(),
                operator_observed_compute_absent: compute_absent,
                now: Some("2026-07-21T20:10:00Z".to_string()),
            };

            // Compute must be observed absent.
            assert!(matches!(
                db.admin_offboard_retired_runtime(input(false)).await,
                Err(CoreError::RetiredRuntimeOffboardAcknowledgementRequired)
            ));
            assert!(db.active_runtime_for_project(&project_id).await.is_some());

            // The binding must match exactly.
            let mut wrong_binding_input = input(true);
            wrong_binding_input.expected_source_machine_id = "replacement-agent".to_string();
            assert!(matches!(
                db.admin_offboard_retired_runtime(wrong_binding_input).await,
                Err(CoreError::RuntimeSpecMismatch)
            ));

            // The owner must match exactly.
            let mut wrong_owner_input = input(true);
            wrong_owner_input.expected_owner_email = "other@finite.vip".to_string();
            assert!(matches!(
                db.admin_offboard_retired_runtime(wrong_owner_input).await,
                Err(CoreError::RetiredRuntimeOffboardOwnerMismatch)
            ));

            let receipt_row_before = db
                .row("runtime_retirement_snapshots", &destroy_id)
                .await
                .expect("staged receipt must read back");
            let receipt = db
                .admin_offboard_retired_runtime(input(true))
                .await
                .unwrap();
            assert_eq!(receipt.project_id, project_id);
            assert_eq!(receipt.agent_runtime_id, runtime_id);
            assert_eq!(receipt.retirement_request_id, destroy_id);
            assert_eq!(
                receipt.retirement_locator,
                runtime_retirement_archive_locator(&destroy_id)
            );

            // Offboarding completed: the link is inactive and the membership
            // archived, while Project, Runtime, and receipt rows survive.
            assert!(db.active_runtime_for_project(&project_id).await.is_none());
            assert!(db.project(&project_id).await.is_some());
            assert!(db.agent_runtime(&runtime_id).await.is_some());
            assert!(
                db.all("project_room_memberships")
                    .await
                    .iter()
                    .any(|membership| {
                        membership["project_id"] == project_id.as_str()
                            && !membership["archived_at"].is_null()
                    })
            );
            assert_eq!(
                db.row("runtime_retirement_snapshots", &destroy_id)
                    .await
                    .unwrap(),
                receipt_row_before,
                "the repair must not touch the stored receipt"
            );
            let events = db.finite_private_admin_audit_events().await.unwrap();
            assert!(events.iter().any(|event| {
                event.action == "runtime.admin_offboard_retired"
                    && event.target_id == runtime_id
                    && event.actor == "admin@finite.vip"
            }));

            // A rerun fails closed on the inactive link.
            assert!(matches!(
                db.admin_offboard_retired_runtime(input(true)).await,
                Err(CoreError::ProjectRuntimeNotFound)
            ));
        })
        .await;
    }

    #[tokio::test]
    async fn admin_friend_key_issue_mirrors_cli_and_records_admin_audit() {
        with_isolated_postgres(|db| async move {
            let raw_key = "fpk_live_test_friend_key_material_0001";
            let issued = db
                .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    friend_email: "Friend@Finite.VIP".to_string(),
                    limit_profile_id: None,
                    raw_key: raw_key.to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();

            assert_eq!(issued.grant.status, FinitePrivateGrantStatus::Active);
            assert_eq!(issued.grant.limit_profile_id, "finite-private-generous-v2");
            assert_eq!(issued.api_key.status, FinitePrivateApiKeyStatus::Active);
            assert_ne!(issued.api_key.key_hash, raw_key);
            assert!(issued.api_key.project_id.is_none());
            assert!(issued.api_key.agent_runtime_id.is_none());

            let resolved = db
                .finite_private_key_and_grant(raw_key)
                .await
                .expect("issued raw key should validate");
            assert_eq!(resolved.0.id, issued.api_key.id);
            assert_eq!(resolved.1.id, issued.grant.id);

            let events = db.finite_private_admin_audit_events().await.unwrap();
            let admin_event = events
                .iter()
                .find(|event| event.action == "finite_private.friend_key.admin_issue")
                .expect("friend key issue should record an admin audit event");
            assert_eq!(admin_event.actor, "admin@finite.vip");
            assert_eq!(
                admin_event.api_key_id.as_deref(),
                Some(issued.api_key.id.as_str())
            );
        })
        .await;
    }

    #[tokio::test]
    async fn admin_rotate_invalidates_old_raw_key_and_revoke_disables_key() {
        with_isolated_postgres(|db| async move {
            let old_raw = "fpk_live_old_raw_key_material_000000001";
            let issued = db
                .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    friend_email: "friend@finite.vip".to_string(),
                    limit_profile_id: None,
                    raw_key: old_raw.to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();

            let new_raw = "fpk_live_new_raw_key_material_000000002";
            let rotated = db
                .admin_rotate_finite_private_api_key(AdminRotateFinitePrivateApiKeyInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    key_id: issued.api_key.id.clone(),
                    raw_key: new_raw.to_string(),
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap();
            assert_ne!(rotated.id, issued.api_key.id);
            assert_eq!(rotated.status, FinitePrivateApiKeyStatus::Active);

            assert!(
                db.finite_private_key_and_grant(old_raw).await.is_none(),
                "old raw key must stop validating after rotate"
            );
            let resolved = db
                .finite_private_key_and_grant(new_raw)
                .await
                .expect("new raw key should validate");
            assert_eq!(resolved.0.id, rotated.id);
            assert_eq!(
                db.finite_private_api_key(&issued.api_key.id)
                    .await
                    .unwrap()
                    .status,
                FinitePrivateApiKeyStatus::Revoked
            );

            let revoked = db
                .admin_revoke_finite_private_api_key(AdminRevokeFinitePrivateApiKeyInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    key_id: rotated.id.clone(),
                    now: Some("2026-05-25T14:00:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(revoked.status, FinitePrivateApiKeyStatus::Revoked);
            assert!(db.finite_private_key_and_grant(new_raw).await.is_none());

            let actions = db
                .finite_private_admin_audit_events()
                .await
                .unwrap()
                .iter()
                .filter(|event| event.actor == "admin@finite.vip")
                .map(|event| event.action.clone())
                .collect::<Vec<_>>();
            assert!(actions.contains(&"finite_private.api_key.admin_rotate".to_string()));
            assert!(actions.contains(&"finite_private.api_key.admin_revoke".to_string()));
        })
        .await;
    }

    #[tokio::test]
    async fn admin_window_reset_clears_burst_window_but_not_weekly_reservations() {
        with_isolated_postgres(|db| async move {
            let raw_key = "fpk_live_reset_raw_key_material_00000003";
            let issued = db
                .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    friend_email: "friend@finite.vip".to_string(),
                    limit_profile_id: None,
                    raw_key: raw_key.to_string(),
                    now: Some(NOW.to_string()),
                })
                .await
                .unwrap();

            let decision = db
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
                .await
                .unwrap();
            assert_eq!(decision.decision, "allow");
            assert_eq!(
                db.finite_private_grant(&issued.grant.id)
                    .await
                    .unwrap()
                    .current_window_used_units,
                1_000
            );

            let reset = db
                .admin_reset_finite_private_usage_window(AdminResetFinitePrivateUsageWindowInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    grant_id: issued.grant.id.clone(),
                    now: Some("2026-05-25T14:00:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(reset.current_window_used_units, 0);
            assert_eq!(
                reset.current_window_started_at.as_deref(),
                Some("2026-05-25T14:00:00Z")
            );

            // Weekly usage is a rolling reservation window; reset must not touch it.
            let (weekly_used, _) = db
                .finite_private_weekly_usage(
                    &issued.grant.id,
                    parse_time("2026-05-25T14:00:00Z").unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(weekly_used, 1_000);

            let events = db.finite_private_admin_audit_events().await.unwrap();
            let admin_event = events
                .iter()
                .find(|event| event.action == "finite_private.grant.admin_window_reset")
                .expect("window reset should record an admin audit event");
            assert_eq!(admin_event.actor, "admin@finite.vip");
        })
        .await;
    }

    #[tokio::test]
    async fn admin_runtime_overviews_assemble_provisioned_box_facts() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_id = complete_self_serve_agent(
                &db,
                "owner@finite.vip",
                "user_workos_owner",
                "first-submit",
                "oslo-agent-001",
                "artifact-v1",
                "2026-05-25T13:02:00Z",
            )
            .await;
            let project_id = db
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();
            let grant = db
                .approve_finite_private_grant(ApproveFinitePrivateGrantInput {
                    verified_email: "owner@finite.vip".to_string(),
                    workos_user_id: Some("user_workos_owner".to_string()),
                    limit_profile_id: None,
                    now: Some(LATER.to_string()),
                })
                .await
                .unwrap();
            db.issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id: grant.id.clone(),
                raw_key: "fpk_live_overview_key_material_00000004".to_string(),
                project_id: Some(project_id.clone()),
                agent_runtime_id: Some(runtime_id.clone()),
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap();

            let overviews = db.admin_runtime_overviews().await.unwrap();
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
            assert_eq!(
                overview.runtime_capabilities,
                Some(*kata_runtime_capabilities().v1())
            );
        })
        .await;
    }

    #[tokio::test]
    async fn explicit_kata_upgrade_binds_compatible_artifact_and_commits_actual_facts_atomically() {
        with_isolated_postgres(|db| async move {
            let launch_code = issue_test_launch_code(&db).await;
            promote_runtime_artifact(&db).await;
            let requested = db
                .request_agent_creation_configured(
                    RequestAgentCreationInput {
                        verified_email: "upgrade@finite.vip".to_string(),
                        workos_user_id: "workos-upgrade".to_string(),
                        display_name: "Upgrade Agent".to_string(),
                        launch_code: launch_code.clone(),
                        idempotency_key: "upgrade-agent".to_string(),
                        now: Some(NOW.to_string()),
                    },
                    AgentCreationConfiguration {
                        placement: Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
                        requested_hosting_tier: None,
                        profile_picture_url: None,
                        owner_chat_account_id: None,
                    },
                ).await
                .unwrap();
            db
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "kata-runner".to_string(),
                    source_host_id: None,
                    lease_token: "launch-lease".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: Some(LATER.to_string()),
                }).await
                .unwrap()
                .unwrap();
            let completed = db
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: requested.request.id,
                    runner_id: "kata-runner".to_string(),
                    lease_token: "launch-lease".to_string(),
                    source_host_id: "oslo-host-1".to_string(),
                    source_machine_id: "finite-kata-upgrade".to_string(),
                    runtime_artifact_id: Some("artifact-v1".to_string()),
                    state_schema_version: None,
                    provider_runtime_handle: None,
                    contact_endpoint: Some("http://127.0.0.1:41001/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: None,
                    hostname: None,
                    runtime_host: Some("http://127.0.0.1:41001".to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: vec!["http://127.0.0.1:41001/contact".to_string()],
                    now: Some("2026-05-25T13:02:00Z".to_string()),
                }).await
                .unwrap();
            let runtime_id = completed.request.agent_runtime_id.unwrap();
            db.exec(&format!(
                "INSERT INTO runtime_relay_credentials \
                 (agent_runtime_id, token_hash, created_at, updated_at) \
                 VALUES ('{runtime_id}', 'existing-relay-token-hash', \
                 '2026-05-25T13:02:00Z', '2026-05-25T13:02:00Z')"
            ))
            .await;
            promote_runtime_artifact_version(
                &db,
                "artifact-mutable",
                "ghcr.io/finitecomputer/agent-runtime:latest",
                "mutable",
                "db-v1",
                "2026-05-25T13:02:10Z",
            ).await;
            let mutable = db
                .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    admin_workos_user_id: "workos-admin".to_string(),
                    project_id: requested.project.id.clone(),
                    target_runtime_artifact_id: "artifact-mutable".to_string(),
                    now: Some("2026-05-25T13:02:20Z".to_string()),
                }).await
                .unwrap_err();
            assert!(matches!(mutable, CoreError::RuntimeUpgradeUnsupported));
            promote_runtime_artifact_version(
                &db,
                "artifact-incompatible",
                &format!(
                    "ghcr.io/finitecomputer/agent-runtime:future@sha256:{}",
                    "c".repeat(64)
                ),
                "future",
                "db-v2",
                "2026-05-25T13:02:30Z",
            ).await;
            let incompatible = db
                .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    admin_workos_user_id: "workos-admin".to_string(),
                    project_id: requested.project.id.clone(),
                    target_runtime_artifact_id: "artifact-incompatible".to_string(),
                    now: Some("2026-05-25T13:02:40Z".to_string()),
                }).await
                .unwrap_err();
            assert!(matches!(
                incompatible,
                CoreError::RuntimeUpgradeStateSchemaIncompatible
            ));
            promote_runtime_artifact_version(
                &db,
                "artifact-v2",
                &format!(
                    "ghcr.io/finitecomputer/agent-runtime:v2@sha256:{}",
                    "b".repeat(64)
                ),
                "v2",
                "db-v1",
                "2026-05-25T13:03:00Z",
            ).await;
            db.exec("UPDATE runtime_artifacts SET recover_known_good_chat = true WHERE id = 'artifact-v2'")
                .await;

            let changed_binding = db
                .admin_request_runtime_upgrade_exact(AdminRuntimeUpgradeExactInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    admin_workos_user_id: "workos-admin".to_string(),
                    project_id: requested.project.id.clone(),
                    expected_agent_runtime_id: "runtime-replaced-after-plan".to_string(),
                    expected_source_host_id: "oslo-host-1".to_string(),
                    expected_source_machine_id: "finite-kata-upgrade".to_string(),
                    target_runtime_artifact_id: "artifact-v2".to_string(),
                    now: Some("2026-05-25T13:03:30Z".to_string()),
                }).await
                .unwrap_err();
            assert!(matches!(changed_binding, CoreError::RuntimeSpecMismatch));
            assert!(db.all_runtime_control_requests().await.is_empty());

            let upgrade = db
                .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    admin_workos_user_id: "workos-admin".to_string(),
                    project_id: requested.project.id.clone(),
                    target_runtime_artifact_id: "artifact-v2".to_string(),
                    now: Some("2026-05-25T13:04:00Z".to_string()),
                }).await
                .unwrap();
            assert_eq!(upgrade.kind, RuntimeControlKind::Upgrade);
            assert_eq!(
                upgrade.target_runtime_artifact_id.as_deref(),
                Some("artifact-v2")
            );
            let conflicting_stop = db
                .request_runtime_stop(RequestRuntimeStopInput {
                    verified_email: "upgrade@finite.vip".to_string(),
                    workos_user_id: "workos-upgrade".to_string(),
                    project_id: requested.project.id.clone(),
                    now: Some("2026-05-25T13:04:30Z".to_string()),
                }).await
                .unwrap_err();
            assert!(matches!(
                conflicting_stop,
                CoreError::RuntimeControlOperationConflict
            ));
            db.exec("UPDATE runtime_artifacts SET retired_at = '2026-05-25T13:04:40Z' WHERE id = 'artifact-v2'")
                .await;
            // A second, healthy Runtime on the same Project, copied from the
            // first so it shares its artifact and capabilities.
            let healthy_runtime_id = "runtime-healthy-behind-poison".to_string();
            db.exec(&format!(
                "INSERT INTO agent_runtimes \
                 (id, project_id, source_host_id, source_machine_id, source_import_key, \
                  runtime_artifact_id, state_schema_version, host_facts, \
                  created_at, updated_at, placement_runner_class, runtime_resource_class, \
                  runtime_capabilities) \
                 SELECT '{healthy_runtime_id}', project_id, source_host_id, \
                        'healthy-behind-poison', \
                        source_host_id || '/healthy-behind-poison', \
                        runtime_artifact_id, state_schema_version, host_facts, \
                        created_at, updated_at, placement_runner_class, \
                        runtime_resource_class, runtime_capabilities \
                 FROM agent_runtimes WHERE id = '{runtime_id}'"
            ))
            .await;
            let user_id = db
                .user_by_email("upgrade@finite.vip")
                .await
                .expect("owner exists")
                .id;
            let healthy_request_id = "runtime_ctl_healthy_behind_poison".to_string();
            let healthy_project_id = requested.project.id.clone();
            db.exec(&format!(
                "INSERT INTO runtime_control_requests \
                 (id, project_id, agent_runtime_id, source_host_id, source_machine_id, \
                  requested_by_user_id, kind, status, created_at, updated_at) \
                 VALUES ('{healthy_request_id}', '{healthy_project_id}', \
                 '{healthy_runtime_id}', 'oslo-host-1', 'healthy-behind-poison', \
                 '{user_id}', 'restart', 'requested', \
                 '2026-05-25T13:04:45Z', '2026-05-25T13:04:45Z')"
            ))
            .await;
            let healthy_lease = db
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: "kata-runner".to_string(),
                    lease_token: "must-not-stick".to_string(),
                    lease_seconds: Some(300),
                    source_host_id: Some("oslo-host-1".to_string()),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: Some("2026-05-25T13:04:50Z".to_string()),
                }).await
                .unwrap()
                .expect("poisoned upgrade must not starve the next healthy request");
            assert_eq!(healthy_lease.request.id, healthy_request_id);
            assert_eq!(
                db.runtime_control_request(&upgrade.id).await.unwrap().status,
                RuntimeControlRequestStatus::Failed
            );
            assert!(
                db.runtime_control_request(&upgrade.id).await.unwrap()
                    .failure_message
                    .as_deref()
                    .unwrap_or_default()
                    .contains("retired")
            );
            db.exec("UPDATE runtime_artifacts SET retired_at = NULL WHERE id = 'artifact-v2'")
                .await;
            // An N-1 request that predates persisted runtime specs.
            db.exec(&format!(
                "UPDATE agent_creation_requests SET runtime_spec = NULL \
                 WHERE agent_runtime_id = '{runtime_id}'"
            ))
            .await;
            let upgrade = db
                .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    admin_workos_user_id: "workos-admin".to_string(),
                    project_id: requested.project.id.clone(),
                    target_runtime_artifact_id: "artifact-v2".to_string(),
                    now: Some("2026-05-25T13:04:55Z".to_string()),
                }).await
                .unwrap();
            let refreshed_secret_references = vec!["FAL_KEY".to_string(), "XAI_API_KEY".to_string()];
            let lease = with_runtime_config(&db, &BTreeMap::new(), &refreshed_secret_references).lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                        runner_id: "kata-runner".to_string(),
                        lease_token: "upgrade-lease".to_string(),
                        lease_seconds: Some(300),
                        source_host_id: Some("oslo-host-1".to_string()),
                        runner_capacity: Some(RunnerLeaseCapacity {
                            runner_classes: vec![RunnerClass::Kata],
                            runtime_capabilities: Some(kata_runtime_capabilities()),
                            ..RunnerLeaseCapacity::default()
                        }),
                        now: Some("2026-05-25T13:05:00Z".to_string()),
                    }).await
                .unwrap()
                .unwrap();
            assert_eq!(
                lease
                    .target_runtime_artifact
                    .as_ref()
                    .map(|artifact| artifact.id.as_str()),
                Some("artifact-v2")
            );
            let synthesized_upgrade_spec = lease.runtime_spec.as_ref().unwrap();
            assert_eq!(
                runtime_spec_v1(synthesized_upgrade_spec).durable_state_id,
                "finite-kata-upgrade",
                "legacy synthesis preserves the source-machine /data directory"
            );
            assert_eq!(
                runtime_spec_v1(synthesized_upgrade_spec).operation_id,
                upgrade.id
            );
            assert_eq!(
                runtime_spec_v1(synthesized_upgrade_spec).secret_references,
                vec!["FINITE_PRIVATE_API_KEY", "FAL_KEY", "XAI_API_KEY"]
            );

            let mismatch = db
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: upgrade.id.clone(),
                    runner_id: "kata-runner".to_string(),
                    lease_token: "upgrade-lease".to_string(),
                    runtime_artifact_id: Some("artifact-v1".to_string()),
                    state_schema_version: Some("db-v1".to_string()),
                    runtime_capabilities: None,
                    runtime_host: Some("http://127.0.0.1:41002".to_string()),
                    published_app_urls: Some(vec!["http://127.0.0.1:41002/contact".to_string()]),
                    retirement_snapshot: None,
                    now: Some("2026-05-25T13:06:00Z".to_string()),
                }).await
                .unwrap_err();
            assert!(matches!(
                mismatch,
                CoreError::RuntimeUpgradeCompletionMismatch
            ));
            assert_eq!(
                db.runtime_control_request(&upgrade.id).await.unwrap().status,
                RuntimeControlRequestStatus::Launching
            );
            assert_eq!(
                db.agent_runtime(&runtime_id).await.unwrap()
                    .runtime_artifact_id
                    .as_deref(),
                Some("artifact-v1")
            );

            db.exec("UPDATE runtime_artifacts SET retired_at = '2026-05-25T13:06:30Z' WHERE id = 'artifact-v2'")
                .await;
            with_runtime_config(&db, &BTreeMap::new(), &refreshed_secret_references).complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                        request_id: upgrade.id.clone(),
                        runner_id: "kata-runner".to_string(),
                        lease_token: "upgrade-lease".to_string(),
                        runtime_artifact_id: Some("artifact-v2".to_string()),
                        state_schema_version: Some("db-v1".to_string()),
                        runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                            RuntimeCapabilitiesV1 {
                                recover_known_good_chat: true,
                                ..*kata_runtime_capabilities().v1()
                            },
                        )),
                        runtime_host: Some("http://127.0.0.1:41002".to_string()),
                        published_app_urls: Some(vec!["http://127.0.0.1:41002/contact".to_string()]),
                        retirement_snapshot: None,
                        now: Some("2026-05-25T13:06:40Z".to_string()),
                    }).await
                .unwrap();
            let runtime = &db.agent_runtime(&runtime_id).await.unwrap();
            assert_eq!(runtime.runtime_artifact_id.as_deref(), Some("artifact-v2"));
            assert_eq!(runtime.source_machine_id, "finite-kata-upgrade");
            assert_eq!(
                runtime.contact_endpoint.as_deref(),
                Some("http://127.0.0.1:41002/contact")
            );
            assert_eq!(runtime.host_facts.runtime_host, "http://127.0.0.1:41002");
            assert!(
                runtime
                    .runtime_capabilities
                    .as_ref()
                    .unwrap()
                    .v1()
                    .recover_known_good_chat
            );
            assert!(!db.query_json(
                "SELECT to_jsonb(t) FROM runtime_relay_credentials t \
                 WHERE t.agent_runtime_id = $1",
                &[&runtime_id],
            )
            .await
            .is_empty());
            let requests = db.all_agent_creation_requests().await;
            let persisted_spec = requests
                .iter()
                .find(|request| request.agent_runtime_id.as_deref() == Some(runtime_id.as_str()))
                .and_then(|request| request.runtime_spec.as_ref())
                .unwrap();
            assert_eq!(
                runtime_spec_v1(persisted_spec).secret_references,
                vec!["FINITE_PRIVATE_API_KEY", "FAL_KEY", "XAI_API_KEY"]
            );
            assert!(
                db.all("project_runtime_links").await.iter()
                    .any(|link| { link["agent_runtime_id"] == runtime_id.as_str() && link["active"] == true })
            );
            assert!(db.all_finite_private_api_keys().await.iter().all(|key| {
                key.agent_runtime_id.as_deref() != Some(runtime_id.as_str())
                    || key.status == FinitePrivateApiKeyStatus::Active
            }));
            assert!(
                db.finite_private_admin_audit_events().await.unwrap().iter()
                    .any(|event| {
                        event.action == "runtime.admin_upgrade"
                            && event.metadata["targetRuntimeArtifactId"] == "artifact-v2"
                    })
            );

            let recovery = db
                .request_runtime_recover_known_good_chat(RequestRuntimeRecoverKnownGoodChatInput {
                    verified_email: "upgrade@finite.vip".to_string(),
                    workos_user_id: "workos-upgrade".to_string(),
                    project_id: requested.project.id,
                    now: Some("2026-05-25T13:07:00Z".to_string()),
                }).await
                .unwrap();
            let recovery_capabilities = RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                recover_known_good_chat: true,
                ..*kata_runtime_capabilities().v1()
            });
            let recovery_lease = db
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: "kata-runner".to_string(),
                    lease_token: "recovery-lease".to_string(),
                    lease_seconds: Some(300),
                    source_host_id: Some("oslo-host-1".to_string()),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        runtime_capabilities: Some(recovery_capabilities),
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: Some("2026-05-25T13:07:01Z".to_string()),
                }).await
                .unwrap()
                .unwrap();
            assert_eq!(recovery_lease.request.id, recovery.id);
            let recovery_spec = runtime_spec_v1(recovery_lease.runtime_spec.as_ref().unwrap());
            assert_eq!(
                recovery_spec.boot_intent,
                RuntimeBootIntent::RecoverKnownGood
            );
            assert_eq!(recovery_spec.runtime_artifact_id, "artifact-v2");
        })
        .await;
    }

    #[tokio::test]
    async fn runtime_upgrade_rejects_non_kata_runtime_before_leasing() {
        with_isolated_postgres(|db| async move {
            promote_runtime_artifact(&db).await;
            let runtime_id = complete_self_serve_agent(
                &db,
                "not-kata@finite.vip",
                "workos-not-kata",
                "not-kata",
                "not-kata-runtime",
                "artifact-v1",
                LATER,
            )
            .await;
            promote_runtime_artifact_version(
                &db,
                "artifact-mutable",
                "ghcr.io/finitecomputer/agent-runtime:latest",
                "mutable",
                "db-v1",
                "2026-05-25T13:03:00Z",
            )
            .await;
            let project_id = db
                .agent_runtime(&runtime_id)
                .await
                .unwrap()
                .project_id
                .clone();
            let error = db
                .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                    admin_verified_email: "admin@finite.vip".to_string(),
                    admin_workos_user_id: "workos-admin".to_string(),
                    project_id,
                    target_runtime_artifact_id: "artifact-mutable".to_string(),
                    now: Some("2026-05-25T13:04:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(error, CoreError::RuntimeUpgradeUnsupported));
            assert!(db.all_runtime_control_requests().await.is_empty());
        })
        .await;
    }

    async fn promote_runtime_artifact(db: &TestDb) {
        promote_runtime_artifact_version(
            db,
            "artifact-v1",
            &format!(
                "ghcr.io/finitecomputer/agent-runtime:v1@sha256:{}",
                "a".repeat(64)
            ),
            "v1",
            "db-v1",
            NOW,
        )
        .await;
    }

    async fn promote_runtime_artifact_version(
        db: &TestDb,
        id: &str,
        reference: &str,
        version_label: &str,
        state_schema_version: &str,
        now: &str,
    ) {
        db.upsert_runtime_artifact(UpsertRuntimeArtifactInput {
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
            recover_known_good_chat: false,
            promoted: true,
            now: Some(now.to_string()),
        })
        .await
        .unwrap();
    }

    async fn complete_self_serve_agent(
        db: &TestDb,
        email: &str,
        workos_user_id: &str,
        idempotency_key: &str,
        source_machine_id: &str,
        artifact_id: &str,
        now: &str,
    ) -> String {
        let launch_code = issue_test_launch_code(db).await;
        let requested = db
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: email.to_string(),
                workos_user_id: workos_user_id.to_string(),
                display_name: source_machine_id.to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: idempotency_key.to_string(),
                now: Some(NOW.to_string()),
            })
            .await
            .unwrap();
        let lease = db
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                source_host_id: None,
                lease_token: format!("lease-{source_machine_id}"),
                lease_seconds: Some(300),
                runner_capacity: None,
                now: Some(LATER.to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        let completed = db
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: requested.request.id,
                runner_id: "runner-oslo-1".to_string(),
                lease_token: format!("lease-{source_machine_id}"),
                source_host_id: "oslo-host-1".to_string(),
                source_machine_id: source_machine_id.to_string(),
                runtime_artifact_id: Some(artifact_id.to_string()),
                state_schema_version: None,
                provider_runtime_handle: None,
                contact_endpoint: None,
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: None,
                hostname: None,
                runtime_host: Some("oslo-host-1".to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                now: Some(now.to_string()),
            })
            .await
            .unwrap();
        assert_eq!(lease.project.id, completed.project.id);
        completed.request.agent_runtime_id.unwrap()
    }

    fn kata_runtime_capabilities() -> RuntimeCapabilitiesEnvelope {
        RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
            restart: true,
            recover_known_good_chat: false,
            runtime_upgrade: true,
            stop: true,
            runtime_retirement: false,
        })
    }

    /// Stage the production anomaly the retired-offboard repair exists for: a
    /// destroy control that stored its verified retirement receipt while the
    /// offboarding transaction never ran, leaving the runtime link active with
    /// no compute behind it. Returns (project_id, agent_runtime_id, destroy
    /// request_id).
    async fn stage_retired_offboard_anomaly(
        db: &TestDb,
        email: &str,
        workos_user_id: &str,
        idempotency_key: &str,
        source_machine_id: &str,
    ) -> (String, String, String) {
        promote_runtime_artifact(db).await;
        let runtime_id = complete_self_serve_agent(
            db,
            email,
            workos_user_id,
            idempotency_key,
            source_machine_id,
            "artifact-v1",
            "2026-07-21T20:00:00Z",
        )
        .await;
        let project_id = db
            .agent_runtime(&runtime_id)
            .await
            .unwrap()
            .project_id
            .clone();
        let retirement_capable =
            serde_json::to_string(&RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                runtime_retirement: true,
                ..*kata_runtime_capabilities().v1()
            }))
            .unwrap();
        db.exec(&format!(
            "UPDATE agent_runtimes SET runtime_capabilities = '{retirement_capable}'::jsonb \
             WHERE id = '{runtime_id}'"
        ))
        .await;
        let destroy = db
            .request_runtime_destroy(RequestRuntimeDestroyInput {
                verified_email: email.to_string(),
                workos_user_id: workos_user_id.to_string(),
                project_id: project_id.clone(),
                now: Some("2026-07-21T20:01:00Z".to_string()),
            })
            .await
            .unwrap();
        let lease = db
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: "runner-oslo-1".to_string(),
                lease_token: format!("destroy-lease-{source_machine_id}"),
                lease_seconds: Some(60),
                source_host_id: Some("oslo-host-1".to_string()),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                        RuntimeCapabilitiesV1 {
                            runtime_retirement: true,
                            ..*kata_runtime_capabilities().v1()
                        },
                    )),
                    ..RunnerLeaseCapacity::default()
                }),
                now: Some("2026-07-21T20:01:30Z".to_string()),
            })
            .await
            .unwrap()
            .unwrap();
        let spec = runtime_spec_v1(lease.runtime_spec.as_ref().unwrap());
        db.exec(&format!(
            "UPDATE runtime_control_requests \
             SET status = 'stopped', lease_token = NULL, lease_expires_at = NULL, \
                 completed_at = CURRENT_TIMESTAMP \
             WHERE id = '{}'",
            destroy.id
        ))
        .await;
        db.exec(&format!(
            "INSERT INTO runtime_retirement_snapshots (
               request_id, project_id, agent_runtime_id, durable_state_id,
               runtime_artifact_id, schema_version, backend, locator,
               zip_bytes, zip_sha256, manifest_sha256, created_at,
               verified_at, recovery_authority_id, retention_policy, stored_at
             ) VALUES (
               '{}', '{}', '{}', '{}',
               '{}', 'runtime_retirement_snapshot.v1', 'borg', '{}',
               8192, '{}', '{}', '2026-07-21T20:02:00Z',
               '2026-07-21T20:03:00Z', 'finite-assisted-test',
               'indefinite_until_purge', CURRENT_TIMESTAMP
             )",
            destroy.id,
            project_id,
            runtime_id,
            spec.durable_state_id,
            spec.runtime_artifact_id,
            runtime_retirement_archive_locator(&destroy.id),
            "a".repeat(64),
            "b".repeat(64),
        ))
        .await;
        (project_id, runtime_id, destroy.id)
    }
}
