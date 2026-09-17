mod account_email;
pub mod hosted_hermes;
pub mod runtime_credentials;
pub use account_email::{AccountEmailChangePreview, AccountEmailChangeRequest};

use crate::billing;
use crate::launch_codes::{
    IssueLaunchCodeBatchInput, IssuedLaunchCodeBatch, LaunchCodeBatch, LaunchCodeBatchDetails,
    LaunchCodeRecord, LaunchCodeStatus, RevokeLaunchCodeBatchInput, hash_launch_code,
    prepare_launch_code_batch,
};
use crate::{
    AdminArchiveUnrecoverableRuntimeInput, AdminAssignFinitePrivateLimitProfileInput,
    AdminIssueFinitePrivateFriendKeyInput, AdminIssuedFinitePrivateKey,
    AdminOffboardRetiredRuntimeInput, AdminResetFinitePrivateUsageWindowInput,
    AdminRevokeFinitePrivateApiKeyInput, AdminRotateFinitePrivateApiKeyInput,
    AdminRuntimeControlInput, AdminRuntimeOverview, AdminRuntimeRelocateExactInput,
    AdminRuntimeRetireExactInput, AdminRuntimeUpgradeExactInput, AdminRuntimeUpgradeInput,
    AgentCreationConfiguration, AgentCreationEntitlement, AgentCreationLease, AgentCreationRequest,
    AgentCreationRequestStatus, AgentRuntime, ApproveFinitePrivateGrantInput, BillingClass,
    BillingOverview, CORE_SCHEMA_SQL, CancelAgentCreationRequestInput,
    CompleteAgentCreationRequestInput, CompleteRuntimeControlRequestInput, CoreError, CoreResult,
    CoreUser, CustomerBillingAccount, CustomerOrganization, FINITE_PRIVATE_SECRET_REFERENCE,
    FailAgentCreationRequestInput, FailRuntimeControlRequestInput, FinitePrivateAdminAccount,
    FinitePrivateAdminAuditEvent, FinitePrivateAdminProject, FinitePrivateAdminState,
    FinitePrivateApiKey, FinitePrivateApiKeyStatus, FinitePrivateDailyResetResult,
    FinitePrivateGrant, FinitePrivateGrantStatus, FinitePrivateLimitProfile,
    FinitePrivateReservation, FinitePrivateReservationStatus, FinitePrivateUsageDecision,
    FinitePrivateUsageNotice, FinitePrivateUsageStatus, HostOwnedRuntimeFacts, HostingTier,
    IssueFinitePrivateApiKeyInput, IssueFinitePrivateFriendKeyInput, IssuedFinitePrivateFriendKey,
    LeaseAgentCreationRequestInput, LeaseRuntimeControlRequestInput, LinkStripeCustomerInput,
    LinkVerifiedUserInput, MAX_RUNTIME_HEALTH_REPORT_REASON_CHARS, OWNER_CHAT_NPUBS_ENV,
    OffboardingPhase, Project, ProjectMembershipRole, ProviderOperationEnvelope,
    ProviderOperationTransition, ProviderOperationTransitionRecord, ProviderOperationV1,
    ProvisionFinitePrivateRuntimeKeyInput, ProvisionFinitePrivateRuntimeKeyResult,
    RUNTIME_HEALTH_REPORT_MAX_INTERVAL_SECONDS, RUNTIME_HEALTH_REPORT_MIN_INTERVAL_SECONDS,
    RecordProviderOperationTransitionInput, RecordRuntimeHealthReportInput,
    RegisterAgentCreationRuntimeInput, RenewRuntimeControlRequestInput, RequestAgentCreationInput,
    RequestAgentCreationResult, RequestRuntimeDestroyInput,
    RequestRuntimeRecoverKnownGoodChatInput, RequestRuntimeRestartInput, RequestRuntimeStopInput,
    ReserveFinitePrivateUsageInput, ResetFinitePrivateUsageWindowInput,
    RetiredRuntimeOffboardReceipt, RetryRuntimeControlRequestInput, RevokeFinitePrivateApiKeyInput,
    RevokeFinitePrivateGrantInput, RotateFinitePrivateApiKeyInput, RuntimeArtifact,
    RuntimeBootIntent, RuntimeCapabilitiesEnvelope, RuntimeControlCompletion,
    RuntimeControlExpectedBinding, RuntimeControlKind, RuntimeControlLease, RuntimeControlRequest,
    RuntimeControlRequestStatus, RuntimeHealthProjection, RuntimeHealthReportAck,
    RuntimeHealthTarget, RuntimeHealthTargetList, RuntimeLifecycleStage, RuntimePlacement,
    RuntimeRelocationEnvelope, RuntimeRelocationV1, RuntimeRetirementSnapshot,
    RuntimeRetirementSnapshotReceipt, RuntimeSpecEnvelope, RuntimeSpecIdentity,
    RuntimeSummaryStatus, SettleFinitePrivateReservationInput,
    SettleFinitePrivateReservationResult, StoreErrorDetail, StoredRuntimeHealth,
    SyncStripeSubscriptionInput, UnrecoverableRuntimeArchiveReceipt, UpsertRuntimeArtifactInput,
    agent_creation_entitlement_id_for, append_provider_operation_transition,
    bound_runtime_capabilities_to_artifact, build_runtime_spec_v1, canonical_agent_email,
    chat_identity_id_for_user, current_time_iso, derive_runtime_summary_status,
    finite_private_api_key_id_for, finite_private_grant_id_for_user,
    generate_finite_private_api_key, hash_finite_private_api_key, merge_provider_runtime_handle,
    merge_runtime_capabilities, new_agent_creation_request_id, new_agent_runtime_id,
    new_customer_org_id, new_self_service_project_id, new_user_id, normalize_id_part,
    normalize_idempotency_key, normalize_owner_chat_account_id, normalize_owner_email,
    normalize_profile_picture_url, normalize_runtime_contact_endpoint, normalize_source_host_id,
    parse_agent_creation_request_status, parse_billing_class, parse_finite_private_api_key_status,
    parse_finite_private_grant_status, parse_finite_private_reservation_status, parse_hosting_tier,
    parse_offboarding_phase, parse_runner_class, parse_runtime_artifact_kind,
    parse_runtime_control_kind, parse_runtime_control_request_status,
    parse_runtime_lifecycle_stage, parse_runtime_resource_class, parse_time,
    parse_user_link_status, project_room_membership_id_for, project_runtime_health,
    project_runtime_link_id_for, provider_operation_allows_generic_failure,
    provider_operation_at_runtime_boundary, runtime_artifact_material_matches,
    runtime_artifact_reference_is_immutable_oci, runtime_lifecycle, runtime_operation_spec_v1,
    runtime_spec_secret_references, runtime_spec_v1, runtime_upgrade_contact_endpoint,
    runtime_upgrade_prelease_rejection_is_terminal, source_import_key, trim_to_option,
    valid_agent_npub, valid_sha256_hex, validate_runtime_capabilities_artifact_policy,
    validate_runtime_capabilities_policy, validate_runtime_relocation_registration,
    validate_runtime_retirement_snapshot_receipt, validate_runtime_spec_binding,
    validate_runtime_spec_environment,
};
use deadpool_postgres::{Manager, ManagerConfig, Object, Pool, RecyclingMethod, Transaction};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use time::Duration;
use time::format_description::well_known::Rfc3339;
use tokio_postgres::{GenericClient, NoTls, Row};
use tracing::Instrument;

const DEFAULT_POSTGRES_POOL_SIZE: usize = 8;

#[derive(Clone)]
pub struct CoreStore {
    pool: Pool,
    runtime_environment: Arc<BTreeMap<String, String>>,
    runtime_secret_references: Arc<Vec<String>>,
    /// When set, every write transaction rolls back instead of committing.
    ///
    /// A dry run executes the real SQL against real production rows and then
    /// discards the write, so the preview reflects the state the operator is
    /// actually about to change. Previewing against an empty store instead
    /// would report creations for rows that already exist and would fail every
    /// operation that looks up an existing row.
    dry_run: bool,
}

struct FinitePrivateAdminAuditInsert<'a> {
    action: &'a str,
    target_type: &'a str,
    target_id: &'a str,
    grant_id: Option<&'a str>,
    api_key_id: Option<&'a str>,
    /// Admin identity for operator-initiated actions; `None` means Core itself.
    actor: Option<&'a str>,
    metadata: Value,
    now: &'a str,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VisibleProject {
    pub project: Project,
    pub runtime: Option<AgentRuntime>,
    /// The runtime's standing readiness projected at read time; present
    /// exactly when `runtime` is.
    #[serde(default)]
    pub runtime_health: Option<RuntimeHealthProjection>,
    pub active_runtime_control: Option<RuntimeControlRequest>,
}

mod artifacts;
use artifacts::*;
mod billing_accounts;
use billing_accounts::*;
mod control_admin;
use control_admin::*;
mod control_completion;
use control_completion::*;
mod control_leases;
use control_leases::*;
mod control_requests;
use control_requests::*;
mod control_rows;
use control_rows::*;
mod creation_completion;
mod creation_failures;
mod creation_leases;
use creation_leases::*;
mod creation_requests;
use creation_requests::*;
mod creation_rows;
use creation_rows::*;
mod health;
use health::*;
mod identity_rows;
use identity_rows::*;
mod launch_code_rows;
use launch_code_rows::*;
mod launch_codes;
mod offboarding;
use offboarding::*;
mod private_admin;
use private_admin::*;
mod private_grants;
use private_grants::*;
mod private_keys;
use private_keys::*;
mod private_reservations;
mod private_rows;
use private_rows::*;
mod private_usage;
mod projects;
use projects::*;
mod provider_operations;
use provider_operations::*;
mod relocation;
use relocation::*;
mod retirement;
mod row_decoding;
use row_decoding::*;
mod runtime_rows;
pub(crate) use billing_accounts::{
    customer_org_exists, ensure_standard_agent_creation_entitlement_row,
};
pub(crate) use identity_rows::{ensure_personal_org_row, upsert_linked_user};
pub(crate) use row_decoding::optional_hosting_tier_column;
use runtime_rows::*;

impl CoreStore {
    pub async fn connect(database_url: &str) -> CoreResult<Self> {
        let config = database_url
            .parse()
            .map_err(|error| pool_config_error("invalid Postgres URL", error))?;
        let manager = Manager::from_config(
            config,
            NoTls,
            ManagerConfig {
                // Avoid adding a validation round trip to every store method.
                // Normal query/connection failures still surface through the
                // existing structured database error path.
                recycling_method: RecyclingMethod::Fast,
            },
        );
        let pool = Pool::builder(manager)
            .max_size(DEFAULT_POSTGRES_POOL_SIZE)
            .build()
            .map_err(|error| pool_config_error("cannot build Postgres pool", error))?;
        let store = Self {
            pool,
            runtime_environment: Arc::new(BTreeMap::new()),
            runtime_secret_references: Arc::new(Vec::new()),
            dry_run: false,
        };
        // Fail startup/retry at the same boundary as the former eager
        // connection. Deadpool otherwise opens its first connection lazily.
        let _ = store.connection().await?;
        Ok(store)
    }

    /// Connect a store whose writes are always rolled back.
    ///
    /// Reads still see committed production state, so a preview reports what
    /// the operation would really do.
    pub async fn connect_dry_run(database_url: &str) -> CoreResult<Self> {
        Ok(Self {
            dry_run: true,
            ..Self::connect(database_url).await?
        })
    }

    pub fn with_runtime_environment(
        mut self,
        runtime_environment: BTreeMap<String, String>,
    ) -> CoreResult<Self> {
        validate_runtime_spec_environment(&runtime_environment)?;
        self.runtime_environment = Arc::new(runtime_environment);
        Ok(self)
    }

    pub fn with_runtime_secret_references(
        mut self,
        runtime_secret_references: Vec<String>,
    ) -> CoreResult<Self> {
        runtime_spec_secret_references(&runtime_secret_references)?;
        self.runtime_secret_references = Arc::new(runtime_secret_references);
        Ok(self)
    }

    async fn connection(&self) -> CoreResult<Object> {
        self.pool.get().await.map_err(|error| {
            CoreError::Database(Box::new(StoreErrorDetail {
                message: format!("Postgres pool checkout failed: {error}"),
                ..StoreErrorDetail::default()
            }))
        })
    }

    /// Commit, or roll back when this store is in dry-run mode.
    ///
    /// Every write path in this impl ends here, so `--dry-run` cannot silently
    /// miss a mutation that a later method introduces.
    async fn finish(&self, tx: Transaction<'_>) -> CoreResult<()> {
        if self.dry_run {
            tx.rollback().await.map_err(store_error)
        } else {
            tx.commit().await.map_err(store_error)
        }
    }

    pub async fn migrate(&self) -> CoreResult<()> {
        let client = self.connection().await?;
        client
            .batch_execute(CORE_SCHEMA_SQL)
            .await
            .map_err(store_error)
    }
}

struct LockedLaunchCode {
    record: LaunchCodeRecord,
    hosting_tier: Option<HostingTier>,
    target_source_host_id: Option<String>,
}

/// How an `agent_creation_requests` INSERT reacts to a conflict.
///
/// `UpsertById` rewrites an existing row with the same surrogate id (the
/// historical whole-row upsert). `SingleFlight` inserts a brand-new row and
/// reports `false` instead when any unique constraint — for a relocation the
/// partial index `agent_creation_requests_one_active_relocation_per_runtime` —
/// was already satisfied by a concurrent committed attempt, leaving the caller
/// to decide between reuse and refusal inside the same transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentCreationInsertConflict {
    UpsertById,
    SingleFlight,
}

const RUNTIME_CONTROL_REQUEST_COLUMNS: &str = "id, project_id, agent_runtime_id, source_host_id,
    source_machine_id, requested_by_user_id, kind, target_runtime_artifact_id,
    status, failure_stage, runner_id, lease_token,
    core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message, core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at, core_rfc3339(completed_at) AS completed_at";

/// Apply the completed runtime status to both the runtime's host facts and its
/// status snapshot (if one exists), touching only that runtime's two rows.
struct RuntimeUpgradeCompletion {
    runtime_artifact_id: String,
    state_schema_version: String,
    runtime_host: String,
    published_app_urls: Vec<String>,
    contact_endpoint: String,
    runtime_spec: Option<RuntimeSpecEnvelope>,
    runtime_capabilities: Option<RuntimeCapabilitiesEnvelope>,
}

/// What happens to the standing-health attribution pin when a completion
/// brings compute up.
enum HealthPin {
    /// Same runtime, same principal (restart, recovery, upgrade).
    Keep,
    /// A new incarnation whose principal the completion knows (a launch that
    /// verified it, or a relocation's expected principal); `None` when the
    /// completing runner did not say, in which case the first report pins.
    Seed(Option<String>),
}

/// RFC3339 rendering for a TIMESTAMPTZ column so stored strings round-trip
/// through `parse_time` (the Finite Private timestamps are parsed, not just
/// echoed).
fn rfc3339_col(expr: &str) -> String {
    format!("core_rfc3339({expr})")
}

fn json_column<T: DeserializeOwned>(row: &Row, name: &str) -> CoreResult<T> {
    let value: Value = row.get(name);
    serde_json::from_value(value).map_err(json_error)
}

fn optional_json_column(row: &Row, name: &str) -> CoreResult<Option<Value>> {
    Ok(row.get(name))
}

fn pool_config_error(context: &str, error: impl std::fmt::Display) -> CoreError {
    CoreError::Database(Box::new(StoreErrorDetail {
        message: format!("{context}: {error}"),
        ..StoreErrorDetail::default()
    }))
}

/// Convert a Postgres error into a structured `CoreError::Database`, preserving
/// the `as_db_error()` fields (SQLSTATE code, constraint, table, column, DETAIL)
/// that `error.to_string()` used to flatten into the useless string "db error".
/// The detail is log-only; the user-facing message stays generic.
pub(crate) fn store_error(error: tokio_postgres::Error) -> CoreError {
    if let Some(db) = error.as_db_error() {
        CoreError::Database(Box::new(StoreErrorDetail {
            message: db.message().to_string(),
            code: Some(db.code().code().to_string()),
            constraint: db.constraint().map(str::to_string),
            table: db.table().map(str::to_string),
            column: db.column().map(str::to_string),
            detail: db.detail().map(str::to_string),
        }))
    } else {
        // Connection/protocol errors have no DbError payload; keep the full
        // message for the logs but still return the generic user surface.
        CoreError::Database(Box::new(StoreErrorDetail {
            message: error.to_string(),
            ..StoreErrorDetail::default()
        }))
    }
}

fn json_error(error: serde_json::Error) -> CoreError {
    CoreError::Database(Box::new(StoreErrorDetail {
        message: format!("failed to (de)serialize a stored row: {error}"),
        ..StoreErrorDetail::default()
    }))
}

#[cfg(test)]
mod test_rows;
#[cfg(test)]
mod tests;
