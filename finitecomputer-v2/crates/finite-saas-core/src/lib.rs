//! Core domain contracts and the stable crate-root facade.

pub mod api;
pub mod auth;
pub mod billing;
pub mod hosted_hermes;
pub mod hosted_hermes_session;
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

mod accounts;
mod agent_creation;
mod error;
mod finite_private;
mod identity;
mod provider_operations;
mod runner_capacity;
mod runtime;
mod runtime_admin;
mod runtime_artifacts;
mod runtime_capabilities;
mod runtime_control;
mod runtime_health;
pub mod runtime_lifecycle;
mod runtime_relocation;
mod runtime_retirement;
mod runtime_spec;
mod schema;
mod wire;

pub(crate) use wire::wire_enum;

pub use accounts::{
    BillingClass, BillingOverview, ChatIdentity, CoreUser, CustomerOrganization,
    LinkVerifiedUserInput, Project, ProjectMembershipRole, ProjectRoomMembership,
    ProjectRuntimeLink, UserLinkStatus, parse_billing_class, parse_project_membership_role,
    parse_user_link_status,
};

pub use agent_creation::{
    AgentCreationConfiguration, AgentCreationEntitlement, AgentCreationLease, AgentCreationRequest,
    AgentCreationRequestStatus, CancelAgentCreationRequestInput, CompleteAgentCreationRequestInput,
    FailAgentCreationRequestInput, LeaseAgentCreationRequestInput,
    RegisterAgentCreationRuntimeInput, ReleaseLaunchHostInput, RequestAgentCreationInput,
    RequestAgentCreationResult, RetryTargetedLaunchCodeInput, parse_agent_creation_request_status,
};

pub use error::{CoreError, CoreResult, StoreErrorDetail};

pub use finite_private::{
    AdminAssignFinitePrivateLimitProfileInput, AdminIssueFinitePrivateFriendKeyInput,
    AdminIssuedFinitePrivateKey, AdminResetFinitePrivateUsageWindowInput,
    AdminRevokeFinitePrivateApiKeyInput, AdminRotateFinitePrivateApiKeyInput,
    ApproveFinitePrivateGrantInput, FINITE_PRIVATE_5X_LIMIT_PROFILE, FinitePrivateAdminAccount,
    FinitePrivateAdminAuditEvent, FinitePrivateAdminProject, FinitePrivateAdminState,
    FinitePrivateApiKey, FinitePrivateApiKeyStatus, FinitePrivateDailyResetResult,
    FinitePrivateGrant, FinitePrivateGrantStatus, FinitePrivateLimitProfile,
    FinitePrivateRequestDiagnostic, FinitePrivateReservation, FinitePrivateReservationStatus,
    FinitePrivateSettlementKind, FinitePrivateUsageDecision, FinitePrivateUsageError,
    FinitePrivateUsageNotice, FinitePrivateUsageStatus, IssueFinitePrivateApiKeyInput,
    IssueFinitePrivateFriendKeyInput, IssuedFinitePrivateFriendKey,
    ProvisionFinitePrivateRuntimeKeyInput, ProvisionFinitePrivateRuntimeKeyResult,
    RecordFinitePrivateRequestDiagnosticInput, ReserveFinitePrivateUsageInput,
    ResetFinitePrivateUsageWindowInput, RevokeFinitePrivateApiKeyInput,
    RevokeFinitePrivateGrantInput, RotateFinitePrivateApiKeyInput,
    SettleFinitePrivateReservationInput, SettleFinitePrivateReservationResult,
    generate_finite_private_api_key, parse_finite_private_api_key_status,
    parse_finite_private_grant_status, parse_finite_private_reservation_status,
    parse_finite_private_settlement_kind,
};

pub use identity::{
    canonical_agent_email, normalize_owner_email, normalize_source_host_id, source_import_key,
};

pub use provider_operations::{
    ProviderOperationEnvelope, ProviderOperationTransition, ProviderOperationTransitionRecord,
    ProviderOperationV1, ProviderRuntimeHandleEnvelope, ProviderRuntimeHandleV1,
    RecordProviderOperationTransitionInput,
};

pub use runner_capacity::{
    InFlightCapacityReservationEnvelope, InFlightCapacityReservationV1, RunnerLeaseCapacity,
};

pub use runtime::{
    AgentRuntime, HostOwnedRuntimeFacts, HostingTier, RunnerClass, RuntimePlacement,
    RuntimeResourceClass, RuntimeSummaryStatus, parse_hosting_tier, parse_runner_class,
    parse_runtime_resource_class, parse_runtime_summary_status,
};

pub use runtime_admin::{
    AdminArchiveUnrecoverableRuntimeInput, AdminOffboardRetiredRuntimeInput,
    AdminRuntimeControlInput, AdminRuntimeOverview, AdminRuntimeRetireExactInput,
    AdminRuntimeUpgradeExactInput, AdminRuntimeUpgradeInput, RetiredRuntimeOffboardReceipt,
    UnrecoverableRuntimeArchiveReceipt,
};

pub use runtime_artifacts::{
    RuntimeArtifact, RuntimeArtifactKind, UpsertRuntimeArtifactInput, parse_runtime_artifact_kind,
};

pub use runtime_capabilities::{RuntimeCapabilitiesEnvelope, RuntimeCapabilitiesV1};

pub use runtime_control::{
    CompleteRuntimeControlRequestInput, FailRuntimeControlRequestInput,
    LeaseRuntimeControlRequestInput, RenewRuntimeControlRequestInput, RequestRuntimeDestroyInput,
    RequestRuntimeRecoverKnownGoodChatInput, RequestRuntimeRestartInput, RequestRuntimeStopInput,
    RetryRuntimeControlRequestInput, RuntimeControlCompletion, RuntimeControlKind,
    RuntimeControlLease, RuntimeControlRequest, RuntimeControlRequestStatus, RuntimeLifecycleStage,
    RuntimeUpgradeCompletionFacts, parse_runtime_control_kind,
    parse_runtime_control_request_status, parse_runtime_lifecycle_stage,
};

pub use runtime_health::{
    MAX_RUNTIME_HEALTH_REPORT_REASON_CHARS, RUNTIME_HEALTH_REPORT_DEFAULT_INTERVAL_SECONDS,
    RUNTIME_HEALTH_REPORT_MAX_INTERVAL_SECONDS, RUNTIME_HEALTH_REPORT_MIN_INTERVAL_SECONDS,
    RUNTIME_HEALTH_REPORT_STALE_MULTIPLIER, RecordRuntimeHealthReportInput,
    RuntimeHealthProjection, RuntimeHealthReportAck, RuntimeHealthReportRequest,
    RuntimeHealthStatus, RuntimeHealthTarget, RuntimeHealthTargetList, StoredRuntimeHealth,
    derive_runtime_summary_status, parse_runtime_health_status, project_runtime_health,
};

pub use runtime_relocation::{
    AdminRuntimeRelocateExactInput, RUNTIME_RELOCATION_SCHEMA, RuntimeRelocationEnvelope,
    RuntimeRelocationV1,
};

pub use runtime_retirement::{
    OffboardingPhase, RUNTIME_RETIREMENT_BACKEND_BORG, RUNTIME_RETIREMENT_RETENTION_INDEFINITE,
    RUNTIME_RETIREMENT_SNAPSHOT_SCHEMA, RuntimeRetirementSnapshot,
    RuntimeRetirementSnapshotReceipt, parse_offboarding_phase, runtime_retirement_archive_locator,
};

pub use runtime_spec::{
    RuntimeBootIntent, RuntimeEndpointContractV1, RuntimeSpecEnvelope, RuntimeSpecV1,
};

pub use schema::{
    CORE_SCHEMA_SQL, RUNTIME_LIFECYCLE_REVERSE_REMAP_SQL, RUNTIME_UPGRADE_ROLLBACK_RESCUE_SQL,
};

pub(crate) use agent_creation::{
    DEFAULT_AGENT_CREATION_LEASE_SECONDS, MAX_AGENT_CREATION_LEASE_SECONDS,
};

pub(crate) use finite_private::{
    DEFAULT_FINITE_PRIVATE_BURST_LIMIT_UNITS, DEFAULT_FINITE_PRIVATE_BURST_WINDOW_SECONDS,
    DEFAULT_FINITE_PRIVATE_LIMIT_PROFILE, DEFAULT_FINITE_PRIVATE_WEEKLY_LIMIT_UNITS,
    FINITE_PRIVATE_5X_BURST_LIMIT_UNITS, FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS,
    finite_private_active_window, finite_private_allow_decision, finite_private_api_key_id_for,
    finite_private_begins_new_epoch, finite_private_denial, finite_private_grant_id_for_user,
    finite_private_limit_reached_message, finite_private_next_daily_reset_at,
    finite_private_reservation_id_for, finite_private_retry_after_label,
    finite_private_window_reset_at, hash_finite_private_api_key,
};

pub(crate) use identity::{
    agent_creation_entitlement_id_for, chat_identity_id_for_user, current_time_iso,
    generate_surrogate_id, id_from_parts, new_agent_creation_request_id, new_agent_runtime_id,
    new_customer_org_id, new_self_service_project_id, new_user_id, normalize_id_part,
    normalize_idempotency_key, normalize_owner_chat_account_id, normalize_profile_picture_url,
    parse_time, project_room_membership_id_for, project_runtime_link_id_for, trim_or_fallback,
    trim_to_option, valid_agent_npub, valid_sha256_hex,
};

pub(crate) use provider_operations::{
    append_provider_operation_transition, merge_provider_runtime_handle,
    provider_operation_allows_generic_failure, provider_operation_at_runtime_boundary,
};

pub(crate) use runner_capacity::{in_flight_capacity_bounds, in_flight_capacity_reservation};

pub(crate) use runtime::{normalize_runtime_contact_endpoint, runtime_upgrade_contact_endpoint};

pub(crate) use runtime_admin::RuntimeControlExpectedBinding;

pub(crate) use runtime_artifacts::{
    runtime_artifact_material_matches, runtime_artifact_reference_is_immutable_oci,
    runtime_upgrade_prelease_rejection_is_terminal,
};

pub(crate) use runtime_capabilities::{
    bound_runtime_capabilities_to_artifact, merge_runtime_capabilities,
    validate_runtime_capabilities_artifact_policy, validate_runtime_capabilities_policy,
};

pub(crate) use runtime_control::runtime_control_request_id_for;

pub(crate) use runtime_relocation::validate_runtime_relocation_registration;

pub(crate) use runtime_retirement::validate_runtime_retirement_snapshot_receipt;

pub(crate) use runtime_spec::{
    FINITE_PRIVATE_SECRET_REFERENCE, OWNER_CHAT_NPUBS_ENV, RuntimeSpecIdentity,
    build_runtime_spec_v1, runtime_operation_spec_v1, runtime_spec_secret_references,
    runtime_spec_v1, validate_runtime_spec_binding, validate_runtime_spec_environment,
};

#[cfg(test)]
mod domain_tests;
