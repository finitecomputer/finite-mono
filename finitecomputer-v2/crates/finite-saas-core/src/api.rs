mod hosted_access;
#[cfg(test)]
mod hosted_browser_proof;
use crate::auth::{CoreAuth, VerifiedRunnerCredential, WorkosAuthError};
use crate::hosted_hermes::{HostedHermesLocation, HostedHermesOrigins};
use crate::launch_codes::{
    IssueLaunchCodeBatchInput, LaunchCodeBatchDetails, RevokeLaunchCodeBatchInput,
};
use crate::store::{AccountEmailChangePreview, AccountEmailChangeRequest};
use crate::store::{CoreStore, VisibleProject};
use crate::{
    AdminAssignFinitePrivateLimitProfileInput, AdminIssueFinitePrivateFriendKeyInput,
    AdminIssuedFinitePrivateKey, AdminResetFinitePrivateUsageWindowInput,
    AdminRevokeFinitePrivateApiKeyInput, AdminRotateFinitePrivateApiKeyInput,
    AdminRuntimeControlInput, AdminRuntimeOverview, AdminRuntimeUpgradeInput,
    AgentCreationConfiguration, AgentCreationLease, AgentCreationRequest, AgentRuntime,
    BillingOverview, CancelAgentCreationRequestInput, CompleteAgentCreationRequestInput,
    CompleteRuntimeControlRequestInput, CoreError, CustomerBillingAccount,
    FailAgentCreationRequestInput, FailRuntimeControlRequestInput, FinitePrivateAdminAuditEvent,
    FinitePrivateAdminState, FinitePrivateApiKey, FinitePrivateDailyResetResult,
    FinitePrivateGrant, FinitePrivateRequestDiagnostic, FinitePrivateSettlementKind,
    FinitePrivateUsageDecision, FinitePrivateUsageStatus, HostingTier,
    IssueFinitePrivateApiKeyInput, LeaseAgentCreationRequestInput, LeaseRuntimeControlRequestInput,
    LinkStripeCustomerInput, LinkStripeCustomerRequest, LinkVerifiedUserInput, Project,
    ProviderOperationEnvelope, ProviderOperationTransition, ProviderRuntimeHandleEnvelope,
    ProvisionFinitePrivateRuntimeKeyInput, ProvisionFinitePrivateRuntimeKeyResult,
    RecordFinitePrivateRequestDiagnosticInput, RecordProviderOperationTransitionInput,
    RecordRuntimeHealthReportInput, RegisterAgentCreationRuntimeInput,
    RenewRuntimeControlRequestInput, RequestAgentCreationInput, RequestAgentCreationResult,
    RequestRuntimeRecoverKnownGoodChatInput, RequestRuntimeRestartInput,
    ReserveFinitePrivateUsageInput, ResetFinitePrivateUsageWindowInput,
    RetryRuntimeControlRequestInput, RevokeFinitePrivateApiKeyInput, RevokeFinitePrivateGrantInput,
    RotateFinitePrivateApiKeyInput, RunnerLeaseCapacity, RuntimeArtifact, RuntimeArtifactKind,
    RuntimeCapabilitiesEnvelope, RuntimeCapabilitiesV1, RuntimeHealthProjection,
    RuntimeHealthReportAck, RuntimeHealthReportRequest, RuntimeHealthStatus,
    RuntimeHealthTargetList, RuntimePlacement, RuntimeSummaryStatus,
    SettleFinitePrivateReservationInput, SettleFinitePrivateReservationResult,
    SyncStripeSubscriptionInput, SyncStripeSubscriptionRequest, UpsertRuntimeArtifactInput,
    derive_runtime_summary_status, normalize_owner_email, normalize_runtime_contact_endpoint,
    normalize_source_host_id,
};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
pub use hosted_access::runtime_router;
use hosted_access::{hosted_access, hosted_route_targets, hosted_session, set_hosted_access};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::env;
use subtle::ConstantTimeEq;
mod errors;
use errors::*;
mod requests;
pub use requests::*;
mod views;
pub use views::*;
mod runner;
use runner::*;
mod finite_private;
use finite_private::*;
mod admin;
use admin::*;
mod identity;
use identity::*;
mod runtime_routes;
use runtime_routes::*;
mod billing;
use billing::*;
mod runtime_lifecycle;
use runtime_lifecycle::*;
mod authorization;
use authorization::*;
#[cfg(test)]
mod tests;

const SERVICE_AUTH_HEADER: &str = "authorization";

const WORKOS_USER_ID_HEADER: &str = "x-finite-workos-user-id";

const WORKOS_EMAIL_HEADER: &str = "x-finite-workos-email";

const WORKOS_EMAIL_VERIFIED_HEADER: &str = "x-finite-workos-email-verified";

#[derive(Clone)]
pub struct CoreApiState {
    store: CoreStore,
    auth: CoreAuth,
    standard_stripe_price_id: Option<String>,
    agent_creation_placement: Option<RuntimePlacement>,
    /// First-use deployment gate. Persisting `kind = 'upgrade'` crosses the
    /// rollback boundary for Core generations that predate that value.
    runtime_upgrades_enabled: bool,
    /// New owner retirement requests are default-off independently of Runner
    /// capability so a rollout can be stopped without stranding in-flight work.
    runtime_retirement_enabled: bool,
    hosted_hermes_origins: HostedHermesOrigins,
    native_sessions: crate::hosted_hermes_session::NativeSessions,
}

pub fn router(store: CoreStore, auth: CoreAuth) -> Router {
    let runtime_upgrades_enabled = env::var("FC_CORE_ENABLE_RUNTIME_UPGRADES")
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE"));
    router_with_runtime_upgrades(store, auth, runtime_upgrades_enabled)
}

/// Build the public API with a trusted deployment-level placement override.
///
/// This exists for local/provider conformance environments such as devfinity.
/// User request bodies remain unable to select a Runner provider.
pub fn router_with_agent_creation_placement(
    store: CoreStore,
    auth: CoreAuth,
    agent_creation_placement: Option<RuntimePlacement>,
) -> Router {
    router_with_hosted_hermes_origins(
        store,
        auth,
        agent_creation_placement,
        HostedHermesOrigins::default(),
    )
}

/// Add trusted location metadata without enabling or granting hosted access.
pub fn router_with_hosted_hermes_origins(
    store: CoreStore,
    auth: CoreAuth,
    agent_creation_placement: Option<RuntimePlacement>,
    hosted_hermes_origins: HostedHermesOrigins,
) -> Router {
    let runtime_upgrades_enabled = env::var("FC_CORE_ENABLE_RUNTIME_UPGRADES")
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE"));
    let runtime_retirement_enabled = env::var("FC_CORE_ENABLE_RUNTIME_RETIREMENT")
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE"));
    router_with_runtime_upgrades_and_agent_creation_placement(
        store,
        auth,
        runtime_upgrades_enabled,
        runtime_retirement_enabled,
        agent_creation_placement,
        hosted_hermes_origins,
    )
}

pub fn router_with_runtime_upgrades(
    store: CoreStore,
    auth: CoreAuth,
    runtime_upgrades_enabled: bool,
) -> Router {
    let runtime_retirement_enabled = env::var("FC_CORE_ENABLE_RUNTIME_RETIREMENT")
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE"));
    router_with_runtime_features(
        store,
        auth,
        runtime_upgrades_enabled,
        runtime_retirement_enabled,
    )
}

pub fn router_with_runtime_features(
    store: CoreStore,
    auth: CoreAuth,
    runtime_upgrades_enabled: bool,
    runtime_retirement_enabled: bool,
) -> Router {
    router_with_runtime_upgrades_and_agent_creation_placement(
        store,
        auth,
        runtime_upgrades_enabled,
        runtime_retirement_enabled,
        None,
        HostedHermesOrigins::default(),
    )
}

fn router_with_runtime_upgrades_and_agent_creation_placement(
    store: CoreStore,
    auth: CoreAuth,
    runtime_upgrades_enabled: bool,
    runtime_retirement_enabled: bool,
    agent_creation_placement: Option<RuntimePlacement>,
    hosted_hermes_origins: HostedHermesOrigins,
) -> Router {
    let standard_stripe_price_id = optional_env_value("FC_CORE_STANDARD_STRIPE_PRICE_ID")
        .or_else(|| optional_env_value("STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID"));
    let state = CoreApiState {
        store,
        auth,
        standard_stripe_price_id,
        agent_creation_placement,
        runtime_upgrades_enabled,
        runtime_retirement_enabled,
        hosted_hermes_origins,
        native_sessions: Default::default(),
    };
    router_from_state(state)
}

fn router_from_state(state: CoreApiState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route(
            "/api/core/v1/agent-creation-requests/{request_id}/runtime-credential",
            post(provision_runtime_credential),
        )
        .route(
            "/api/core/v1/runtime-control-requests/{request_id}/runtime-credential",
            post(provision_upgrade_credential),
        )
        .route(
            "/api/core/v1/me/runtimes/{runtime_id}/hosted-access",
            get(hosted_access).put(set_hosted_access),
        )
        .route(
            "/api/core/v1/me/runtimes/{runtime_id}/hosted-hermes-session",
            post(hosted_session),
        )
        .route(
            "/api/core/v1/me/runtimes/{runtime_id}/hosted-hermes-location",
            get(hosted_hermes_location),
        )
        .route(
            "/api/core/v1/runtime-artifacts/{artifact_id}",
            get(runtime_artifact).put(upsert_runtime_artifact),
        )
        .route(
            "/api/core/v1/runtime-health-reports",
            post(report_runtime_health),
        )
        .route(
            "/api/core/v1/runtime-health-targets",
            get(runtime_health_targets),
        )
        .route(
            "/api/core/v1/hosted-hermes-route-targets",
            get(hosted_route_targets),
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
            "/api/core/v1/finite-private/usage",
            get(finite_private_usage_status_for_api_key),
        )
        .route(
            "/api/core/v1/finite-private/usage/reset",
            post(claim_finite_private_daily_reset_for_api_key),
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
        .route(
            "/internal/finite-private/v1/request-diagnostics",
            post(record_finite_private_request_diagnostic),
        )
        .route("/api/core/v1/admin/runtimes", get(admin_runtimes))
        .route(
            "/api/core/v1/admin/account-email-target",
            post(admin_account_email_target),
        )
        .route(
            "/api/core/v1/admin/account-email-changes/{action}",
            post(admin_account_email_change).get(admin_account_email_operation),
        )
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
        .route(
            "/api/core/v1/admin/finite-private/grants/{grant_id}/limit-profile",
            post(admin_assign_finite_private_limit_profile),
        )
        .route("/api/core/v1/me", get(me))
        .route("/api/core/v1/me/dashboard-summary", get(dashboard_summary))
        .route(
            "/api/core/v1/me/finite-private/usage",
            get(finite_private_usage_status_for_user),
        )
        .route(
            "/api/core/v1/me/finite-private/usage/reset",
            post(claim_finite_private_daily_reset_for_user),
        )
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
            "/api/core/v1/runtime-control-requests/{request_id}/renew",
            post(renew_runtime_control_request),
        )
        .route(
            "/api/core/v1/runtime-control-requests/{request_id}/retry",
            post(retry_runtime_control_request),
        )
        .route(
            "/api/core/v1/agent-creation-requests/{request_id}/complete",
            post(complete_agent_creation_request),
        )
        .route(
            "/api/core/v1/agent-creation-requests/{request_id}/provider-operation/transitions",
            post(record_provider_operation_transition),
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
        .with_state(state)
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
