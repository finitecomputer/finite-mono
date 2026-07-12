use crate::launch_codes::{
    IssueLaunchCodeBatchInput, IssuedLaunchCodeBatch, LaunchCodeBatch, LaunchCodeBatchDetails,
    LaunchCodeRecord, LaunchCodeStatus, RevokeLaunchCodeBatchInput, hash_launch_code,
    prepare_launch_code_batch,
};
use crate::{
    AdminIssueFinitePrivateFriendKeyInput, AdminIssuedFinitePrivateKey,
    AdminResetFinitePrivateUsageWindowInput, AdminRevokeFinitePrivateApiKeyInput,
    AdminRotateFinitePrivateApiKeyInput, AdminRuntimeControlInput, AdminRuntimeOverview,
    AdminRuntimeUpgradeInput, AgentCreationConfiguration, AgentCreationEntitlement,
    AgentCreationLease, AgentCreationRequest, AgentCreationRequestStatus, AgentRuntime,
    ApproveFinitePrivateGrantInput, ArchiveImportedProjectInput, BillingClass, BillingOverview,
    BillingSubscriptionStatus, BridgeCoreState, CORE_SCHEMA_SQL, CancelAgentCreationRequestInput,
    ClaimProjectImportsInput, ClaimProjectImportsResult, CompleteAgentCreationRequestInput,
    CompleteRuntimeControlRequestInput, CoreError, CoreResult, CoreUser, CustomerBillingAccount,
    CustomerOrganization, ExistingHostProjectImport, FINITE_PRIVATE_SECRET_REFERENCE,
    FailAgentCreationRequestInput, FailRuntimeControlRequestInput, FinitePrivateAdminAuditEvent,
    FinitePrivateAdminState, FinitePrivateApiKey, FinitePrivateApiKeyStatus, FinitePrivateGrant,
    FinitePrivateGrantStatus, FinitePrivateLimitProfile, FinitePrivateReservation,
    FinitePrivateReservationStatus, FinitePrivateUsageDecision, HostOwnedRuntimeFacts, HostingTier,
    IssueFinitePrivateApiKeyInput, LeaseAgentCreationRequestInput, LeaseRuntimeControlRequestInput,
    LinkStripeCustomerInput, LinkVerifiedUserInput, Project, ProjectImportCandidate,
    ProjectMembershipRole, ProviderOperationEnvelope, ProviderOperationTransition,
    ProviderOperationTransitionRecord, ProviderOperationV1, ProvisionFinitePrivateRuntimeKeyInput,
    ProvisionFinitePrivateRuntimeKeyResult, ReconcileExistingHostImportsOptions,
    ReconcileExistingHostImportsReport, RecordProviderOperationTransitionInput,
    RegisterAgentCreationRuntimeInput, RelayEventsOutput, RelayHeartbeat,
    RequestAgentCreationInput, RequestAgentCreationResult, RequestRuntimeDestroyInput,
    RequestRuntimeRecoverKnownGoodChatInput, RequestRuntimeRestartInput, RequestRuntimeStopInput,
    ReserveFinitePrivateUsageInput, ResetFinitePrivateUsageWindowInput,
    RevokeFinitePrivateApiKeyInput, RevokeFinitePrivateGrantInput, RotateFinitePrivateApiKeyInput,
    RuntimeArtifact, RuntimeBootIntent, RuntimeCapabilitiesEnvelope, RuntimeControlKind,
    RuntimeControlLease, RuntimeControlRequest, RuntimeControlRequestStatus, RuntimePlacement,
    RuntimeRelayCredential, RuntimeSpecEnvelope, RuntimeSpecIdentity, RuntimeStatusSnapshot,
    RuntimeSummaryStatus, SettleFinitePrivateReservationInput,
    SettleFinitePrivateReservationResult, SourceHostRelayEndpoint, StoreErrorDetail,
    SyncStripeSubscriptionInput, UpsertRuntimeArtifactInput, UpsertSourceHostRelayEndpointInput,
    agent_creation_entitlement_id_for, append_provider_operation_transition,
    bound_runtime_capabilities_to_artifact, build_runtime_spec_v1, chat_identity_id_for_user,
    current_time_iso, finite_private_api_key_id_for, finite_private_grant_id_for_user,
    generate_finite_private_api_key, hash_finite_private_api_key, merge_provider_runtime_handle,
    merge_runtime_capabilities, new_agent_creation_request_id, new_agent_runtime_id,
    new_customer_org_id, new_self_service_project_id, new_user_id, normalize_id_part,
    normalize_idempotency_key, normalize_owner_email, normalize_profile_picture_url,
    normalize_runtime_contact_endpoint, normalize_source_host_id,
    parse_agent_creation_request_status, parse_billing_class, parse_billing_subscription_status,
    parse_finite_private_api_key_status, parse_finite_private_grant_status,
    parse_finite_private_reservation_status, parse_hosting_tier, parse_import_candidate_status,
    parse_runner_class, parse_runtime_artifact_kind, parse_runtime_control_kind,
    parse_runtime_control_request_status, parse_runtime_resource_class,
    parse_runtime_summary_status, parse_time, parse_user_link_status,
    project_room_membership_id_for, project_runtime_link_id_for,
    provider_operation_allows_generic_failure, provider_operation_at_runtime_boundary,
    runtime_artifact_material_matches, runtime_artifact_reference_is_immutable_oci,
    runtime_operation_spec_v1, runtime_relay_token_hash, runtime_spec_v1,
    runtime_upgrade_prelease_rejection_is_terminal, should_replace_stripe_subscription,
    source_import_key, trim_to_option, validate_runtime_capabilities_artifact_policy,
    validate_runtime_capabilities_policy, validate_runtime_spec_binding,
    validate_runtime_spec_environment,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use time::Duration;
use time::format_description::well_known::Rfc3339;
use tokio::sync::Mutex;
use tokio_postgres::{Client, GenericClient, NoTls, Row};
use tracing::Instrument;

#[derive(Clone)]
pub enum CoreStore {
    Memory(MemoryCoreStore),
    Postgres(PostgresCoreStore),
}

#[derive(Clone, Default)]
pub struct MemoryCoreStore {
    state: Arc<Mutex<BridgeCoreState>>,
    runtime_environment: Arc<BTreeMap<String, String>>,
}

#[derive(Clone)]
pub struct PostgresCoreStore {
    client: Arc<Mutex<Client>>,
    runtime_environment: Arc<BTreeMap<String, String>>,
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

impl CoreStore {
    pub fn memory() -> Self {
        Self::Memory(MemoryCoreStore::default())
    }

    pub async fn connect_postgres(database_url: &str) -> CoreResult<Self> {
        Ok(Self::Postgres(
            PostgresCoreStore::connect(database_url).await?,
        ))
    }

    pub fn with_runtime_environment(
        mut self,
        runtime_environment: BTreeMap<String, String>,
    ) -> CoreResult<Self> {
        validate_runtime_spec_environment(&runtime_environment)?;
        let runtime_environment = Arc::new(runtime_environment);
        match &mut self {
            Self::Memory(store) => store.runtime_environment = runtime_environment,
            Self::Postgres(store) => store.runtime_environment = runtime_environment,
        }
        Ok(self)
    }

    pub async fn migrate(&self) -> CoreResult<()> {
        match self {
            Self::Memory(_) => Ok(()),
            Self::Postgres(store) => store.migrate().await,
        }
    }

    pub async fn reconcile_existing_host_imports(
        &self,
        records: Vec<ExistingHostProjectImport>,
        options: ReconcileExistingHostImportsOptions,
    ) -> CoreResult<ReconcileExistingHostImportsReport> {
        match self {
            Self::Memory(store) => {
                store
                    .reconcile_existing_host_imports(records, options)
                    .await
            }
            Self::Postgres(store) => {
                store
                    .reconcile_existing_host_imports(records, options)
                    .await
            }
        }
    }

    pub async fn claim_project_imports(
        &self,
        input: ClaimProjectImportsInput,
    ) -> CoreResult<ClaimProjectImportsResult> {
        match self {
            Self::Memory(store) => store.claim_project_imports(input).await,
            Self::Postgres(store) => store.claim_project_imports(input).await,
        }
    }

    pub async fn issue_launch_code_batch(
        &self,
        input: IssueLaunchCodeBatchInput,
    ) -> CoreResult<IssuedLaunchCodeBatch> {
        match self {
            Self::Memory(store) => store.issue_launch_code_batch(input).await,
            Self::Postgres(store) => store.issue_launch_code_batch(input).await,
        }
    }

    pub async fn list_launch_code_batches(&self) -> CoreResult<Vec<LaunchCodeBatchDetails>> {
        match self {
            Self::Memory(store) => store.list_launch_code_batches().await,
            Self::Postgres(store) => store.list_launch_code_batches().await,
        }
    }

    pub async fn revoke_launch_code_batch(
        &self,
        input: RevokeLaunchCodeBatchInput,
    ) -> CoreResult<LaunchCodeBatchDetails> {
        match self {
            Self::Memory(store) => store.revoke_launch_code_batch(input).await,
            Self::Postgres(store) => store.revoke_launch_code_batch(input).await,
        }
    }

    pub async fn request_agent_creation(
        &self,
        input: RequestAgentCreationInput,
    ) -> CoreResult<RequestAgentCreationResult> {
        self.request_agent_creation_configured(input, AgentCreationConfiguration::default())
            .await
    }

    pub async fn request_agent_creation_configured(
        &self,
        input: RequestAgentCreationInput,
        configuration: AgentCreationConfiguration,
    ) -> CoreResult<RequestAgentCreationResult> {
        match self {
            Self::Memory(store) => {
                store
                    .request_agent_creation_configured(input, configuration)
                    .await
            }
            Self::Postgres(store) => {
                store
                    .request_agent_creation_configured(input, configuration)
                    .await
            }
        }
    }

    pub async fn request_runtime_restart(
        &self,
        input: RequestRuntimeRestartInput,
    ) -> CoreResult<RuntimeControlRequest> {
        match self {
            Self::Memory(store) => store.request_runtime_restart(input).await,
            Self::Postgres(store) => store.request_runtime_restart(input).await,
        }
    }

    pub async fn request_runtime_recover_known_good_chat(
        &self,
        input: RequestRuntimeRecoverKnownGoodChatInput,
    ) -> CoreResult<RuntimeControlRequest> {
        match self {
            Self::Memory(store) => store.request_runtime_recover_known_good_chat(input).await,
            Self::Postgres(store) => store.request_runtime_recover_known_good_chat(input).await,
        }
    }

    pub async fn request_runtime_stop(
        &self,
        input: RequestRuntimeStopInput,
    ) -> CoreResult<RuntimeControlRequest> {
        match self {
            Self::Memory(store) => store.request_runtime_stop(input).await,
            Self::Postgres(store) => store.request_runtime_stop(input).await,
        }
    }

    pub async fn request_runtime_destroy(
        &self,
        input: RequestRuntimeDestroyInput,
    ) -> CoreResult<RuntimeControlRequest> {
        match self {
            Self::Memory(store) => store.request_runtime_destroy(input).await,
            Self::Postgres(store) => store.request_runtime_destroy(input).await,
        }
    }

    pub async fn archive_imported_project(
        &self,
        input: ArchiveImportedProjectInput,
    ) -> CoreResult<()> {
        match self {
            Self::Memory(store) => store.archive_imported_project(input).await,
            Self::Postgres(store) => store.archive_imported_project(input).await,
        }
    }

    pub async fn link_verified_user(&self, input: LinkVerifiedUserInput) -> CoreResult<CoreUser> {
        match self {
            Self::Memory(store) => store.link_verified_user(input).await,
            Self::Postgres(store) => store.link_verified_user(input).await,
        }
    }

    pub async fn billing_overview(
        &self,
        input: LinkVerifiedUserInput,
    ) -> CoreResult<BillingOverview> {
        match self {
            Self::Memory(store) => store.billing_overview(input).await,
            Self::Postgres(store) => store.billing_overview(input).await,
        }
    }

    pub async fn link_stripe_customer(
        &self,
        input: LinkStripeCustomerInput,
    ) -> CoreResult<CustomerBillingAccount> {
        match self {
            Self::Memory(store) => store.link_stripe_customer(input).await,
            Self::Postgres(store) => store.link_stripe_customer(input).await,
        }
    }

    pub async fn sync_stripe_subscription(
        &self,
        input: SyncStripeSubscriptionInput,
    ) -> CoreResult<CustomerBillingAccount> {
        match self {
            Self::Memory(store) => store.sync_stripe_subscription(input).await,
            Self::Postgres(store) => store.sync_stripe_subscription(input).await,
        }
    }

    pub async fn lease_agent_creation_request(
        &self,
        input: LeaseAgentCreationRequestInput,
    ) -> CoreResult<Option<AgentCreationLease>> {
        match self {
            Self::Memory(store) => store.lease_agent_creation_request(input).await,
            Self::Postgres(store) => store.lease_agent_creation_request(input).await,
        }
    }

    pub async fn record_provider_operation_transition(
        &self,
        input: RecordProviderOperationTransitionInput,
    ) -> CoreResult<ProviderOperationEnvelope> {
        match self {
            Self::Memory(store) => store.record_provider_operation_transition(input).await,
            Self::Postgres(store) => store.record_provider_operation_transition(input).await,
        }
    }

    pub async fn lease_runtime_control_request(
        &self,
        input: LeaseRuntimeControlRequestInput,
    ) -> CoreResult<Option<RuntimeControlLease>> {
        match self {
            Self::Memory(store) => store.lease_runtime_control_request(input).await,
            Self::Postgres(store) => store.lease_runtime_control_request(input).await,
        }
    }

    pub async fn complete_runtime_control_request(
        &self,
        input: CompleteRuntimeControlRequestInput,
    ) -> CoreResult<RuntimeControlRequest> {
        match self {
            Self::Memory(store) => store.complete_runtime_control_request(input).await,
            Self::Postgres(store) => store.complete_runtime_control_request(input).await,
        }
    }

    pub async fn fail_runtime_control_request(
        &self,
        input: FailRuntimeControlRequestInput,
    ) -> CoreResult<RuntimeControlRequest> {
        match self {
            Self::Memory(store) => store.fail_runtime_control_request(input).await,
            Self::Postgres(store) => store.fail_runtime_control_request(input).await,
        }
    }

    pub async fn complete_agent_creation_request(
        &self,
        input: CompleteAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationLease> {
        match self {
            Self::Memory(store) => store.complete_agent_creation_request(input).await,
            Self::Postgres(store) => store.complete_agent_creation_request(input).await,
        }
    }

    pub async fn register_agent_creation_runtime(
        &self,
        input: RegisterAgentCreationRuntimeInput,
    ) -> CoreResult<AgentCreationLease> {
        match self {
            Self::Memory(store) => store.register_agent_creation_runtime(input).await,
            Self::Postgres(store) => store.register_agent_creation_runtime(input).await,
        }
    }

    pub async fn fail_agent_creation_request(
        &self,
        input: FailAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationRequest> {
        match self {
            Self::Memory(store) => store.fail_agent_creation_request(input).await,
            Self::Postgres(store) => store.fail_agent_creation_request(input).await,
        }
    }

    pub async fn cancel_agent_creation_request(
        &self,
        input: CancelAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationRequest> {
        match self {
            Self::Memory(store) => store.cancel_agent_creation_request(input).await,
            Self::Postgres(store) => store.cancel_agent_creation_request(input).await,
        }
    }

    pub async fn record_runtime_heartbeat(&self, relay_token: &str) -> CoreResult<RelayHeartbeat> {
        match self {
            Self::Memory(store) => store.record_runtime_heartbeat(relay_token).await,
            Self::Postgres(store) => store.record_runtime_heartbeat(relay_token).await,
        }
    }

    pub async fn relay_events_for_runtime(
        &self,
        relay_token: &str,
    ) -> CoreResult<RelayEventsOutput> {
        match self {
            Self::Memory(store) => store.relay_events_for_runtime(relay_token).await,
            Self::Postgres(store) => store.relay_events_for_runtime(relay_token).await,
        }
    }

    pub async fn runtime_heartbeat_for_machine(
        &self,
        source_machine_id: &str,
    ) -> CoreResult<RelayHeartbeat> {
        match self {
            Self::Memory(store) => store.runtime_heartbeat_for_machine(source_machine_id).await,
            Self::Postgres(store) => store.runtime_heartbeat_for_machine(source_machine_id).await,
        }
    }

    pub async fn claimable_candidates_for_email(
        &self,
        email: Option<&str>,
    ) -> CoreResult<Vec<ProjectImportCandidate>> {
        match self {
            Self::Memory(store) => store.claimable_candidates_for_email(email).await,
            Self::Postgres(store) => store.claimable_candidates_for_email(email).await,
        }
    }

    pub async fn visible_projects_for_workos_user(
        &self,
        workos_user_id: &str,
    ) -> CoreResult<Vec<VisibleProject>> {
        match self {
            Self::Memory(store) => store.visible_projects_for_workos_user(workos_user_id).await,
            Self::Postgres(store) => store.visible_projects_for_workos_user(workos_user_id).await,
        }
    }

    pub async fn agent_creation_requests_for_workos_user(
        &self,
        workos_user_id: &str,
    ) -> CoreResult<Vec<AgentCreationRequest>> {
        match self {
            Self::Memory(store) => {
                store
                    .agent_creation_requests_for_workos_user(workos_user_id)
                    .await
            }
            Self::Postgres(store) => {
                store
                    .agent_creation_requests_for_workos_user(workos_user_id)
                    .await
            }
        }
    }

    pub async fn source_host_relay_endpoint(
        &self,
        source_host_id: &str,
    ) -> CoreResult<Option<SourceHostRelayEndpoint>> {
        match self {
            Self::Memory(store) => store.source_host_relay_endpoint(source_host_id).await,
            Self::Postgres(store) => store.source_host_relay_endpoint(source_host_id).await,
        }
    }

    pub async fn upsert_source_host_relay_endpoint(
        &self,
        input: UpsertSourceHostRelayEndpointInput,
    ) -> CoreResult<SourceHostRelayEndpoint> {
        match self {
            Self::Memory(store) => store.upsert_source_host_relay_endpoint(input).await,
            Self::Postgres(store) => store.upsert_source_host_relay_endpoint(input).await,
        }
    }

    pub async fn runtime_artifact(&self, id: &str) -> CoreResult<Option<RuntimeArtifact>> {
        match self {
            Self::Memory(store) => store.runtime_artifact(id).await,
            Self::Postgres(store) => store.runtime_artifact(id).await,
        }
    }

    pub async fn upsert_runtime_artifact(
        &self,
        input: UpsertRuntimeArtifactInput,
    ) -> CoreResult<RuntimeArtifact> {
        match self {
            Self::Memory(store) => store.upsert_runtime_artifact(input).await,
            Self::Postgres(store) => store.upsert_runtime_artifact(input).await,
        }
    }

    pub async fn approve_finite_private_grant(
        &self,
        input: ApproveFinitePrivateGrantInput,
    ) -> CoreResult<FinitePrivateGrant> {
        match self {
            Self::Memory(store) => store.approve_finite_private_grant(input).await,
            Self::Postgres(store) => store.approve_finite_private_grant(input).await,
        }
    }

    pub async fn issue_finite_private_api_key(
        &self,
        input: IssueFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        match self {
            Self::Memory(store) => store.issue_finite_private_api_key(input).await,
            Self::Postgres(store) => store.issue_finite_private_api_key(input).await,
        }
    }

    pub async fn provision_finite_private_runtime_key(
        &self,
        input: ProvisionFinitePrivateRuntimeKeyInput,
    ) -> CoreResult<ProvisionFinitePrivateRuntimeKeyResult> {
        match self {
            Self::Memory(store) => store.provision_finite_private_runtime_key(input).await,
            Self::Postgres(store) => store.provision_finite_private_runtime_key(input).await,
        }
    }

    pub async fn revoke_finite_private_grant(
        &self,
        input: RevokeFinitePrivateGrantInput,
    ) -> CoreResult<FinitePrivateGrant> {
        match self {
            Self::Memory(store) => store.revoke_finite_private_grant(input).await,
            Self::Postgres(store) => store.revoke_finite_private_grant(input).await,
        }
    }

    pub async fn revoke_finite_private_api_key(
        &self,
        input: RevokeFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        match self {
            Self::Memory(store) => store.revoke_finite_private_api_key(input).await,
            Self::Postgres(store) => store.revoke_finite_private_api_key(input).await,
        }
    }

    pub async fn rotate_finite_private_api_key(
        &self,
        input: RotateFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        match self {
            Self::Memory(store) => store.rotate_finite_private_api_key(input).await,
            Self::Postgres(store) => store.rotate_finite_private_api_key(input).await,
        }
    }

    pub async fn reset_finite_private_usage_window(
        &self,
        input: ResetFinitePrivateUsageWindowInput,
    ) -> CoreResult<FinitePrivateGrant> {
        match self {
            Self::Memory(store) => store.reset_finite_private_usage_window(input).await,
            Self::Postgres(store) => store.reset_finite_private_usage_window(input).await,
        }
    }

    pub async fn admin_runtime_overviews(&self) -> CoreResult<Vec<AdminRuntimeOverview>> {
        match self {
            Self::Memory(store) => store.admin_runtime_overviews().await,
            Self::Postgres(store) => store.admin_runtime_overviews().await,
        }
    }

    pub async fn admin_request_runtime_restart(
        &self,
        input: AdminRuntimeControlInput,
    ) -> CoreResult<RuntimeControlRequest> {
        match self {
            Self::Memory(store) => store.admin_request_runtime_restart(input).await,
            Self::Postgres(store) => store.admin_request_runtime_restart(input).await,
        }
    }

    pub async fn admin_request_runtime_recover_known_good_chat(
        &self,
        input: AdminRuntimeControlInput,
    ) -> CoreResult<RuntimeControlRequest> {
        match self {
            Self::Memory(store) => {
                store
                    .admin_request_runtime_recover_known_good_chat(input)
                    .await
            }
            Self::Postgres(store) => {
                store
                    .admin_request_runtime_recover_known_good_chat(input)
                    .await
            }
        }
    }

    pub async fn admin_request_runtime_upgrade(
        &self,
        input: AdminRuntimeUpgradeInput,
    ) -> CoreResult<RuntimeControlRequest> {
        match self {
            Self::Memory(store) => store.admin_request_runtime_upgrade(input).await,
            Self::Postgres(store) => store.admin_request_runtime_upgrade(input).await,
        }
    }

    pub async fn admin_issue_finite_private_friend_key(
        &self,
        input: AdminIssueFinitePrivateFriendKeyInput,
    ) -> CoreResult<AdminIssuedFinitePrivateKey> {
        match self {
            Self::Memory(store) => store.admin_issue_finite_private_friend_key(input).await,
            Self::Postgres(store) => store.admin_issue_finite_private_friend_key(input).await,
        }
    }

    pub async fn admin_rotate_finite_private_api_key(
        &self,
        input: AdminRotateFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        match self {
            Self::Memory(store) => store.admin_rotate_finite_private_api_key(input).await,
            Self::Postgres(store) => store.admin_rotate_finite_private_api_key(input).await,
        }
    }

    pub async fn admin_revoke_finite_private_api_key(
        &self,
        input: AdminRevokeFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        match self {
            Self::Memory(store) => store.admin_revoke_finite_private_api_key(input).await,
            Self::Postgres(store) => store.admin_revoke_finite_private_api_key(input).await,
        }
    }

    pub async fn admin_reset_finite_private_usage_window(
        &self,
        input: AdminResetFinitePrivateUsageWindowInput,
    ) -> CoreResult<FinitePrivateGrant> {
        match self {
            Self::Memory(store) => store.admin_reset_finite_private_usage_window(input).await,
            Self::Postgres(store) => store.admin_reset_finite_private_usage_window(input).await,
        }
    }

    pub async fn finite_private_admin_audit_events(
        &self,
    ) -> CoreResult<Vec<FinitePrivateAdminAuditEvent>> {
        match self {
            Self::Memory(store) => store.finite_private_admin_audit_events().await,
            Self::Postgres(store) => store.finite_private_admin_audit_events().await,
        }
    }

    pub async fn finite_private_admin_state(&self) -> CoreResult<FinitePrivateAdminState> {
        match self {
            Self::Memory(store) => store.finite_private_admin_state().await,
            Self::Postgres(store) => store.finite_private_admin_state().await,
        }
    }

    pub async fn reserve_finite_private_usage(
        &self,
        input: ReserveFinitePrivateUsageInput,
    ) -> CoreResult<FinitePrivateUsageDecision> {
        match self {
            Self::Memory(store) => store.reserve_finite_private_usage(input).await,
            Self::Postgres(store) => store.reserve_finite_private_usage(input).await,
        }
    }

    pub async fn settle_finite_private_reservation(
        &self,
        input: SettleFinitePrivateReservationInput,
    ) -> CoreResult<SettleFinitePrivateReservationResult> {
        match self {
            Self::Memory(store) => store.settle_finite_private_reservation(input).await,
            Self::Postgres(store) => store.settle_finite_private_reservation(input).await,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VisibleProject {
    pub project: Project,
    pub runtime: Option<AgentRuntime>,
}

impl MemoryCoreStore {
    pub async fn reconcile_existing_host_imports(
        &self,
        records: Vec<ExistingHostProjectImport>,
        options: ReconcileExistingHostImportsOptions,
    ) -> CoreResult<ReconcileExistingHostImportsReport> {
        let mut state = self.state.lock().await;
        state.reconcile_existing_host_imports(&records, options)
    }

    pub async fn claim_project_imports(
        &self,
        input: ClaimProjectImportsInput,
    ) -> CoreResult<ClaimProjectImportsResult> {
        let mut state = self.state.lock().await;
        state.claim_project_imports(input)
    }

    pub async fn issue_launch_code_batch(
        &self,
        input: IssueLaunchCodeBatchInput,
    ) -> CoreResult<IssuedLaunchCodeBatch> {
        let prepared = prepare_launch_code_batch(input)?;
        let response = IssuedLaunchCodeBatch {
            batch: prepared.batch.clone(),
            codes: prepared.issued_codes,
        };
        let mut state = self.state.lock().await;
        state
            .launch_code_batches
            .insert(prepared.batch.id.clone(), prepared.batch);
        for record in prepared.records {
            state.launch_codes.insert(record.id.clone(), record);
        }
        Ok(response)
    }

    pub async fn list_launch_code_batches(&self) -> CoreResult<Vec<LaunchCodeBatchDetails>> {
        let state = self.state.lock().await;
        let mut batches = state
            .launch_code_batches
            .values()
            .map(|batch| memory_launch_code_batch_details(&state, batch))
            .collect::<Vec<_>>();
        batches.sort_by(|left, right| {
            right
                .batch
                .created_at
                .cmp(&left.batch.created_at)
                .then_with(|| right.batch.id.cmp(&left.batch.id))
        });
        Ok(batches)
    }

    pub async fn revoke_launch_code_batch(
        &self,
        input: RevokeLaunchCodeBatchInput,
    ) -> CoreResult<LaunchCodeBatchDetails> {
        let actor = input.revoked_by_workos_user_id.trim();
        if actor.is_empty() {
            return Err(CoreError::MissingWorkosUserId);
        }
        let now = input.now.unwrap_or(current_time_iso()?);
        parse_time(&now)?;
        let mut state = self.state.lock().await;
        let batch_id = input.batch_id.trim();
        let batch = state
            .launch_code_batches
            .get_mut(batch_id)
            .ok_or(CoreError::LaunchCodeBatchNotFound)?;
        if batch.revoked_at.is_none() {
            batch.revoked_at = Some(now);
            batch.revoked_by_workos_user_id = Some(actor.to_string());
        }
        let batch = batch.clone();
        Ok(memory_launch_code_batch_details(&state, &batch))
    }

    pub async fn request_agent_creation(
        &self,
        input: RequestAgentCreationInput,
    ) -> CoreResult<RequestAgentCreationResult> {
        self.request_agent_creation_configured(input, AgentCreationConfiguration::default())
            .await
    }

    pub async fn request_agent_creation_configured(
        &self,
        input: RequestAgentCreationInput,
        configuration: AgentCreationConfiguration,
    ) -> CoreResult<RequestAgentCreationResult> {
        let mut state = self.state.lock().await;
        state.request_agent_creation_configured(input, configuration)
    }

    pub async fn request_runtime_restart(
        &self,
        input: RequestRuntimeRestartInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut state = self.state.lock().await;
        state.request_runtime_restart(input)
    }

    pub async fn request_runtime_recover_known_good_chat(
        &self,
        input: RequestRuntimeRecoverKnownGoodChatInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut state = self.state.lock().await;
        state.request_runtime_recover_known_good_chat(input)
    }

    pub async fn request_runtime_stop(
        &self,
        input: RequestRuntimeStopInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut state = self.state.lock().await;
        state.request_runtime_stop(input)
    }

    pub async fn request_runtime_destroy(
        &self,
        input: RequestRuntimeDestroyInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut state = self.state.lock().await;
        state.request_runtime_destroy(input)
    }

    pub async fn archive_imported_project(
        &self,
        input: ArchiveImportedProjectInput,
    ) -> CoreResult<()> {
        let mut state = self.state.lock().await;
        state.archive_imported_project(input)
    }

    pub async fn link_verified_user(&self, input: LinkVerifiedUserInput) -> CoreResult<CoreUser> {
        let mut state = self.state.lock().await;
        state.link_verified_user(input)
    }

    pub async fn billing_overview(
        &self,
        input: LinkVerifiedUserInput,
    ) -> CoreResult<BillingOverview> {
        let mut state = self.state.lock().await;
        state.billing_overview(input)
    }

    pub async fn link_stripe_customer(
        &self,
        input: LinkStripeCustomerInput,
    ) -> CoreResult<CustomerBillingAccount> {
        let mut state = self.state.lock().await;
        state.link_stripe_customer(input)
    }

    pub async fn sync_stripe_subscription(
        &self,
        input: SyncStripeSubscriptionInput,
    ) -> CoreResult<CustomerBillingAccount> {
        let mut state = self.state.lock().await;
        state.sync_stripe_subscription(input)
    }

    pub async fn lease_agent_creation_request(
        &self,
        input: LeaseAgentCreationRequestInput,
    ) -> CoreResult<Option<AgentCreationLease>> {
        let mut state = self.state.lock().await;
        state.lease_agent_creation_request_with_runtime_environment(
            input,
            self.runtime_environment.as_ref(),
        )
    }

    pub async fn record_provider_operation_transition(
        &self,
        input: RecordProviderOperationTransitionInput,
    ) -> CoreResult<ProviderOperationEnvelope> {
        let mut state = self.state.lock().await;
        state.record_provider_operation_transition(input)
    }

    pub async fn lease_runtime_control_request(
        &self,
        input: LeaseRuntimeControlRequestInput,
    ) -> CoreResult<Option<RuntimeControlLease>> {
        let mut state = self.state.lock().await;
        state.lease_runtime_control_request_with_runtime_environment(
            input,
            self.runtime_environment.as_ref(),
        )
    }

    pub async fn complete_runtime_control_request(
        &self,
        input: CompleteRuntimeControlRequestInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut state = self.state.lock().await;
        state.complete_runtime_control_request(input)
    }

    pub async fn fail_runtime_control_request(
        &self,
        input: FailRuntimeControlRequestInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut state = self.state.lock().await;
        state.fail_runtime_control_request(input)
    }

    pub async fn complete_agent_creation_request(
        &self,
        input: CompleteAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationLease> {
        let mut state = self.state.lock().await;
        state.complete_agent_creation_request(input)
    }

    pub async fn register_agent_creation_runtime(
        &self,
        input: RegisterAgentCreationRuntimeInput,
    ) -> CoreResult<AgentCreationLease> {
        let mut state = self.state.lock().await;
        state.register_agent_creation_runtime(input)
    }

    pub async fn fail_agent_creation_request(
        &self,
        input: FailAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationRequest> {
        let mut state = self.state.lock().await;
        state.fail_agent_creation_request(input)
    }

    pub async fn cancel_agent_creation_request(
        &self,
        input: CancelAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationRequest> {
        let mut state = self.state.lock().await;
        state.cancel_agent_creation_request(input)
    }

    pub async fn record_runtime_heartbeat(&self, relay_token: &str) -> CoreResult<RelayHeartbeat> {
        let mut state = self.state.lock().await;
        state.record_runtime_heartbeat(relay_token)
    }

    pub async fn relay_events_for_runtime(
        &self,
        relay_token: &str,
    ) -> CoreResult<RelayEventsOutput> {
        let state = self.state.lock().await;
        state.relay_events_for_runtime(relay_token)
    }

    pub async fn runtime_heartbeat_for_machine(
        &self,
        source_machine_id: &str,
    ) -> CoreResult<RelayHeartbeat> {
        let state = self.state.lock().await;
        state.runtime_heartbeat_for_machine(source_machine_id)
    }

    pub async fn claimable_candidates_for_email(
        &self,
        email: Option<&str>,
    ) -> CoreResult<Vec<ProjectImportCandidate>> {
        let state = self.state.lock().await;
        Ok(state.claimable_candidates_for_email(email))
    }

    pub async fn visible_projects_for_workos_user(
        &self,
        workos_user_id: &str,
    ) -> CoreResult<Vec<VisibleProject>> {
        let state = self.state.lock().await;
        Ok(visible_projects_for_workos_user(&state, workos_user_id))
    }

    pub async fn agent_creation_requests_for_workos_user(
        &self,
        workos_user_id: &str,
    ) -> CoreResult<Vec<AgentCreationRequest>> {
        let state = self.state.lock().await;
        Ok(agent_creation_requests_for_workos_user(
            &state,
            workos_user_id,
        ))
    }

    pub async fn source_host_relay_endpoint(
        &self,
        source_host_id: &str,
    ) -> CoreResult<Option<SourceHostRelayEndpoint>> {
        let state = self.state.lock().await;
        state.source_host_relay_endpoint(source_host_id)
    }

    pub async fn upsert_source_host_relay_endpoint(
        &self,
        input: UpsertSourceHostRelayEndpointInput,
    ) -> CoreResult<SourceHostRelayEndpoint> {
        let mut state = self.state.lock().await;
        state.upsert_source_host_relay_endpoint(input)
    }

    pub async fn runtime_artifact(&self, id: &str) -> CoreResult<Option<RuntimeArtifact>> {
        let state = self.state.lock().await;
        state.runtime_artifact(id)
    }

    pub async fn upsert_runtime_artifact(
        &self,
        input: UpsertRuntimeArtifactInput,
    ) -> CoreResult<RuntimeArtifact> {
        let mut state = self.state.lock().await;
        state.upsert_runtime_artifact(input)
    }

    pub async fn approve_finite_private_grant(
        &self,
        input: ApproveFinitePrivateGrantInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut state = self.state.lock().await;
        state.approve_finite_private_grant(input)
    }

    pub async fn issue_finite_private_api_key(
        &self,
        input: IssueFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut state = self.state.lock().await;
        state.issue_finite_private_api_key(input)
    }

    pub async fn provision_finite_private_runtime_key(
        &self,
        input: ProvisionFinitePrivateRuntimeKeyInput,
    ) -> CoreResult<ProvisionFinitePrivateRuntimeKeyResult> {
        let mut state = self.state.lock().await;
        state.provision_finite_private_runtime_key(input)
    }

    pub async fn revoke_finite_private_grant(
        &self,
        input: RevokeFinitePrivateGrantInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut state = self.state.lock().await;
        state.revoke_finite_private_grant(input)
    }

    pub async fn revoke_finite_private_api_key(
        &self,
        input: RevokeFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut state = self.state.lock().await;
        state.revoke_finite_private_api_key(input)
    }

    pub async fn rotate_finite_private_api_key(
        &self,
        input: RotateFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut state = self.state.lock().await;
        state.rotate_finite_private_api_key(input)
    }

    pub async fn reset_finite_private_usage_window(
        &self,
        input: ResetFinitePrivateUsageWindowInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut state = self.state.lock().await;
        state.reset_finite_private_usage_window(input)
    }

    pub async fn admin_runtime_overviews(&self) -> CoreResult<Vec<AdminRuntimeOverview>> {
        let state = self.state.lock().await;
        Ok(state.admin_runtime_overviews())
    }

    pub async fn admin_request_runtime_restart(
        &self,
        input: AdminRuntimeControlInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut state = self.state.lock().await;
        state.admin_request_runtime_restart(input)
    }

    pub async fn admin_request_runtime_recover_known_good_chat(
        &self,
        input: AdminRuntimeControlInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut state = self.state.lock().await;
        state.admin_request_runtime_recover_known_good_chat(input)
    }

    pub async fn admin_request_runtime_upgrade(
        &self,
        input: AdminRuntimeUpgradeInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut state = self.state.lock().await;
        state.admin_request_runtime_upgrade(input)
    }

    pub async fn admin_issue_finite_private_friend_key(
        &self,
        input: AdminIssueFinitePrivateFriendKeyInput,
    ) -> CoreResult<AdminIssuedFinitePrivateKey> {
        let mut state = self.state.lock().await;
        state.admin_issue_finite_private_friend_key(input)
    }

    pub async fn admin_rotate_finite_private_api_key(
        &self,
        input: AdminRotateFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut state = self.state.lock().await;
        state.admin_rotate_finite_private_api_key(input)
    }

    pub async fn admin_revoke_finite_private_api_key(
        &self,
        input: AdminRevokeFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut state = self.state.lock().await;
        state.admin_revoke_finite_private_api_key(input)
    }

    pub async fn admin_reset_finite_private_usage_window(
        &self,
        input: AdminResetFinitePrivateUsageWindowInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut state = self.state.lock().await;
        state.admin_reset_finite_private_usage_window(input)
    }

    pub async fn finite_private_admin_audit_events(
        &self,
    ) -> CoreResult<Vec<FinitePrivateAdminAuditEvent>> {
        let state = self.state.lock().await;
        Ok(finite_private_admin_audit_events_from_state(&state))
    }

    pub async fn finite_private_admin_state(&self) -> CoreResult<FinitePrivateAdminState> {
        let state = self.state.lock().await;
        Ok(finite_private_admin_state_from_state(&state))
    }

    pub async fn reserve_finite_private_usage(
        &self,
        input: ReserveFinitePrivateUsageInput,
    ) -> CoreResult<FinitePrivateUsageDecision> {
        let mut state = self.state.lock().await;
        state.reserve_finite_private_usage(input)
    }

    pub async fn settle_finite_private_reservation(
        &self,
        input: SettleFinitePrivateReservationInput,
    ) -> CoreResult<SettleFinitePrivateReservationResult> {
        let mut state = self.state.lock().await;
        state.settle_finite_private_reservation(input)
    }
}

fn memory_launch_code_batch_details(
    state: &BridgeCoreState,
    batch: &LaunchCodeBatch,
) -> LaunchCodeBatchDetails {
    let mut codes = state
        .launch_codes
        .values()
        .filter(|code| code.batch_id == batch.id)
        .map(LaunchCodeRecord::status)
        .collect::<Vec<_>>();
    codes.sort_by(|left, right| left.id.cmp(&right.id));
    LaunchCodeBatchDetails {
        batch: batch.clone(),
        codes,
    }
}

impl PostgresCoreStore {
    pub async fn connect(database_url: &str) -> CoreResult<Self> {
        let (client, connection) = tokio_postgres::connect(database_url, NoTls)
            .await
            .map_err(store_error)?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                eprintln!("finite-saas-core postgres connection error: {error}");
            }
        });

        Ok(Self {
            client: Arc::new(Mutex::new(client)),
            runtime_environment: Arc::new(BTreeMap::new()),
        })
    }

    pub async fn migrate(&self) -> CoreResult<()> {
        let client = self.client.lock().await;
        client
            .batch_execute(CORE_SCHEMA_SQL)
            .await
            .map_err(store_error)
    }

    pub async fn reconcile_existing_host_imports(
        &self,
        records: Vec<ExistingHostProjectImport>,
        options: ReconcileExistingHostImportsOptions,
    ) -> CoreResult<ReconcileExistingHostImportsReport> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let report = postgres_reconcile_existing_host_imports(&tx, &records, options).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(report)
    }

    pub async fn claim_project_imports(
        &self,
        input: ClaimProjectImportsInput,
    ) -> CoreResult<ClaimProjectImportsResult> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_claim_project_imports(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn issue_launch_code_batch(
        &self,
        input: IssueLaunchCodeBatchInput,
    ) -> CoreResult<IssuedLaunchCodeBatch> {
        let prepared = prepare_launch_code_batch(input)?;
        let response = IssuedLaunchCodeBatch {
            batch: prepared.batch.clone(),
            codes: prepared.issued_codes,
        };
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let code_count = i32::try_from(prepared.batch.code_count)
            .map_err(|_| CoreError::InvalidLaunchCodeBatchSize)?;
        tx.execute(
            "INSERT INTO launch_code_batches
               (id, name, hosting_tier, code_count, expires_at, revoked_at,
                revoked_by_workos_user_id, created_by_workos_user_id, created_at)
             VALUES ($1, $2, $3, $4, $5::text::timestamptz, NULL, NULL, $6,
                     $7::text::timestamptz)",
            &[
                &prepared.batch.id,
                &prepared.batch.name,
                &prepared.batch.hosting_tier.map(HostingTier::as_str),
                &code_count,
                &prepared.batch.expires_at,
                &prepared.batch.created_by_workos_user_id,
                &prepared.batch.created_at,
            ],
        )
        .await
        .map_err(store_error)?;
        for record in prepared.records {
            tx.execute(
                "INSERT INTO launch_codes
                   (id, batch_id, code_hash, redeemed_customer_org_id,
                    redemption_idempotency_key, redeemed_at, created_at)
                 VALUES ($1, $2, $3, NULL, NULL, NULL, $4::text::timestamptz)",
                &[
                    &record.id,
                    &record.batch_id,
                    &record.code_hash,
                    &record.created_at,
                ],
            )
            .await
            .map_err(store_error)?;
        }
        tx.commit().await.map_err(store_error)?;
        Ok(response)
    }

    pub async fn list_launch_code_batches(&self) -> CoreResult<Vec<LaunchCodeBatchDetails>> {
        let client = self.client.lock().await;
        postgres_list_launch_code_batches(&*client).await
    }

    pub async fn revoke_launch_code_batch(
        &self,
        input: RevokeLaunchCodeBatchInput,
    ) -> CoreResult<LaunchCodeBatchDetails> {
        let actor = input.revoked_by_workos_user_id.trim();
        if actor.is_empty() {
            return Err(CoreError::MissingWorkosUserId);
        }
        let now = input.now.unwrap_or(current_time_iso()?);
        parse_time(&now)?;
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let row = tx
            .query_opt(
                "UPDATE launch_code_batches
                    SET revoked_at = COALESCE(revoked_at, $2::text::timestamptz),
                        revoked_by_workos_user_id = COALESCE(revoked_by_workos_user_id, $3)
                  WHERE id = $1
                  RETURNING id, name, hosting_tier, code_count, expires_at::text,
                            revoked_at::text, revoked_by_workos_user_id,
                            created_by_workos_user_id, created_at::text",
                &[&input.batch_id.trim(), &now, &actor],
            )
            .await
            .map_err(store_error)?
            .ok_or(CoreError::LaunchCodeBatchNotFound)?;
        let batch = launch_code_batch_from_row(&row)?;
        let details = postgres_launch_code_batch_details(&tx, batch).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(details)
    }

    pub async fn admin_request_runtime_upgrade(
        &self,
        input: AdminRuntimeUpgradeInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_request_runtime_upgrade(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn request_agent_creation(
        &self,
        input: RequestAgentCreationInput,
    ) -> CoreResult<RequestAgentCreationResult> {
        self.request_agent_creation_configured(input, AgentCreationConfiguration::default())
            .await
    }

    pub async fn request_agent_creation_configured(
        &self,
        input: RequestAgentCreationInput,
        configuration: AgentCreationConfiguration,
    ) -> CoreResult<RequestAgentCreationResult> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_request_agent_creation(&tx, input, configuration).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn request_runtime_restart(
        &self,
        input: RequestRuntimeRestartInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_request_runtime_control(&tx, input, RuntimeControlKind::Restart).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn request_runtime_recover_known_good_chat(
        &self,
        input: RequestRuntimeRecoverKnownGoodChatInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_request_runtime_control(
            &tx,
            input,
            RuntimeControlKind::RecoverKnownGoodChatRuntime,
        )
        .await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn request_runtime_stop(
        &self,
        input: RequestRuntimeStopInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_request_runtime_control(&tx, input, RuntimeControlKind::Stop).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn request_runtime_destroy(
        &self,
        input: RequestRuntimeDestroyInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_request_runtime_control(&tx, input, RuntimeControlKind::Destroy).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn archive_imported_project(
        &self,
        input: ArchiveImportedProjectInput,
    ) -> CoreResult<()> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let verified_email = normalize_owner_email(Some(&input.verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let user = ensure_linked_user_row(
            &tx,
            &verified_email,
            &input.workos_user_id,
            BillingClass::Standard,
            &now,
        )
        .await?;
        let updated = tx
            .execute(
                "UPDATE project_room_memberships AS membership
             SET archived_at = $1::text::timestamptz
             FROM chat_identities AS identity, projects AS project
             WHERE membership.project_id = $2
               AND identity.id = membership.chat_identity_id
               AND identity.user_id = $3
               AND project.id = membership.project_id
               AND project.owner_user_id = $3
               AND project.import_candidate_id IS NOT NULL",
                &[&now, &input.project_id, &user.id],
            )
            .await
            .map_err(store_error)?;
        if updated == 0 {
            return Err(CoreError::ProjectNotFound);
        }
        tx.commit().await.map_err(store_error)?;
        Ok(())
    }

    pub async fn link_verified_user(&self, input: LinkVerifiedUserInput) -> CoreResult<CoreUser> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let verified_email = normalize_owner_email(Some(&input.verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let workos_user_id = input.workos_user_id.trim().to_string();
        if workos_user_id.is_empty() {
            return Err(CoreError::MissingWorkosUserId);
        }
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let user = ensure_linked_user_row(
            &tx,
            &verified_email,
            &workos_user_id,
            BillingClass::Standard,
            &now,
        )
        .await?;
        tx.commit().await.map_err(store_error)?;
        Ok(user)
    }

    pub async fn billing_overview(
        &self,
        input: LinkVerifiedUserInput,
    ) -> CoreResult<BillingOverview> {
        // Read-only: no global lock, no full-state rewrite, no writes at all.
        // A read that wrote the whole DB was anti-pattern #3; this is targeted
        // SELECTs inside a READ ONLY transaction.
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        tx.execute("SET TRANSACTION READ ONLY", &[])
            .await
            .map_err(store_error)?;
        let overview = postgres_billing_overview(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(overview)
    }

    pub async fn link_stripe_customer(
        &self,
        input: LinkStripeCustomerInput,
    ) -> CoreResult<CustomerBillingAccount> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let account = postgres_link_stripe_customer(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(account)
    }

    pub async fn sync_stripe_subscription(
        &self,
        input: SyncStripeSubscriptionInput,
    ) -> CoreResult<CustomerBillingAccount> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let account = postgres_sync_stripe_subscription(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(account)
    }

    pub async fn lease_agent_creation_request(
        &self,
        input: LeaseAgentCreationRequestInput,
    ) -> CoreResult<Option<AgentCreationLease>> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_lease_agent_creation_request(&tx, input, self.runtime_environment.as_ref())
                .await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn record_provider_operation_transition(
        &self,
        input: RecordProviderOperationTransitionInput,
    ) -> CoreResult<ProviderOperationEnvelope> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_record_provider_operation_transition(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn lease_runtime_control_request(
        &self,
        input: LeaseRuntimeControlRequestInput,
    ) -> CoreResult<Option<RuntimeControlLease>> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_lease_runtime_control_request(&tx, input, self.runtime_environment.as_ref())
                .await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn complete_runtime_control_request(
        &self,
        input: CompleteRuntimeControlRequestInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_complete_runtime_control_request(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn fail_runtime_control_request(
        &self,
        input: FailRuntimeControlRequestInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_fail_runtime_control_request(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn complete_agent_creation_request(
        &self,
        input: CompleteAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationLease> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_complete_agent_creation_request(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn register_agent_creation_runtime(
        &self,
        input: RegisterAgentCreationRuntimeInput,
    ) -> CoreResult<AgentCreationLease> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_register_agent_creation_runtime(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn fail_agent_creation_request(
        &self,
        input: FailAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationRequest> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_fail_agent_creation_request(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn cancel_agent_creation_request(
        &self,
        input: CancelAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationRequest> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_cancel_agent_creation_request(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn record_runtime_heartbeat(&self, relay_token: &str) -> CoreResult<RelayHeartbeat> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_record_runtime_heartbeat(&tx, relay_token).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn relay_events_for_runtime(
        &self,
        relay_token: &str,
    ) -> CoreResult<RelayEventsOutput> {
        let client = self.client.lock().await;
        postgres_relay_events_for_runtime(&*client, relay_token).await
    }

    pub async fn runtime_heartbeat_for_machine(
        &self,
        source_machine_id: &str,
    ) -> CoreResult<RelayHeartbeat> {
        let client = self.client.lock().await;
        postgres_runtime_heartbeat_for_machine(&*client, source_machine_id).await
    }

    pub async fn claimable_candidates_for_email(
        &self,
        email: Option<&str>,
    ) -> CoreResult<Vec<ProjectImportCandidate>> {
        let client = self.client.lock().await;
        postgres_claimable_candidates_for_email(&*client, email).await
    }

    pub async fn visible_projects_for_workos_user(
        &self,
        workos_user_id: &str,
    ) -> CoreResult<Vec<VisibleProject>> {
        let client = self.client.lock().await;
        postgres_visible_projects_for_workos_user(&*client, workos_user_id).await
    }

    pub async fn agent_creation_requests_for_workos_user(
        &self,
        workos_user_id: &str,
    ) -> CoreResult<Vec<AgentCreationRequest>> {
        let client = self.client.lock().await;
        postgres_agent_creation_requests_for_workos_user(&*client, workos_user_id).await
    }

    pub async fn source_host_relay_endpoint(
        &self,
        source_host_id: &str,
    ) -> CoreResult<Option<SourceHostRelayEndpoint>> {
        let source_host_id = normalize_source_host_id(source_host_id)?;
        let client = self.client.lock().await;
        select_source_host_relay(&*client, &source_host_id).await
    }

    pub async fn upsert_source_host_relay_endpoint(
        &self,
        input: UpsertSourceHostRelayEndpointInput,
    ) -> CoreResult<SourceHostRelayEndpoint> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let endpoint = postgres_upsert_source_host_relay_endpoint(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(endpoint)
    }

    pub async fn runtime_artifact(&self, id: &str) -> CoreResult<Option<RuntimeArtifact>> {
        let id = trim_to_option(Some(id)).ok_or(CoreError::MissingRuntimeArtifactId)?;
        let client = self.client.lock().await;
        select_runtime_artifact(&*client, &id).await
    }

    pub async fn upsert_runtime_artifact(
        &self,
        input: UpsertRuntimeArtifactInput,
    ) -> CoreResult<RuntimeArtifact> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let artifact = postgres_upsert_runtime_artifact(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(artifact)
    }

    pub async fn approve_finite_private_grant(
        &self,
        input: ApproveFinitePrivateGrantInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let grant = postgres_approve_finite_private_grant(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(grant)
    }

    pub async fn issue_finite_private_api_key(
        &self,
        input: IssueFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let key = postgres_issue_finite_private_api_key(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(key)
    }

    pub async fn provision_finite_private_runtime_key(
        &self,
        input: ProvisionFinitePrivateRuntimeKeyInput,
    ) -> CoreResult<ProvisionFinitePrivateRuntimeKeyResult> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_provision_finite_private_runtime_key(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn revoke_finite_private_grant(
        &self,
        input: RevokeFinitePrivateGrantInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let grant = postgres_revoke_finite_private_grant(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(grant)
    }

    pub async fn revoke_finite_private_api_key(
        &self,
        input: RevokeFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let key = postgres_revoke_finite_private_api_key(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(key)
    }

    pub async fn rotate_finite_private_api_key(
        &self,
        input: RotateFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let key = postgres_rotate_finite_private_api_key(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(key)
    }

    pub async fn reset_finite_private_usage_window(
        &self,
        input: ResetFinitePrivateUsageWindowInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let grant = postgres_reset_finite_private_usage_window(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(grant)
    }

    pub async fn admin_runtime_overviews(&self) -> CoreResult<Vec<AdminRuntimeOverview>> {
        let client = self.client.lock().await;
        postgres_admin_runtime_overviews(&*client).await
    }

    pub async fn admin_request_runtime_restart(
        &self,
        input: AdminRuntimeControlInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_admin_request_runtime_control(&tx, input, RuntimeControlKind::Restart, None)
                .await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn admin_request_runtime_recover_known_good_chat(
        &self,
        input: AdminRuntimeControlInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_request_runtime_control(
            &tx,
            input,
            RuntimeControlKind::RecoverKnownGoodChatRuntime,
            None,
        )
        .await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn admin_issue_finite_private_friend_key(
        &self,
        input: AdminIssueFinitePrivateFriendKeyInput,
    ) -> CoreResult<AdminIssuedFinitePrivateKey> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_issue_finite_private_friend_key(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn admin_rotate_finite_private_api_key(
        &self,
        input: AdminRotateFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_rotate_finite_private_api_key(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn admin_revoke_finite_private_api_key(
        &self,
        input: AdminRevokeFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_revoke_finite_private_api_key(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn admin_reset_finite_private_usage_window(
        &self,
        input: AdminResetFinitePrivateUsageWindowInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_reset_finite_private_usage_window(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    pub async fn finite_private_admin_audit_events(
        &self,
    ) -> CoreResult<Vec<FinitePrivateAdminAuditEvent>> {
        let client = self.client.lock().await;
        postgres_finite_private_admin_audit_events(&*client).await
    }

    pub async fn finite_private_admin_state(&self) -> CoreResult<FinitePrivateAdminState> {
        let client = self.client.lock().await;
        postgres_finite_private_admin_state(&*client).await
    }

    pub async fn reserve_finite_private_usage(
        &self,
        input: ReserveFinitePrivateUsageInput,
    ) -> CoreResult<FinitePrivateUsageDecision> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let decision = postgres_reserve_finite_private_usage(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(decision)
    }

    pub async fn settle_finite_private_reservation(
        &self,
        input: SettleFinitePrivateReservationInput,
    ) -> CoreResult<SettleFinitePrivateReservationResult> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_settle_finite_private_reservation(&tx, input).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }
}

fn visible_projects_for_workos_user(
    state: &BridgeCoreState,
    workos_user_id: &str,
) -> Vec<VisibleProject> {
    let Some(user) = state
        .users
        .values()
        .find(|user| user.workos_user_id.as_deref() == Some(workos_user_id))
    else {
        return Vec::new();
    };

    state
        .visible_projects_for_user(&user.id)
        .into_iter()
        .map(|project| {
            let runtime = state
                .project_runtime_links
                .values()
                .find(|link| link.project_id == project.id && link.active)
                .and_then(|link| state.agent_runtimes.get(&link.agent_runtime_id))
                .cloned();
            VisibleProject { project, runtime }
        })
        .collect()
}

fn finite_private_admin_audit_events_from_state(
    state: &BridgeCoreState,
) -> Vec<FinitePrivateAdminAuditEvent> {
    let mut events = state
        .finite_private_admin_audit_events
        .values()
        .cloned()
        .collect::<Vec<_>>();
    events.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    events
}

fn finite_private_admin_state_from_state(state: &BridgeCoreState) -> FinitePrivateAdminState {
    let mut grants = state
        .finite_private_grants
        .values()
        .cloned()
        .collect::<Vec<_>>();
    grants.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut api_keys = state
        .finite_private_api_keys
        .values()
        .cloned()
        .collect::<Vec<_>>();
    api_keys.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    FinitePrivateAdminState {
        grants,
        api_keys,
        admin_audit_events: finite_private_admin_audit_events_from_state(state),
    }
}

fn agent_creation_requests_for_workos_user(
    state: &BridgeCoreState,
    workos_user_id: &str,
) -> Vec<AgentCreationRequest> {
    let Some(user) = state
        .users
        .values()
        .find(|user| user.workos_user_id.as_deref() == Some(workos_user_id))
    else {
        return Vec::new();
    };

    let mut requests = state
        .agent_creation_requests
        .values()
        .filter(|request| request.owner_user_id == user.id)
        .cloned()
        .collect::<Vec<_>>();
    requests.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    requests
}

/// Observability wrapper around the agent-creation mutation. This is the single
/// most incident-prone write path (it is the one that shipped broken for
/// standard billing while the server logged nothing), so it always runs inside
/// a span carrying `org_id`/`user_id`/`operation` and emits a structured error
/// log on failure. Full DB detail is logged in the `ApiError` conversion behind
/// a correlation id; here we anchor the failure to the org and user.
async fn postgres_request_agent_creation<C>(
    client: &C,
    input: RequestAgentCreationInput,
    configuration: AgentCreationConfiguration,
) -> CoreResult<RequestAgentCreationResult>
where
    C: GenericClient + Sync,
{
    // Best-effort identity for the span/log. Surrogate ids are no longer
    // derivable from the email, so we resolve the real ids by natural-key
    // lookup; failures here must not fail the request, so they just log "-".
    let user = match normalize_owner_email(Some(&input.verified_email)) {
        Some(email) => select_user_by_email(client, &email).await.ok().flatten(),
        None => None,
    };
    let user_id = user.as_ref().map(|user| user.id.clone());
    let org_id = match user_id.as_deref() {
        Some(user_id) => select_personal_org_by_owner(client, user_id)
            .await
            .ok()
            .flatten()
            .map(|org| org.id),
        None => None,
    };
    let span = tracing::info_span!(
        "request_agent_creation",
        operation = "request_agent_creation",
        user_id = user_id.as_deref().unwrap_or("-"),
        org_id = org_id.as_deref().unwrap_or("-"),
    );
    let result = postgres_request_agent_creation_inner(client, input, configuration)
        .instrument(span)
        .await;
    if let Err(error) = &result {
        tracing::error!(
            operation = "request_agent_creation",
            user_id = user_id.as_deref().unwrap_or("-"),
            org_id = org_id.as_deref().unwrap_or("-"),
            error = %error,
            "agent creation request failed"
        );
    }
    result
}

async fn postgres_request_agent_creation_inner<C>(
    client: &C,
    input: RequestAgentCreationInput,
    configuration: AgentCreationConfiguration,
) -> CoreResult<RequestAgentCreationResult>
where
    C: GenericClient + Sync,
{
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
        BillingClass::Sponsored
    } else {
        BillingClass::Standard
    };
    // Gate on billing/launch against the EXISTING org (resolved by natural key),
    // before minting any rows. On the standard-billing path the org already
    // exists from checkout; a brand-new email has no org and thus no billing.
    let existing_org_id = match select_user_by_email(client, &verified_email).await? {
        Some(user) => select_personal_org_by_owner(client, &user.id)
            .await?
            .map(|org| org.id),
        None => None,
    };
    let locked_launch_code = if let Some(code) = launch_code.as_deref() {
        let locked = lock_postgres_launch_code(client, code, &now).await?;
        if let (Some(redeemed_org_id), Some(redeemed_key)) = (
            locked.record.redeemed_customer_org_id.as_deref(),
            locked.record.redemption_idempotency_key.as_deref(),
        ) {
            // A concurrent identical retry may have resolved the org while
            // this transaction waited on the code row lock. Re-read the
            // natural-key mapping after the lock before deciding whether the
            // already-bound redemption is the same account/request.
            let current_org_id = match select_user_by_email(client, &verified_email).await? {
                Some(user) => select_personal_org_by_owner(client, &user.id)
                    .await?
                    .map(|org| org.id),
                None => None,
            };
            if current_org_id.as_deref() != Some(redeemed_org_id) || idempotency_key != redeemed_key
            {
                return Err(CoreError::InvalidLaunchCode);
            }
        } else if locked.record.redeemed_customer_org_id.is_some()
            || locked.record.redemption_idempotency_key.is_some()
        {
            return Err(CoreError::InvalidLaunchCode);
        }
        Some(locked)
    } else if !match existing_org_id.as_deref() {
        Some(org_id) => postgres_customer_org_has_active_billing(client, org_id).await?,
        None => false,
    } {
        return Err(CoreError::BillingRequired);
    } else {
        None
    };
    let hosting_tier = if let Some(locked) = locked_launch_code.as_ref() {
        locked.hosting_tier.unwrap_or(HostingTier::Standard)
    } else {
        let org_id = existing_org_id
            .as_deref()
            .ok_or(CoreError::MissingHostingTier)?;
        select_customer_billing_account(client, org_id, false)
            .await?
            .and_then(|account| account.hosting_tier)
            .ok_or(CoreError::MissingHostingTier)?
    };
    let placement = configuration
        .placement
        .unwrap_or_else(|| RuntimePlacement::for_hosting_tier(hosting_tier));
    if client
        .query_opt(
            "SELECT id FROM users WHERE workos_user_id = $1 AND normalized_email <> $2",
            &[&workos_user_id, &verified_email],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::WorkosUserConflict);
    }

    let user = upsert_linked_user(client, &verified_email, &workos_user_id, &now).await?;
    let org = ensure_personal_org_row(client, &user, billing_class, &now).await?;

    // Dedupe via the UNIQUE(owner_user_id, idempotency_key): look up an existing
    // request, return it as reused; only mint fresh surrogate ids on a new one.
    if let Some(existing_request) =
        select_agent_creation_request_by_idempotency(client, &user.id, &idempotency_key).await?
    {
        if let Some(locked) = locked_launch_code.as_ref()
            && (locked.record.redeemed_customer_org_id.as_deref() != Some(org.id.as_str())
                || locked.record.redemption_idempotency_key.as_deref()
                    != Some(idempotency_key.as_str())
                || existing_request.requested_launch_code.as_deref()
                    != Some(locked.record.id.as_str()))
        {
            return Err(CoreError::InvalidLaunchCode);
        }
        let project = select_project(client, &existing_request.project_id)
            .await?
            .ok_or_else(|| missing_request_project_error(&existing_request))?;
        ensure_hosted_web_membership_row(client, &user, &project.id, &now).await?;
        return Ok(RequestAgentCreationResult {
            project,
            request: existing_request,
            reused: true,
        });
    }

    let allowed_new_agent_runtimes = if let Some(locked) = locked_launch_code.as_ref() {
        if locked.record.redeemed_customer_org_id.is_none() {
            grant_launch_code_agent_creation_entitlement_row(
                client,
                &org.id,
                &locked.record.id,
                hosting_tier,
                &now,
            )
            .await?
            .allowed_new_agent_runtimes
        } else {
            select_agent_creation_entitlement_by_org(client, &org.id)
                .await?
                .map(|entitlement| entitlement.allowed_new_agent_runtimes)
                .unwrap_or(0)
        }
    } else {
        select_agent_creation_entitlement_by_org(client, &org.id)
            .await?
            .map(|entitlement| entitlement.allowed_new_agent_runtimes)
            .unwrap_or(1)
    };
    let active_request_count =
        postgres_active_agent_creation_entitlement_count(client, &org.id).await?;
    if active_request_count >= i64::from(allowed_new_agent_runtimes) {
        return Err(CoreError::AgentCreationEntitlementExhausted);
    }
    if let Some(locked) = locked_launch_code.as_ref() {
        if locked.record.redeemed_customer_org_id.is_none() {
            redeem_postgres_launch_code(client, &locked.record.id, &org.id, &idempotency_key, &now)
                .await?;
        }
    } else {
        ensure_standard_agent_creation_entitlement_row(client, &org.id, &now).await?;
    }

    let request_id = new_agent_creation_request_id()?;
    let project_id = new_self_service_project_id()?;
    let project = Project {
        id: project_id.clone(),
        customer_org_id: org.id.clone(),
        owner_user_id: user.id.clone(),
        display_name: display_name.clone(),
        import_candidate_id: None,
        hosting_tier: Some(hosting_tier),
        placement: Some(placement),
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    upsert_project_row(client, &project).await?;

    let request = AgentCreationRequest {
        id: request_id,
        customer_org_id: org.id,
        owner_user_id: user.id.clone(),
        project_id: project_id.clone(),
        idempotency_key,
        display_name,
        runner_class: placement.runner_class,
        hosting_tier: Some(hosting_tier),
        placement: Some(placement),
        desired_runtime_artifact_id: None,
        runtime_spec: None,
        profile_picture_url,
        status: AgentCreationRequestStatus::Requested,
        requested_launch_code: locked_launch_code.map(|locked| locked.record.id),
        agent_runtime_id: None,
        runner_id: None,
        lease_token: None,
        lease_expires_at: None,
        failure_message: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    upsert_agent_creation_request_row(client, &request).await?;
    ensure_hosted_web_membership_row(client, &user, &project_id, &request.created_at).await?;

    Ok(RequestAgentCreationResult {
        project,
        request,
        reused: false,
    })
}

async fn postgres_customer_org_has_active_billing<C>(
    client: &C,
    customer_org_id: &str,
) -> CoreResult<bool>
where
    C: GenericClient + Sync,
{
    let Some(row) = client
        .query_opt(
            "SELECT subscription_status FROM customer_billing_accounts
             WHERE customer_org_id = $1",
            &[&customer_org_id],
        )
        .await
        .map_err(store_error)?
    else {
        return Ok(false);
    };
    let Some(status) = row.get::<_, Option<String>>("subscription_status") else {
        return Ok(false);
    };
    let status = parse_billing_subscription_status(&status)
        .ok_or(CoreError::InvalidBillingSubscriptionStatus)?;
    Ok(BillingSubscriptionStatus::can_create_agent(status))
}

async fn select_customer_billing_account<C>(
    client: &C,
    customer_org_id: &str,
    for_update: bool,
) -> CoreResult<Option<CustomerBillingAccount>>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT customer_org_id, hosting_tier, stripe_customer_id, stripe_subscription_id, stripe_price_id,
                subscription_status, current_period_end::text, cancel_at_period_end,
                last_stripe_event_id, last_stripe_event_created, created_at::text, updated_at::text
         FROM customer_billing_accounts WHERE customer_org_id = $1{}",
        if for_update { " FOR UPDATE" } else { "" }
    );
    client
        .query_opt(&sql, &[&customer_org_id])
        .await
        .map_err(store_error)?
        .map(|row| customer_billing_account_from_row(&row))
        .transpose()
}

async fn select_customer_billing_account_by_stripe_customer<C>(
    client: &C,
    stripe_customer_id: &str,
) -> CoreResult<Option<CustomerBillingAccount>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT customer_org_id, hosting_tier, stripe_customer_id, stripe_subscription_id, stripe_price_id,
                    subscription_status, current_period_end::text, cancel_at_period_end,
                    last_stripe_event_id, last_stripe_event_created, created_at::text, updated_at::text
             FROM customer_billing_accounts WHERE stripe_customer_id = $1",
            &[&stripe_customer_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| customer_billing_account_from_row(&row))
        .transpose()
}

async fn select_agent_creation_entitlement_by_org<C>(
    client: &C,
    customer_org_id: &str,
) -> CoreResult<Option<AgentCreationEntitlement>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, customer_org_id, hosting_tier, allowed_new_agent_runtimes, launch_code,
                    created_at::text, updated_at::text
             FROM agent_creation_entitlements WHERE customer_org_id = $1",
            &[&customer_org_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| agent_creation_entitlement_from_row(&row))
        .transpose()
}

/// Mirrors `active_agent_creation_entitlement_count`: active self-serve runtime
/// links plus pending (`requested`/`launching`) self-serve requests for the org,
/// all row-scoped. Imported legacy runtimes remain visible but do not consume a
/// hosted/self-serve launch entitlement.
async fn postgres_active_agent_creation_entitlement_count<C>(
    client: &C,
    customer_org_id: &str,
) -> CoreResult<i64>
where
    C: GenericClient + Sync,
{
    Ok(client
        .query_one(
            "SELECT (
                (SELECT COUNT(*) FROM project_runtime_links links
                 JOIN projects projects ON projects.id = links.project_id
                 WHERE projects.customer_org_id = $1
                   AND projects.import_candidate_id IS NULL
                   AND links.active = TRUE)
                +
                (SELECT COUNT(*) FROM agent_creation_requests
                 WHERE customer_org_id = $1 AND status IN ('requested', 'launching'))
             )::BIGINT",
            &[&customer_org_id],
        )
        .await
        .map_err(store_error)?
        .get(0))
}

/// Read-only billing overview via targeted SELECTs. NEVER writes: an org that
/// does not exist yet (a user who has not reached checkout) yields a synthesized
/// Standard view (`requires_billing`, `!can_create_agent`) rather than creating
/// rows — the persisted org is minted on the write paths (checkout/link), not here.
async fn postgres_billing_overview<C>(
    client: &C,
    input: LinkVerifiedUserInput,
) -> CoreResult<BillingOverview>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let verified_email = normalize_owner_email(Some(&input.verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let workos_user_id = input.workos_user_id.trim().to_string();
    if workos_user_id.is_empty() {
        return Err(CoreError::MissingWorkosUserId);
    }

    let org = match select_user_by_email(client, &verified_email).await? {
        Some(user) => select_personal_org_by_owner(client, &user.id).await?,
        None => None,
    };
    let org = match org {
        Some(org) => org,
        // No persisted org yet: synthesize the Standard default the write paths
        // would create, WITHOUT inserting anything.
        None => CustomerOrganization {
            id: new_customer_org_id()?,
            owner_user_id: String::new(),
            name: verified_email,
            billing_class: BillingClass::Standard,
            created_at: now.clone(),
            updated_at: now,
        },
    };

    let billing_account = select_customer_billing_account(client, &org.id, false).await?;
    let agent_creation_entitlement =
        select_agent_creation_entitlement_by_org(client, &org.id).await?;
    let has_active_billing = postgres_customer_org_has_active_billing(client, &org.id).await?;
    let active_count = postgres_active_agent_creation_entitlement_count(client, &org.id).await?;

    let can_create_agent = agent_creation_entitlement
        .as_ref()
        .is_some_and(|entitlement| {
            active_count < i64::from(entitlement.allowed_new_agent_runtimes)
        })
        && (has_active_billing
            || org.billing_class == BillingClass::Grandfathered
            || org.billing_class == BillingClass::Sponsored);
    let requires_billing = !has_active_billing && org.billing_class == BillingClass::Standard;

    Ok(BillingOverview {
        customer_org: org,
        billing_account,
        agent_creation_entitlement,
        can_create_agent,
        requires_billing,
    })
}

/// Guard `link_stripe_customer_to_org`'s conflict rules: a Stripe customer id may
/// belong to exactly one org, and an org's existing customer id cannot change.
async fn ensure_no_stripe_customer_conflict<C>(
    client: &C,
    customer_org_id: &str,
    stripe_customer_id: &str,
    existing: Option<&CustomerBillingAccount>,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    if let Some(existing_customer_id) =
        existing.and_then(|account| account.stripe_customer_id.as_deref())
        && existing_customer_id != stripe_customer_id
    {
        return Err(CoreError::StripeCustomerConflict);
    }
    if client
        .query_opt(
            "SELECT customer_org_id FROM customer_billing_accounts
             WHERE stripe_customer_id = $1 AND customer_org_id <> $2",
            &[&stripe_customer_id, &customer_org_id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::StripeCustomerConflict);
    }
    Ok(())
}

/// Row-scoped equivalent of `link_stripe_customer_to_org`: upsert only the
/// `customer_billing_accounts` row for this org, setting the Stripe customer id
/// while preserving any existing subscription fields.
async fn postgres_link_stripe_customer_to_org<C>(
    client: &C,
    customer_org_id: &str,
    stripe_customer_id: &str,
    now: &str,
) -> CoreResult<CustomerBillingAccount>
where
    C: GenericClient + Sync,
{
    let existing = select_customer_billing_account(client, customer_org_id, true).await?;
    ensure_no_stripe_customer_conflict(
        client,
        customer_org_id,
        stripe_customer_id,
        existing.as_ref(),
    )
    .await?;
    let row = client
        .query_one(
            "INSERT INTO customer_billing_accounts
               (customer_org_id, hosting_tier, stripe_customer_id, cancel_at_period_end, created_at, updated_at)
             VALUES ($1, 'standard', $2, FALSE, $3::text::timestamptz, $3::text::timestamptz)
             ON CONFLICT (customer_org_id) DO UPDATE SET
               stripe_customer_id = EXCLUDED.stripe_customer_id,
               updated_at = EXCLUDED.updated_at
             RETURNING customer_org_id, stripe_customer_id, stripe_subscription_id, stripe_price_id,
                       hosting_tier, subscription_status, current_period_end::text, cancel_at_period_end,
                       last_stripe_event_id, last_stripe_event_created,
                       created_at::text, updated_at::text",
            &[&customer_org_id, &stripe_customer_id, &now],
        )
        .await
        .map_err(store_error)?;
    customer_billing_account_from_row(&row)
}

async fn postgres_link_stripe_customer<C>(
    client: &C,
    input: LinkStripeCustomerInput,
) -> CoreResult<CustomerBillingAccount>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let verified_email = normalize_owner_email(Some(&input.verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let workos_user_id = input.workos_user_id.trim().to_string();
    if workos_user_id.is_empty() {
        return Err(CoreError::MissingWorkosUserId);
    }
    let stripe_customer_id = trim_to_option(Some(&input.stripe_customer_id))
        .ok_or(CoreError::MissingStripeCustomerId)?;

    // Same WorkOS-conflict guard the in-memory `ensure_linked_user_with_billing_class`
    // enforces: a workos id may not move to a different email.
    if client
        .query_opt(
            "SELECT id FROM users WHERE workos_user_id = $1 AND normalized_email <> $2",
            &[&workos_user_id, &verified_email],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::WorkosUserConflict);
    }

    let user = upsert_linked_user(client, &verified_email, &workos_user_id, &now).await?;
    let org = ensure_personal_org_row(client, &user, BillingClass::Standard, &now).await?;
    postgres_link_stripe_customer_to_org(client, &org.id, &stripe_customer_id, &now).await
}

async fn postgres_sync_stripe_subscription<C>(
    client: &C,
    input: SyncStripeSubscriptionInput,
) -> CoreResult<CustomerBillingAccount>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let stripe_customer_id = trim_to_option(Some(&input.stripe_customer_id))
        .ok_or(CoreError::MissingStripeCustomerId)?;
    let stripe_subscription_id = trim_to_option(Some(&input.stripe_subscription_id))
        .ok_or(CoreError::MissingStripeSubscriptionId)?;
    let stripe_price_id = trim_to_option(input.stripe_price_id.as_deref());

    // Resolve the org: explicit id, else the account that already owns this
    // Stripe customer (natural key), else there is nothing to sync.
    let customer_org_id = match trim_to_option(input.customer_org_id.as_deref()) {
        Some(org_id) => org_id,
        None => select_customer_billing_account_by_stripe_customer(client, &stripe_customer_id)
            .await?
            .map(|account| account.customer_org_id)
            .ok_or(CoreError::BillingAccountNotFound)?,
    };
    if !customer_org_exists(client, &customer_org_id).await? {
        return Err(CoreError::BillingAccountNotFound);
    }

    // Lock this org's billing row for the transaction (row-scoped concurrency).
    let existing = select_customer_billing_account(client, &customer_org_id, true).await?;

    // Event-ordering guard: for the SAME subscription, drop a webhook whose
    // Stripe `event.created` predates the last applied one.
    if let Some(account) = existing.as_ref()
        && account.stripe_subscription_id.as_deref() == Some(stripe_subscription_id.as_str())
        && let (Some(last_created), Some(incoming_created)) = (
            account.last_stripe_event_created,
            input.stripe_event_created,
        )
        && incoming_created < last_created
    {
        return Ok(account.clone());
    }

    // Subscription-replacement guard: don't let a new subscription id clobber an
    // active one unless the status transition warrants it.
    if let Some(account) = existing.as_ref()
        && let Some(existing_subscription_id) = account.stripe_subscription_id.as_deref()
        && existing_subscription_id != stripe_subscription_id
        && !should_replace_stripe_subscription(
            account.subscription_status,
            input.subscription_status,
        )
    {
        return Ok(account.clone());
    }

    ensure_no_stripe_customer_conflict(
        client,
        &customer_org_id,
        &stripe_customer_id,
        existing.as_ref(),
    )
    .await?;

    if input.subscription_status.can_create_agent() {
        let expected_price_id = trim_to_option(input.expected_stripe_price_id.as_deref())
            .ok_or(CoreError::MissingStripeStandardPriceId)?;
        if stripe_price_id.as_deref() != Some(expected_price_id.as_str()) {
            return Err(CoreError::StripeSubscriptionPriceMismatch);
        }
    }

    let subscription_status = input.subscription_status.as_str();
    let last_stripe_event_id = trim_to_option(input.stripe_event_id.as_deref());
    let last_stripe_event_created = input
        .stripe_event_created
        .or_else(|| existing.as_ref().and_then(|a| a.last_stripe_event_created));
    let created_at = existing
        .as_ref()
        .map(|account| account.created_at.clone())
        .unwrap_or_else(|| now.clone());

    let row = client
        .query_one(
            "INSERT INTO customer_billing_accounts (
                customer_org_id, hosting_tier, stripe_customer_id, stripe_subscription_id, stripe_price_id,
                subscription_status, current_period_end, cancel_at_period_end,
                last_stripe_event_id, last_stripe_event_created, created_at, updated_at
             )
             VALUES ($1, 'standard', $2, $3, $4, $5, $6::text::timestamptz, $7, $8, $9, $10::text::timestamptz, $11::text::timestamptz)
             ON CONFLICT (customer_org_id) DO UPDATE SET
               stripe_customer_id = EXCLUDED.stripe_customer_id,
               stripe_subscription_id = EXCLUDED.stripe_subscription_id,
               stripe_price_id = EXCLUDED.stripe_price_id,
               subscription_status = EXCLUDED.subscription_status,
               current_period_end = EXCLUDED.current_period_end,
               cancel_at_period_end = EXCLUDED.cancel_at_period_end,
               last_stripe_event_id = EXCLUDED.last_stripe_event_id,
               last_stripe_event_created = EXCLUDED.last_stripe_event_created,
               updated_at = EXCLUDED.updated_at
             RETURNING customer_org_id, stripe_customer_id, stripe_subscription_id, stripe_price_id,
                       hosting_tier, subscription_status, current_period_end::text, cancel_at_period_end,
                       last_stripe_event_id, last_stripe_event_created,
                       created_at::text, updated_at::text",
            &[
                &customer_org_id,
                &stripe_customer_id,
                &stripe_subscription_id,
                &stripe_price_id,
                &subscription_status,
                &input.current_period_end,
                &input.cancel_at_period_end,
                &last_stripe_event_id,
                &last_stripe_event_created,
                &created_at,
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    let account = customer_billing_account_from_row(&row)?;

    // Grant on active/trialing; zero the standard (non-launch-code) grant otherwise.
    if input.subscription_status.can_create_agent() {
        ensure_standard_agent_creation_entitlement_row(client, &customer_org_id, &now).await?;
        client
            .execute(
                "UPDATE customer_orgs
                 SET billing_class = 'standard', updated_at = $2::text::timestamptz
                 WHERE id = $1",
                &[&customer_org_id, &now],
            )
            .await
            .map_err(store_error)?;
    } else {
        client
            .execute(
                "UPDATE agent_creation_entitlements
                 SET allowed_new_agent_runtimes = 0, updated_at = $2::text::timestamptz
                 WHERE customer_org_id = $1 AND launch_code IS NULL",
                &[&customer_org_id, &now],
            )
            .await
            .map_err(store_error)?;
    }

    Ok(account)
}

async fn customer_org_exists<C>(client: &C, org_id: &str) -> CoreResult<bool>
where
    C: GenericClient + Sync,
{
    Ok(client
        .query_opt("SELECT id FROM customer_orgs WHERE id = $1", &[&org_id])
        .await
        .map_err(store_error)?
        .is_some())
}

async fn postgres_lease_agent_creation_request<C>(
    client: &C,
    input: LeaseAgentCreationRequestInput,
    runtime_environment: &BTreeMap<String, String>,
) -> CoreResult<Option<AgentCreationLease>>
where
    C: GenericClient + Sync,
{
    validate_runtime_spec_environment(runtime_environment)?;
    let now = input.now.unwrap_or(current_time_iso()?);
    let now_time = parse_time(&now)?;
    let runner_id =
        trim_to_option(Some(&input.runner_id)).ok_or(CoreError::MissingAgentCreationRunnerId)?;
    let lease_token = trim_to_option(Some(&input.lease_token))
        .ok_or(CoreError::MissingAgentCreationLeaseToken)?;
    let lease_seconds = input
        .lease_seconds
        .unwrap_or(crate::DEFAULT_AGENT_CREATION_LEASE_SECONDS);
    if !(1..=crate::MAX_AGENT_CREATION_LEASE_SECONDS).contains(&lease_seconds) {
        return Err(CoreError::InvalidAgentCreationLeaseDuration);
    }
    if input
        .runner_capacity
        .as_ref()
        .is_some_and(|capacity| !capacity.accepts_agent_creation())
    {
        return Ok(None);
    }
    // Partition the claim by source host: a runner declaring a host leases only
    // requests routable to it (`target_source_host_id` NULL = any runner, else
    // must match). This replaces the global claim across all rows; the
    // `agent_creation_requests_lease_partition_idx` backs the scan, and
    // FOR UPDATE SKIP LOCKED keeps concurrent runners off each other's rows.
    let source_host_id = input
        .source_host_id
        .as_deref()
        .map(normalize_source_host_id)
        .transpose()?;
    let runner_classes = input.runner_capacity.as_ref().map(|capacity| {
        capacity
            .runner_classes
            .iter()
            .map(|runner_class| runner_class.as_str().to_owned())
            .collect::<Vec<_>>()
    });
    let lease_expires_at = (now_time + Duration::seconds(lease_seconds)).format(&Rfc3339)?;
    let Some(row) = client
        .query_opt(
            "WITH candidate AS (
                SELECT id
                FROM agent_creation_requests
                WHERE (
                        status = 'requested'
                        OR (
                          status = 'launching'
                          AND (lease_expires_at IS NULL OR lease_expires_at <= $4::text::timestamptz)
                        )
                      )
                  AND (
                        target_source_host_id IS NULL
                        OR $5::text IS NULL
                        OR target_source_host_id = $5
                      )
                  AND (
                        $6::text[] IS NULL
                        OR runner_class = ANY($6::text[])
                      )
                ORDER BY created_at, id
                FOR UPDATE SKIP LOCKED
                LIMIT 1
             )
             UPDATE agent_creation_requests AS request
             SET status = 'launching',
                 runner_id = $1,
                 lease_token = $2,
                 lease_expires_at = $3::text::timestamptz,
                 failure_message = NULL,
                 updated_at = $4::text::timestamptz
             FROM candidate
             WHERE request.id = candidate.id
             RETURNING request.id, request.customer_org_id, request.owner_user_id,
                       request.project_id, request.idempotency_key, request.display_name,
                       request.runner_class, request.hosting_tier,
                       request.placement_runner_class, request.runtime_resource_class,
                       request.desired_runtime_artifact_id, request.runtime_spec,
                       request.profile_picture_url,
                       request.status, request.requested_launch_code, request.agent_runtime_id,
                       request.runner_id, request.lease_token, request.lease_expires_at::text,
                       request.failure_message, request.created_at::text, request.updated_at::text",
            &[
                &runner_id,
                &lease_token,
                &lease_expires_at,
                &now,
                &source_host_id,
                &runner_classes,
            ],
        )
        .await
        .map_err(store_error)?
    else {
        return Ok(None);
    };
    let mut request = agent_creation_request_from_row(&row)?;
    let project = select_project(client, &request.project_id)
        .await?
        .ok_or_else(|| missing_request_project_error(&request))?;
    let placement = request
        .placement
        .or(project.placement)
        .or_else(|| RuntimePlacement::from_legacy_runner_class(request.runner_class));
    if placement.is_some_and(|placement| placement.runner_class != request.runner_class) {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    let prepared = if let Some(existing_spec) = request.runtime_spec.as_ref() {
        let spec = runtime_spec_v1(existing_spec);
        let runtime_id = request
            .agent_runtime_id
            .as_deref()
            .unwrap_or(spec.agent_runtime_id.as_str());
        let placement = placement.ok_or(CoreError::RuntimeSpecMismatch)?;
        let artifact_id = request
            .desired_runtime_artifact_id
            .as_deref()
            .unwrap_or(spec.runtime_artifact_id.as_str());
        let artifact = select_runtime_artifact(client, artifact_id)
            .await?
            .ok_or(CoreError::RuntimeArtifactNotFound)?;
        ensure_artifact_launchable(&artifact)?;
        validate_runtime_spec_binding(
            existing_spec,
            Some(&request.id),
            &request.project_id,
            runtime_id,
            placement,
            &artifact,
        )?;
        Some((runtime_id.to_string(), artifact.id, existing_spec.clone()))
    } else if let Some(placement) = placement {
        let runtime_id = request
            .agent_runtime_id
            .clone()
            .map(Ok)
            .unwrap_or_else(new_agent_runtime_id)?;
        let artifact = match request.desired_runtime_artifact_id.as_deref() {
            Some(artifact_id) => {
                let artifact = select_runtime_artifact(client, artifact_id)
                    .await?
                    .ok_or(CoreError::RuntimeArtifactNotFound)?;
                ensure_artifact_launchable(&artifact)?;
                artifact
            }
            None => select_latest_launchable_runtime_artifact(client).await?,
        };
        let runtime_spec = build_runtime_spec_v1(
            RuntimeSpecIdentity {
                operation_id: &request.id,
                project_id: &request.project_id,
                agent_runtime_id: &runtime_id,
                placement,
            },
            &artifact,
            &runtime_id,
            runtime_environment.clone(),
            vec![FINITE_PRIVATE_SECRET_REFERENCE.to_string()],
            RuntimeBootIntent::Normal,
        )?;
        Some((runtime_id, artifact.id, runtime_spec))
    } else {
        None
    };
    if let Some((runtime_id, artifact_id, runtime_spec)) = prepared {
        let runtime_spec_value = serde_json::to_value(&runtime_spec).map_err(json_error)?;
        client
            .execute(
                "UPDATE agent_creation_requests
                 SET agent_runtime_id = $2, desired_runtime_artifact_id = $3,
                     runtime_spec = $4
                 WHERE id = $1",
                &[&request.id, &runtime_id, &artifact_id, &runtime_spec_value],
            )
            .await
            .map_err(store_error)?;
        request.agent_runtime_id = Some(runtime_id);
        request.desired_runtime_artifact_id = Some(artifact_id);
        request.runtime_spec = Some(runtime_spec);
    }
    let provider_operation = select_provider_operation(client, &request.id).await?;
    Ok(Some(AgentCreationLease {
        project,
        request,
        provider_operation,
    }))
}

async fn postgres_record_provider_operation_transition<C>(
    client: &C,
    input: RecordProviderOperationTransitionInput,
) -> CoreResult<ProviderOperationEnvelope>
where
    C: GenericClient + Sync,
{
    if matches!(
        input.transition,
        ProviderOperationTransition::ProviderHandleRecorded { .. }
            | ProviderOperationTransition::Ready
    ) {
        return Err(CoreError::ProviderOperationBoundaryNotReached);
    }
    let request = locked_agent_creation_request(client, &input.request_id).await?;
    let now = current_time_iso()?;
    verify_agent_creation_lease_active(client, &request, &input.runner_id, &input.lease_token)
        .await?;
    let project = select_project(client, &request.project_id)
        .await?
        .ok_or_else(|| missing_request_project_error(&request))?;
    let placement = request
        .placement
        .or(project.placement)
        .or_else(|| RuntimePlacement::from_legacy_runner_class(request.runner_class))
        .ok_or(CoreError::ProviderOperationIdentityMismatch)?;
    if placement != input.placement {
        return Err(CoreError::ProviderOperationIdentityMismatch);
    }
    let existing = select_provider_operation(client, &input.request_id).await?;
    let previous_len = existing
        .as_ref()
        .map(|operation| operation.v1().transitions.len())
        .unwrap_or_default();
    let updated = append_provider_operation_transition(
        existing.as_ref(),
        &input.request_id,
        &input.correlation_id,
        input.placement,
        input.transition,
        &now,
    )?;
    persist_provider_operation_delta(client, previous_len, &updated).await?;
    select_provider_operation(client, &input.request_id)
        .await?
        .ok_or(CoreError::ProviderOperationTransitionConflict)
}

async fn postgres_register_agent_creation_runtime<C>(
    client: &C,
    input: RegisterAgentCreationRuntimeInput,
) -> CoreResult<AgentCreationLease>
where
    C: GenericClient + Sync,
{
    let now = input.now.clone().unwrap_or(current_time_iso()?);
    let source_host_id = normalize_source_host_id(&input.source_host_id)?;
    let source_machine_id = normalize_id_part(&input.source_machine_id);
    if source_machine_id.is_empty() {
        return Err(CoreError::MissingSourceMachineId);
    }
    let token_hash = trim_to_option(Some(&input.runtime_relay_token_hash))
        .ok_or(CoreError::MissingRuntimeRelayTokenHash)?;
    let artifact_id = trim_to_option(input.runtime_artifact_id.as_deref())
        .ok_or(CoreError::MissingRuntimeArtifactId)?;
    let artifact = select_runtime_artifact(client, &artifact_id)
        .await?
        .ok_or(CoreError::RuntimeArtifactNotFound)?;
    ensure_artifact_launchable(&artifact)?;
    let state_schema_version = trim_to_option(input.state_schema_version.as_deref())
        .unwrap_or_else(|| artifact.state_schema_version.clone());
    let request = locked_agent_creation_request(client, &input.request_id).await?;
    verify_agent_creation_lease(&request, &input.runner_id, &input.lease_token)?;
    let provider_operation = select_provider_operation(client, &input.request_id).await?;
    let provider_operation_now = provider_operation
        .as_ref()
        .map(|_| current_time_iso())
        .transpose()?;
    if provider_operation_now.is_some() {
        verify_agent_creation_lease_active(client, &request, &input.runner_id, &input.lease_token)
            .await?;
    }
    let project = select_project(client, &request.project_id)
        .await?
        .ok_or_else(|| missing_request_project_error(&request))?;
    let source_import_key = source_import_key(&source_host_id, &source_machine_id);
    ensure_runtime_source_available(client, &source_import_key, &project.id).await?;
    // New-generation requests preallocate the Core runtime id inside their
    // persisted RuntimeSpec. N-1 rows retain source-key adoption semantics.
    let runtime_by_source =
        select_agent_runtime_by_source_import_key(client, &source_import_key).await?;
    let placement = request.placement.or(project.placement).or(runtime_by_source
        .as_ref()
        .and_then(|runtime| runtime.placement));
    let runtime_id = if let Some(runtime_spec) = request.runtime_spec.as_ref() {
        let placement = placement.ok_or(CoreError::RuntimeSpecMismatch)?;
        let spec = runtime_spec_v1(runtime_spec);
        validate_runtime_spec_binding(
            runtime_spec,
            Some(&request.id),
            &project.id,
            &spec.agent_runtime_id,
            placement,
            &artifact,
        )?;
        if request.agent_runtime_id.as_deref() != Some(spec.agent_runtime_id.as_str())
            || runtime_by_source
                .as_ref()
                .is_some_and(|runtime| runtime.id != spec.agent_runtime_id)
        {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        spec.agent_runtime_id.clone()
    } else {
        runtime_by_source
            .as_ref()
            .map(|runtime| runtime.id.clone())
            .map(Ok)
            .unwrap_or_else(new_agent_runtime_id)?
    };
    let existing_runtime = match runtime_by_source {
        Some(runtime) => Some(runtime),
        None => select_agent_runtime(client, &runtime_id)
            .await?
            .filter(|runtime| runtime.source_import_key == source_import_key),
    };
    let (provider_runtime_handle, provider_runtime_handle_history) = merge_provider_runtime_handle(
        existing_runtime.as_ref(),
        input.provider_runtime_handle.clone(),
        placement,
    )?;
    let contact_endpoint = normalize_runtime_contact_endpoint(input.contact_endpoint.as_deref())?
        .or_else(|| existing_runtime.as_ref()?.contact_endpoint.clone());
    let bounded_runtime_capabilities =
        bound_runtime_capabilities_to_artifact(input.runtime_capabilities.clone(), &artifact);
    validate_runtime_capabilities_policy(bounded_runtime_capabilities.as_ref(), placement)?;
    let runtime_capabilities =
        merge_runtime_capabilities(existing_runtime.as_ref(), bounded_runtime_capabilities)?;
    let runtime = AgentRuntime {
        id: runtime_id.clone(),
        project_id: project.id.clone(),
        source_host_id: source_host_id.clone(),
        source_machine_id,
        source_import_key,
        runtime_artifact_id: Some(artifact.id),
        state_schema_version: Some(state_schema_version),
        placement,
        provider_runtime_handle,
        provider_runtime_handle_history,
        contact_endpoint,
        runtime_capabilities,
        host_facts: runtime_host_facts_from_register_input(&input, &request, &source_host_id),
        created_at: existing_runtime
            .map(|runtime| runtime.created_at)
            .unwrap_or_else(|| now.clone()),
        updated_at: now.clone(),
    };
    let updated_provider_operation = provider_operation_at_runtime_boundary(
        provider_operation.as_ref(),
        runtime.provider_runtime_handle.as_ref(),
        false,
        provider_operation_now.as_deref().unwrap_or(&now),
    )?;
    if let Some(operation) = updated_provider_operation.as_ref() {
        persist_provider_operation_delta(
            client,
            provider_operation
                .as_ref()
                .map(|operation| operation.v1().transitions.len())
                .unwrap_or_default(),
            operation,
        )
        .await?;
    }
    let provider_operation_ack = if updated_provider_operation.is_some() {
        select_provider_operation(client, &input.request_id).await?
    } else {
        provider_operation.clone()
    };
    upsert_agent_runtime_row(client, &runtime).await?;
    upsert_runtime_relay_credential_row(
        client,
        &RuntimeRelayCredential {
            agent_runtime_id: runtime_id.clone(),
            token_hash,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
    )
    .await?;
    activate_project_runtime_link(client, &project.id, &runtime_id, &now).await?;
    let request =
        update_agent_creation_runtime_registered(client, &input.request_id, &runtime_id, &now)
            .await?;
    Ok(AgentCreationLease {
        project,
        request,
        provider_operation: provider_operation_ack,
    })
}

async fn postgres_complete_agent_creation_request<C>(
    client: &C,
    input: CompleteAgentCreationRequestInput,
) -> CoreResult<AgentCreationLease>
where
    C: GenericClient + Sync,
{
    let now = input.now.clone().unwrap_or(current_time_iso()?);
    let source_host_id = normalize_source_host_id(&input.source_host_id)?;
    let source_machine_id = normalize_id_part(&input.source_machine_id);
    if source_machine_id.is_empty() {
        return Err(CoreError::MissingSourceMachineId);
    }
    let request = locked_agent_creation_request(client, &input.request_id).await?;
    verify_agent_creation_lease(&request, &input.runner_id, &input.lease_token)?;
    let provider_operation = select_provider_operation(client, &input.request_id).await?;
    let provider_operation_now = provider_operation
        .as_ref()
        .map(|_| current_time_iso())
        .transpose()?;
    if provider_operation_now.is_some() {
        verify_agent_creation_lease_active(client, &request, &input.runner_id, &input.lease_token)
            .await?;
    }
    let existing_runtime = match request.agent_runtime_id.as_deref() {
        Some(runtime_id) => select_agent_runtime(client, runtime_id).await?,
        None => None,
    };
    let artifact_id = trim_to_option(input.runtime_artifact_id.as_deref())
        .or_else(|| existing_runtime.as_ref()?.runtime_artifact_id.clone())
        .ok_or(CoreError::MissingRuntimeArtifactId)?;
    let artifact = select_runtime_artifact(client, &artifact_id)
        .await?
        .ok_or(CoreError::RuntimeArtifactNotFound)?;
    ensure_artifact_launchable(&artifact)?;
    let state_schema_version = trim_to_option(input.state_schema_version.as_deref())
        .or_else(|| existing_runtime.as_ref()?.state_schema_version.clone())
        .unwrap_or_else(|| artifact.state_schema_version.clone());
    let project = select_project(client, &request.project_id)
        .await?
        .ok_or_else(|| missing_request_project_error(&request))?;
    let source_import_key = source_import_key(&source_host_id, &source_machine_id);
    ensure_runtime_source_available(client, &source_import_key, &project.id).await?;
    let runtime_by_source =
        select_agent_runtime_by_source_import_key(client, &source_import_key).await?;
    let placement = request
        .placement
        .or(project.placement)
        .or(existing_runtime
            .as_ref()
            .and_then(|runtime| runtime.placement))
        .or(runtime_by_source
            .as_ref()
            .and_then(|runtime| runtime.placement));
    let runtime_id = if let Some(runtime_spec) = request.runtime_spec.as_ref() {
        let placement = placement.ok_or(CoreError::RuntimeSpecMismatch)?;
        let spec = runtime_spec_v1(runtime_spec);
        validate_runtime_spec_binding(
            runtime_spec,
            Some(&request.id),
            &project.id,
            &spec.agent_runtime_id,
            placement,
            &artifact,
        )?;
        if request.agent_runtime_id.as_deref() != Some(spec.agent_runtime_id.as_str())
            || runtime_by_source
                .as_ref()
                .is_some_and(|runtime| runtime.id != spec.agent_runtime_id)
        {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        spec.agent_runtime_id.clone()
    } else {
        runtime_by_source
            .as_ref()
            .map(|runtime| runtime.id.clone())
            .map(Ok)
            .unwrap_or_else(new_agent_runtime_id)?
    };
    let runtime_by_id = select_agent_runtime(client, &runtime_id).await?;
    let existing_runtime = existing_runtime.or(runtime_by_source).or(runtime_by_id);
    let (provider_runtime_handle, provider_runtime_handle_history) = merge_provider_runtime_handle(
        existing_runtime.as_ref(),
        input.provider_runtime_handle.clone(),
        placement,
    )?;
    let contact_endpoint = normalize_runtime_contact_endpoint(input.contact_endpoint.as_deref())?
        .or_else(|| existing_runtime.as_ref()?.contact_endpoint.clone());
    let bounded_runtime_capabilities =
        bound_runtime_capabilities_to_artifact(input.runtime_capabilities.clone(), &artifact);
    validate_runtime_capabilities_policy(bounded_runtime_capabilities.as_ref(), placement)?;
    let runtime_capabilities =
        merge_runtime_capabilities(existing_runtime.as_ref(), bounded_runtime_capabilities)?;
    let runtime = AgentRuntime {
        id: runtime_id.clone(),
        project_id: project.id.clone(),
        source_host_id: source_host_id.clone(),
        source_machine_id,
        source_import_key,
        runtime_artifact_id: Some(artifact.id),
        state_schema_version: Some(state_schema_version),
        placement,
        provider_runtime_handle,
        provider_runtime_handle_history,
        contact_endpoint,
        runtime_capabilities,
        host_facts: runtime_host_facts_from_complete_input(&input, &request, &source_host_id),
        created_at: existing_runtime
            .map(|runtime| runtime.created_at)
            .unwrap_or_else(|| now.clone()),
        updated_at: now.clone(),
    };
    let updated_provider_operation = provider_operation_at_runtime_boundary(
        provider_operation.as_ref(),
        runtime.provider_runtime_handle.as_ref(),
        true,
        provider_operation_now.as_deref().unwrap_or(&now),
    )?;
    if let Some(operation) = updated_provider_operation.as_ref() {
        let previous_len = provider_operation
            .as_ref()
            .map(|operation| operation.v1().transitions.len())
            .unwrap_or_default();
        // Completion may atomically cross both server-owned boundaries.
        for length in previous_len..operation.v1().transitions.len() {
            let partial = ProviderOperationEnvelope::V1(ProviderOperationV1 {
                agent_creation_request_id: operation.v1().agent_creation_request_id.clone(),
                correlation_id: operation.v1().correlation_id.clone(),
                placement: operation.v1().placement,
                transitions: operation.v1().transitions[..=length].to_vec(),
            });
            persist_provider_operation_delta(client, length, &partial).await?;
        }
    }
    let provider_operation_ack = if updated_provider_operation.is_some() {
        select_provider_operation(client, &input.request_id).await?
    } else {
        provider_operation.clone()
    };
    upsert_agent_runtime_row(client, &runtime).await?;
    activate_project_runtime_link(client, &project.id, &runtime_id, &now).await?;
    let request =
        update_agent_creation_completed(client, &input.request_id, &runtime_id, &now).await?;
    Ok(AgentCreationLease {
        project,
        request,
        provider_operation: provider_operation_ack,
    })
}

async fn postgres_fail_agent_creation_request<C>(
    client: &C,
    input: FailAgentCreationRequestInput,
) -> CoreResult<AgentCreationRequest>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let failure_message = trim_to_option(Some(&input.failure_message))
        .ok_or(CoreError::MissingAgentCreationFailureMessage)?;
    let request = locked_agent_creation_request(client, &input.request_id).await?;
    if let Some(operation) = select_provider_operation(client, &input.request_id).await? {
        verify_agent_creation_lease_active(client, &request, &input.runner_id, &input.lease_token)
            .await?;
        if !provider_operation_allows_generic_failure(&operation) {
            return Err(CoreError::ProviderOperationBoundaryNotReached);
        }
    } else {
        verify_agent_creation_lease(&request, &input.runner_id, &input.lease_token)?;
    }
    if let Some(key_id) = input.provisioned_finite_private_api_key_id.as_deref() {
        let key_id = trim_to_option(Some(key_id)).ok_or(CoreError::InvalidFinitePrivateApiKey)?;
        let key_row = client
            .query_opt(
                "SELECT id, grant_id, project_id, agent_runtime_id, key_hash, status,
                        created_at::text, updated_at::text
                 FROM finite_private_api_keys WHERE id = $1 FOR UPDATE",
                &[&key_id],
            )
            .await
            .map_err(store_error)?
            .ok_or(CoreError::InvalidFinitePrivateApiKey)?;
        let key = finite_private_api_key_from_row(&key_row)?;
        if key.project_id.as_deref() != Some(request.project_id.as_str()) {
            return Err(CoreError::InvalidFinitePrivateApiKey);
        }
        postgres_revoke_finite_private_api_key(
            client,
            RevokeFinitePrivateApiKeyInput {
                key_id,
                now: Some(now.clone()),
            },
        )
        .await?;
    }
    if let Some(runtime_id) = request.agent_runtime_id.as_deref() {
        delete_runtime_rows(client, runtime_id).await?;
    }
    let row = client
        .query_one(
            "UPDATE agent_creation_requests
             SET status = 'failed',
                 agent_runtime_id = NULL,
                 lease_token = NULL,
                 lease_expires_at = NULL,
                 failure_message = $2,
                 updated_at = $3::text::timestamptz
             WHERE id = $1
             RETURNING id, customer_org_id, owner_user_id, project_id, idempotency_key,
                       display_name, runner_class, hosting_tier, placement_runner_class,
                       runtime_resource_class, desired_runtime_artifact_id, runtime_spec,
                       profile_picture_url,
                       status, requested_launch_code, agent_runtime_id,
                       runner_id, lease_token, lease_expires_at::text, failure_message,
                       created_at::text, updated_at::text",
            &[&input.request_id, &failure_message, &now],
        )
        .await
        .map_err(store_error)?;
    agent_creation_request_from_row(&row)
}

async fn postgres_cancel_agent_creation_request<C>(
    client: &C,
    input: CancelAgentCreationRequestInput,
) -> CoreResult<AgentCreationRequest>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let request = locked_agent_creation_request(client, &input.request_id).await?;
    if request.status == AgentCreationRequestStatus::Running {
        return Err(CoreError::AgentCreationRequestNotCancellable);
    }
    if select_provider_operation(client, &input.request_id)
        .await?
        .is_some_and(|operation| !provider_operation_allows_generic_failure(&operation))
    {
        return Err(CoreError::ProviderOperationBoundaryNotReached);
    }
    // Cancellation is the final cleanup step for a failed or pre-provider
    // request. Revoke a project-scoped launch key even when a crashed runner
    // never named it in its failure acknowledgment. Ambiguous/post-mutation
    // operations returned above without touching keys or Runtime facts.
    let key_rows = client
        .query(
            "SELECT id FROM finite_private_api_keys
             WHERE project_id = $1 AND status = 'active'
             FOR UPDATE",
            &[&request.project_id],
        )
        .await
        .map_err(store_error)?;
    for key_id in key_rows.into_iter().map(|row| row.get::<_, String>("id")) {
        postgres_revoke_finite_private_api_key(
            client,
            RevokeFinitePrivateApiKeyInput {
                key_id,
                now: Some(now.clone()),
            },
        )
        .await?;
    }
    if let Some(runtime_id) = request.agent_runtime_id.as_deref() {
        delete_runtime_rows(client, runtime_id).await?;
    }
    let row = client
        .query_one(
            "UPDATE agent_creation_requests
             SET status = 'cancelled',
                 agent_runtime_id = NULL,
                 runner_id = NULL,
                 lease_token = NULL,
                 lease_expires_at = NULL,
                 failure_message = NULL,
                 updated_at = $2::text::timestamptz
             WHERE id = $1
             RETURNING id, customer_org_id, owner_user_id, project_id, idempotency_key,
                       display_name, runner_class, hosting_tier, placement_runner_class,
                       runtime_resource_class, desired_runtime_artifact_id, runtime_spec,
                       profile_picture_url,
                       status, requested_launch_code, agent_runtime_id,
                       runner_id, lease_token, lease_expires_at::text, failure_message,
                       created_at::text, updated_at::text",
            &[&input.request_id, &now],
        )
        .await
        .map_err(store_error)?;
    agent_creation_request_from_row(&row)
}

async fn postgres_provision_finite_private_runtime_key<C>(
    client: &C,
    input: ProvisionFinitePrivateRuntimeKeyInput,
) -> CoreResult<ProvisionFinitePrivateRuntimeKeyResult>
where
    C: GenericClient + Sync,
{
    let now = input.now.clone().unwrap_or(current_time_iso()?);
    let request = locked_agent_creation_request(client, &input.request_id).await?;
    verify_agent_creation_lease(&request, &input.runner_id, &input.lease_token)?;
    let project = select_project(client, &request.project_id)
        .await?
        .ok_or_else(|| missing_request_project_error(&request))?;
    let user = select_user_by_id(client, &request.owner_user_id)
        .await?
        .ok_or_else(|| {
            CoreError::Store(format!(
                "agent creation request {} references missing owner user {}",
                request.id, request.owner_user_id
            ))
        })?;
    let source_host_id = input
        .source_host_id
        .as_deref()
        .and_then(|value| trim_to_option(Some(value)))
        .map(|value| normalize_source_host_id(&value))
        .transpose()?;
    let source_machine_id = input
        .source_machine_id
        .as_deref()
        .and_then(|value| trim_to_option(Some(value)))
        .map(|value| {
            let normalized = normalize_id_part(&value);
            if normalized.is_empty() {
                Err(CoreError::MissingSourceMachineId)
            } else {
                Ok(normalized)
            }
        })
        .transpose()?;
    // Resolve the runtime to bind the key to by natural key (source_import_key)
    // rather than rederiving its id from the source identifiers.
    let agent_runtime_id = match (source_host_id.as_deref(), source_machine_id.as_deref()) {
        (Some(source_host_id), Some(source_machine_id)) => {
            let key = source_import_key(source_host_id, source_machine_id);
            select_agent_runtime_by_source_import_key(client, &key)
                .await?
                .map(|runtime| runtime.id)
        }
        _ => match request.agent_runtime_id.clone() {
            Some(runtime_id) if select_agent_runtime(client, &runtime_id).await?.is_some() => {
                Some(runtime_id)
            }
            _ => None,
        },
    };
    let grant = approve_finite_private_grant_row(
        client,
        &user,
        crate::DEFAULT_FINITE_PRIVATE_LIMIT_PROFILE,
        &now,
    )
    .await?;
    let raw_api_key = generate_finite_private_api_key()?;
    let api_key = issue_finite_private_api_key_row(
        client,
        &grant,
        &raw_api_key,
        Some(project.id),
        agent_runtime_id,
        &now,
    )
    .await?;
    Ok(ProvisionFinitePrivateRuntimeKeyResult {
        grant,
        api_key,
        raw_api_key,
    })
}

async fn postgres_revoke_finite_private_api_key<C>(
    client: &C,
    input: RevokeFinitePrivateApiKeyInput,
) -> CoreResult<FinitePrivateApiKey>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let key_id =
        trim_to_option(Some(&input.key_id)).ok_or(CoreError::InvalidFinitePrivateApiKey)?;
    let row = client
        .query_opt(
            "UPDATE finite_private_api_keys
             SET status = 'revoked', updated_at = $2::text::timestamptz
             WHERE id = $1
             RETURNING id, grant_id, project_id, agent_runtime_id, key_hash, status,
                       created_at::text, updated_at::text",
            &[&key_id, &now],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::InvalidFinitePrivateApiKey)?;
    let key = finite_private_api_key_from_row(&row)?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.api_key.revoke",
            target_type: "api_key",
            target_id: &key.id,
            grant_id: Some(&key.grant_id),
            api_key_id: Some(&key.id),
            actor: None,
            metadata: json!({}),
            now: &now,
        },
    )
    .await?;
    Ok(key)
}

async fn postgres_record_runtime_heartbeat<C>(
    client: &C,
    relay_token: &str,
) -> CoreResult<RelayHeartbeat>
where
    C: GenericClient + Sync,
{
    let now = current_time_iso()?;
    let token_hash = runtime_relay_token_hash(relay_token)?;
    let row = client
        .query_opt(
            "SELECT runtime.id, runtime.project_id, runtime.source_host_id,
                    runtime.source_machine_id, runtime.source_import_key,
                    runtime.runtime_artifact_id, runtime.state_schema_version,
                    runtime.placement_runner_class, runtime.runtime_resource_class,
                    runtime.provider_runtime_handle, runtime.provider_runtime_handle_history,
                    runtime.contact_endpoint, runtime.runtime_capabilities,
                    runtime.host_facts, runtime.created_at::text, runtime.updated_at::text
             FROM runtime_relay_credentials AS credential
             JOIN agent_runtimes AS runtime ON runtime.id = credential.agent_runtime_id
             WHERE credential.token_hash = $1
             FOR UPDATE OF runtime",
            &[&token_hash],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::InvalidRuntimeRelayToken)?;
    let mut runtime = agent_runtime_from_row(&row)?;
    runtime.host_facts.runtime_status = RuntimeSummaryStatus::Online;
    runtime.updated_at = now.clone();
    upsert_agent_runtime_row(client, &runtime).await?;
    upsert_runtime_status_snapshot_row(
        client,
        &RuntimeStatusSnapshot {
            agent_runtime_id: runtime.id.clone(),
            status: RuntimeSummaryStatus::Online,
            last_heartbeat_at: Some(now.clone()),
            runtime_host: runtime.host_facts.runtime_host.clone(),
            active_inference_profile: runtime.host_facts.active_inference_profile.clone(),
            hermes_available: runtime.host_facts.hermes_available,
            updated_at: now.clone(),
        },
    )
    .await?;
    Ok(RelayHeartbeat {
        ok: true,
        machine_id: runtime.source_machine_id,
        last_seen_at: now,
    })
}

async fn postgres_runtime_heartbeat_for_machine<C>(
    client: &C,
    source_machine_id: &str,
) -> CoreResult<RelayHeartbeat>
where
    C: GenericClient + Sync,
{
    let source_machine_id = normalize_id_part(source_machine_id);
    if source_machine_id.is_empty() {
        return Err(CoreError::MissingSourceMachineId);
    }
    let row = client
        .query_opt(
            "SELECT runtime.source_machine_id, snapshot.last_heartbeat_at::text
             FROM agent_runtimes AS runtime
             JOIN runtime_status_snapshots AS snapshot ON snapshot.agent_runtime_id = runtime.id
             WHERE runtime.source_machine_id = $1
               AND snapshot.status = 'online'
               AND snapshot.last_heartbeat_at IS NOT NULL
             ORDER BY snapshot.last_heartbeat_at DESC
             LIMIT 1",
            &[&source_machine_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::RuntimeHeartbeatNotFound)?;
    Ok(RelayHeartbeat {
        ok: true,
        machine_id: row.get("source_machine_id"),
        last_seen_at: row.get("last_heartbeat_at"),
    })
}

async fn postgres_visible_projects_for_workos_user<C>(
    client: &C,
    workos_user_id: &str,
) -> CoreResult<Vec<VisibleProject>>
where
    C: GenericClient + Sync,
{
    let Some(user_id) = client
        .query_opt(
            "SELECT id FROM users WHERE workos_user_id = $1",
            &[&workos_user_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| row.get::<_, String>("id"))
    else {
        return Ok(Vec::new());
    };
    let rows = client
        .query(
            "SELECT project.id AS project_id, project.customer_org_id, project.owner_user_id,
                    project.display_name, project.import_candidate_id, project.hosting_tier,
                    project.placement_runner_class, project.runtime_resource_class,
                    project.created_at::text,
                    project.updated_at::text,
                    runtime.id AS runtime_id, runtime.project_id AS runtime_project_id,
                    runtime.source_host_id, runtime.source_machine_id, runtime.source_import_key,
                    runtime.runtime_artifact_id, runtime.state_schema_version,
                    runtime.placement_runner_class AS runtime_placement_runner_class,
                    runtime.runtime_resource_class AS runtime_runtime_resource_class,
                    runtime.provider_runtime_handle, runtime.provider_runtime_handle_history,
                    runtime.contact_endpoint, runtime.runtime_capabilities,
                    runtime.host_facts, runtime.created_at::text AS runtime_created_at,
                    runtime.updated_at::text AS runtime_updated_at
             FROM project_room_memberships AS membership
             JOIN chat_identities AS identity ON identity.id = membership.chat_identity_id
             JOIN projects AS project ON project.id = membership.project_id
             LEFT JOIN project_runtime_links AS link
               ON link.project_id = project.id AND link.active
             LEFT JOIN agent_runtimes AS runtime ON runtime.id = link.agent_runtime_id
             WHERE identity.user_id = $1
               AND membership.archived_at IS NULL
               AND NOT EXISTS (
                 SELECT 1 FROM agent_creation_requests hidden
                 WHERE hidden.project_id = project.id
                   AND hidden.status = 'cancelled'
                   AND hidden.agent_runtime_id IS NULL
               )
             ORDER BY project.created_at, project.id",
            &[&user_id],
        )
        .await
        .map_err(store_error)?;
    rows.into_iter()
        .map(|row| {
            let project = Project {
                id: row.get("project_id"),
                customer_org_id: row.get("customer_org_id"),
                owner_user_id: row.get("owner_user_id"),
                display_name: row.get("display_name"),
                import_candidate_id: row.get("import_candidate_id"),
                hosting_tier: optional_hosting_tier_column(&row, "hosting_tier")?,
                placement: optional_runtime_placement_columns(
                    &row,
                    "placement_runner_class",
                    "runtime_resource_class",
                )?,
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            };
            let runtime = row
                .get::<_, Option<String>>("runtime_id")
                .map(|id| {
                    Ok::<AgentRuntime, CoreError>(AgentRuntime {
                        id,
                        project_id: row.get("runtime_project_id"),
                        source_host_id: row.get("source_host_id"),
                        source_machine_id: row.get("source_machine_id"),
                        source_import_key: row.get("source_import_key"),
                        runtime_artifact_id: row.get("runtime_artifact_id"),
                        state_schema_version: row.get("state_schema_version"),
                        placement: optional_runtime_placement_columns(
                            &row,
                            "runtime_placement_runner_class",
                            "runtime_runtime_resource_class",
                        )?,
                        provider_runtime_handle: optional_json_column(
                            &row,
                            "provider_runtime_handle",
                        )?
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(json_error)?,
                        provider_runtime_handle_history: optional_json_column(
                            &row,
                            "provider_runtime_handle_history",
                        )?
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(json_error)?
                        .unwrap_or_default(),
                        contact_endpoint: row.get("contact_endpoint"),
                        runtime_capabilities: optional_json_column(&row, "runtime_capabilities")?
                            .map(serde_json::from_value)
                            .transpose()
                            .map_err(json_error)?,
                        host_facts: json_column(&row, "host_facts")?,
                        created_at: row.get("runtime_created_at"),
                        updated_at: row.get("runtime_updated_at"),
                    })
                })
                .transpose()?;
            Ok(VisibleProject { project, runtime })
        })
        .collect()
}

async fn postgres_agent_creation_requests_for_workos_user<C>(
    client: &C,
    workos_user_id: &str,
) -> CoreResult<Vec<AgentCreationRequest>>
where
    C: GenericClient + Sync,
{
    let rows = client
        .query(
            "SELECT request.id, request.customer_org_id, request.owner_user_id,
                    request.project_id, request.idempotency_key, request.display_name,
                    request.runner_class, request.hosting_tier,
                    request.placement_runner_class, request.runtime_resource_class,
                    request.desired_runtime_artifact_id, request.runtime_spec,
                    request.profile_picture_url,
                    request.status, request.requested_launch_code, request.agent_runtime_id,
                    request.runner_id, request.lease_token, request.lease_expires_at::text,
                    request.failure_message, request.created_at::text, request.updated_at::text
             FROM agent_creation_requests AS request
             JOIN users AS owner ON owner.id = request.owner_user_id
             WHERE owner.workos_user_id = $1
             ORDER BY request.created_at, request.id",
            &[&workos_user_id],
        )
        .await
        .map_err(store_error)?;
    rows.iter().map(agent_creation_request_from_row).collect()
}

fn core_user_from_row(row: &Row) -> CoreResult<CoreUser> {
    let status: String = row.get("link_status");
    Ok(CoreUser {
        id: row.get("id"),
        email: row.get("normalized_email"),
        status: parse_user_link_status(&status)
            .ok_or_else(|| CoreError::Store(format!("invalid user link status {status}")))?,
        workos_user_id: row.get("workos_user_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn customer_org_from_row(row: &Row) -> CoreResult<CustomerOrganization> {
    let billing_class: String = row.get("billing_class");
    Ok(CustomerOrganization {
        id: row.get("id"),
        owner_user_id: row.get("owner_user_id"),
        name: row.get("name"),
        billing_class: parse_billing_class(&billing_class)
            .ok_or_else(|| CoreError::Store(format!("invalid billing class {billing_class}")))?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn optional_hosting_tier_column(row: &Row, name: &str) -> CoreResult<Option<HostingTier>> {
    let value: Option<String> = row.get(name);
    value
        .map(|value| {
            parse_hosting_tier(&value)
                .ok_or_else(|| CoreError::Store(format!("invalid hosting tier {value}")))
        })
        .transpose()
}

fn optional_runtime_placement_columns(
    row: &Row,
    runner_name: &str,
    resource_name: &str,
) -> CoreResult<Option<RuntimePlacement>> {
    let runner: Option<String> = row.get(runner_name);
    let resource: Option<String> = row.get(resource_name);
    match (runner, resource) {
        (None, None) => Ok(None),
        (Some(runner), Some(resource)) => Ok(Some(RuntimePlacement {
            runner_class: parse_runner_class(&runner)
                .ok_or_else(|| CoreError::Store(format!("invalid agent runner class {runner}")))?,
            runtime_resource_class: parse_runtime_resource_class(&resource).ok_or_else(|| {
                CoreError::Store(format!("invalid runtime resource class {resource}"))
            })?,
        })),
        _ => Err(CoreError::Store(
            "incomplete persisted runtime placement".to_string(),
        )),
    }
}

fn customer_billing_account_from_row(row: &Row) -> CoreResult<CustomerBillingAccount> {
    let status: Option<String> = row.get("subscription_status");
    Ok(CustomerBillingAccount {
        customer_org_id: row.get("customer_org_id"),
        hosting_tier: optional_hosting_tier_column(row, "hosting_tier")?,
        stripe_customer_id: row.get("stripe_customer_id"),
        stripe_subscription_id: row.get("stripe_subscription_id"),
        stripe_price_id: row.get("stripe_price_id"),
        subscription_status: status
            .as_deref()
            .map(|value| {
                parse_billing_subscription_status(value).ok_or_else(|| {
                    CoreError::Store(format!("invalid billing subscription status {value}"))
                })
            })
            .transpose()?,
        current_period_end: row.get("current_period_end"),
        cancel_at_period_end: row.get("cancel_at_period_end"),
        last_stripe_event_id: row.get("last_stripe_event_id"),
        last_stripe_event_created: row.get("last_stripe_event_created"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn agent_creation_entitlement_from_row(row: &Row) -> CoreResult<AgentCreationEntitlement> {
    Ok(AgentCreationEntitlement {
        id: row.get("id"),
        customer_org_id: row.get("customer_org_id"),
        hosting_tier: optional_hosting_tier_column(row, "hosting_tier")?,
        allowed_new_agent_runtimes: row.get("allowed_new_agent_runtimes"),
        launch_code: row.get("launch_code"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn project_from_row(row: &Row) -> CoreResult<Project> {
    Ok(Project {
        id: row.get("id"),
        customer_org_id: row.get("customer_org_id"),
        owner_user_id: row.get("owner_user_id"),
        display_name: row.get("display_name"),
        import_candidate_id: row.get("import_candidate_id"),
        hosting_tier: optional_hosting_tier_column(row, "hosting_tier")?,
        placement: optional_runtime_placement_columns(
            row,
            "placement_runner_class",
            "runtime_resource_class",
        )?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn agent_creation_request_from_row(row: &Row) -> CoreResult<AgentCreationRequest> {
    let status: String = row.get("status");
    let runner_class: String = row.get("runner_class");
    let runtime_spec = optional_json_column(row, "runtime_spec")?;
    Ok(AgentCreationRequest {
        id: row.get("id"),
        customer_org_id: row.get("customer_org_id"),
        owner_user_id: row.get("owner_user_id"),
        project_id: row.get("project_id"),
        idempotency_key: row.get("idempotency_key"),
        display_name: row.get("display_name"),
        runner_class: parse_runner_class(&runner_class).ok_or_else(|| {
            CoreError::Store(format!("invalid agent runner class {runner_class}"))
        })?,
        hosting_tier: optional_hosting_tier_column(row, "hosting_tier")?,
        placement: optional_runtime_placement_columns(
            row,
            "placement_runner_class",
            "runtime_resource_class",
        )?,
        desired_runtime_artifact_id: row.get("desired_runtime_artifact_id"),
        runtime_spec: runtime_spec
            .map(serde_json::from_value)
            .transpose()
            .map_err(json_error)?,
        profile_picture_url: row.get("profile_picture_url"),
        status: parse_agent_creation_request_status(&status).ok_or_else(|| {
            CoreError::Store(format!("invalid agent creation request status {status}"))
        })?,
        requested_launch_code: row.get("requested_launch_code"),
        agent_runtime_id: row.get("agent_runtime_id"),
        runner_id: row.get("runner_id"),
        lease_token: row.get("lease_token"),
        lease_expires_at: row.get("lease_expires_at"),
        failure_message: row.get("failure_message"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn runtime_artifact_from_row(row: &Row) -> CoreResult<RuntimeArtifact> {
    let kind: String = row.get("kind");
    Ok(RuntimeArtifact {
        id: row.get("id"),
        kind: parse_runtime_artifact_kind(&kind)
            .ok_or_else(|| CoreError::Store(format!("invalid runtime artifact kind {kind}")))?,
        reference: row.get("reference"),
        version_label: row.get("version_label"),
        source_git_sha: row.get("source_git_sha"),
        finitec_version: row.get("finitec_version"),
        hermes_source_ref: row.get("hermes_source_ref"),
        finite_platform_plugin_ref: row.get("finite_platform_plugin_ref"),
        state_schema_version: row.get("state_schema_version"),
        base_image: row.get("base_image"),
        recover_known_good_chat: row.get("recover_known_good_chat"),
        created_at: row.get("created_at"),
        promoted_at: row.get("promoted_at"),
        retired_at: row.get("retired_at"),
    })
}

fn agent_runtime_from_row(row: &Row) -> CoreResult<AgentRuntime> {
    let provider_runtime_handle = optional_json_column(row, "provider_runtime_handle")?;
    let provider_runtime_handle_history =
        optional_json_column(row, "provider_runtime_handle_history")?;
    let runtime_capabilities = optional_json_column(row, "runtime_capabilities")?;
    Ok(AgentRuntime {
        id: row.get("id"),
        project_id: row.get("project_id"),
        source_host_id: row.get("source_host_id"),
        source_machine_id: row.get("source_machine_id"),
        source_import_key: row.get("source_import_key"),
        runtime_artifact_id: row.get("runtime_artifact_id"),
        state_schema_version: row.get("state_schema_version"),
        placement: optional_runtime_placement_columns(
            row,
            "placement_runner_class",
            "runtime_resource_class",
        )?,
        provider_runtime_handle: provider_runtime_handle
            .map(serde_json::from_value)
            .transpose()
            .map_err(json_error)?,
        provider_runtime_handle_history: provider_runtime_handle_history
            .map(serde_json::from_value)
            .transpose()
            .map_err(json_error)?
            .unwrap_or_default(),
        contact_endpoint: row.get("contact_endpoint"),
        runtime_capabilities: runtime_capabilities
            .map(serde_json::from_value)
            .transpose()
            .map_err(json_error)?,
        host_facts: json_column(row, "host_facts")?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn finite_private_grant_from_row(row: &Row) -> CoreResult<FinitePrivateGrant> {
    let status: String = row.get("status");
    Ok(FinitePrivateGrant {
        id: row.get("id"),
        user_id: row.get("user_id"),
        limit_profile_id: row.get("limit_profile_id"),
        status: parse_finite_private_grant_status(&status).ok_or_else(|| {
            CoreError::Store(format!("invalid finite private grant status {status}"))
        })?,
        current_window_started_at: row.get("current_window_started_at"),
        current_window_used_units: row.get("current_window_used_units"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn finite_private_api_key_from_row(row: &Row) -> CoreResult<FinitePrivateApiKey> {
    let status: String = row.get("status");
    Ok(FinitePrivateApiKey {
        id: row.get("id"),
        grant_id: row.get("grant_id"),
        project_id: row.get("project_id"),
        agent_runtime_id: row.get("agent_runtime_id"),
        key_hash: row.get("key_hash"),
        status: parse_finite_private_api_key_status(&status).ok_or_else(|| {
            CoreError::Store(format!("invalid finite private API key status {status}"))
        })?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

async fn select_user_by_id<C>(client: &C, user_id: &str) -> CoreResult<Option<CoreUser>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, normalized_email, link_status, workos_user_id,
                    created_at::text, updated_at::text
             FROM users WHERE id = $1",
            &[&user_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| core_user_from_row(&row))
        .transpose()
}

/// Resolve a user by their natural key (`users.normalized_email UNIQUE`). This
/// replaces the old `user_id = f(email)` derivation: identity is looked up, not
/// reconstructed, so a re-signup after a wipe finds nothing and mints a fresh id.
async fn select_user_by_email<C>(client: &C, email: &str) -> CoreResult<Option<CoreUser>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, normalized_email, link_status, workos_user_id,
                    created_at::text, updated_at::text
             FROM users WHERE normalized_email = $1",
            &[&email],
        )
        .await
        .map_err(store_error)?
        .map(|row| core_user_from_row(&row))
        .transpose()
}

/// Resolve the one personal org for an owner via the
/// `customer_orgs_one_personal_org_per_owner` unique index.
async fn select_personal_org_by_owner<C>(
    client: &C,
    owner_user_id: &str,
) -> CoreResult<Option<CustomerOrganization>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, owner_user_id, name, billing_class, created_at::text, updated_at::text
             FROM customer_orgs WHERE owner_user_id = $1",
            &[&owner_user_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| customer_org_from_row(&row))
        .transpose()
}

async fn select_project<C>(client: &C, project_id: &str) -> CoreResult<Option<Project>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, customer_org_id, owner_user_id, display_name, import_candidate_id,
                    hosting_tier, placement_runner_class, runtime_resource_class,
                    created_at::text, updated_at::text
             FROM projects WHERE id = $1",
            &[&project_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| project_from_row(&row))
        .transpose()
}

/// Idempotency lookup by the natural key `(owner_user_id, idempotency_key)` —
/// the same tuple the `agent_creation_requests` UNIQUE constraint enforces. The
/// request's primary key is a surrogate, so dedupe is done by looking the row up
/// here, never by rederiving the id from the idempotency inputs.
async fn select_agent_creation_request_by_idempotency<C>(
    client: &C,
    owner_user_id: &str,
    idempotency_key: &str,
) -> CoreResult<Option<AgentCreationRequest>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, customer_org_id, owner_user_id, project_id, idempotency_key,
                    display_name, runner_class, hosting_tier, placement_runner_class,
                    runtime_resource_class, desired_runtime_artifact_id, runtime_spec,
                    profile_picture_url,
                    status, requested_launch_code, agent_runtime_id,
                    runner_id, lease_token, lease_expires_at::text, failure_message,
                    created_at::text, updated_at::text
             FROM agent_creation_requests
             WHERE owner_user_id = $1 AND idempotency_key = $2",
            &[&owner_user_id, &idempotency_key],
        )
        .await
        .map_err(store_error)?
        .map(|row| agent_creation_request_from_row(&row))
        .transpose()
}

async fn locked_agent_creation_request<C>(
    client: &C,
    request_id: &str,
) -> CoreResult<AgentCreationRequest>
where
    C: GenericClient + Sync,
{
    let row = client
        .query_opt(
            "SELECT id, customer_org_id, owner_user_id, project_id, idempotency_key,
                    display_name, runner_class, hosting_tier, placement_runner_class,
                    runtime_resource_class, desired_runtime_artifact_id, runtime_spec,
                    profile_picture_url,
                    status, requested_launch_code, agent_runtime_id,
                    runner_id, lease_token, lease_expires_at::text, failure_message,
                    created_at::text, updated_at::text
             FROM agent_creation_requests WHERE id = $1
             FOR UPDATE",
            &[&request_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::AgentCreationRequestNotFound)?;
    agent_creation_request_from_row(&row)
}

async fn select_runtime_artifact<C>(
    client: &C,
    artifact_id: &str,
) -> CoreResult<Option<RuntimeArtifact>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, kind, reference, version_label, source_git_sha, finitec_version,
                    hermes_source_ref, finite_platform_plugin_ref, state_schema_version,
                    base_image, recover_known_good_chat,
                    created_at::text, promoted_at::text, retired_at::text
             FROM runtime_artifacts WHERE id = $1",
            &[&artifact_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| runtime_artifact_from_row(&row))
        .transpose()
}

async fn select_latest_launchable_runtime_artifact<C>(client: &C) -> CoreResult<RuntimeArtifact>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, kind, reference, version_label, source_git_sha, finitec_version,
                    hermes_source_ref, finite_platform_plugin_ref, state_schema_version,
                    base_image, recover_known_good_chat,
                    created_at::text, promoted_at::text, retired_at::text
             FROM runtime_artifacts
             WHERE promoted_at IS NOT NULL AND retired_at IS NULL AND kind = 'oci_image'
             ORDER BY promoted_at DESC, created_at DESC, id DESC
             LIMIT 1",
            &[],
        )
        .await
        .map_err(store_error)?
        .map(|row| runtime_artifact_from_row(&row))
        .transpose()?
        .filter(|artifact| runtime_artifact_reference_is_immutable_oci(&artifact.reference))
        .ok_or(CoreError::RuntimeArtifactUnavailable)
}

async fn select_agent_runtime<C>(client: &C, runtime_id: &str) -> CoreResult<Option<AgentRuntime>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, project_id, source_host_id, source_machine_id, source_import_key,
                    runtime_artifact_id, state_schema_version, placement_runner_class,
                    runtime_resource_class, provider_runtime_handle,
                    provider_runtime_handle_history, contact_endpoint, runtime_capabilities,
                    host_facts,
                    created_at::text, updated_at::text
             FROM agent_runtimes WHERE id = $1",
            &[&runtime_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| agent_runtime_from_row(&row))
        .transpose()
}

/// Resolve a runtime by its natural key (`agent_runtimes.source_import_key`
/// UNIQUE). Registration/completion for the same source reuse this row's
/// surrogate id instead of rederiving an id from the host/machine identifiers.
async fn select_agent_runtime_by_source_import_key<C>(
    client: &C,
    source_import_key: &str,
) -> CoreResult<Option<AgentRuntime>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT id, project_id, source_host_id, source_machine_id, source_import_key,
                    runtime_artifact_id, state_schema_version, placement_runner_class,
                    runtime_resource_class, provider_runtime_handle,
                    provider_runtime_handle_history, contact_endpoint, runtime_capabilities,
                    host_facts,
                    created_at::text, updated_at::text
             FROM agent_runtimes WHERE source_import_key = $1",
            &[&source_import_key],
        )
        .await
        .map_err(store_error)?
        .map(|row| agent_runtime_from_row(&row))
        .transpose()
}

async fn postgres_list_launch_code_batches<C>(client: &C) -> CoreResult<Vec<LaunchCodeBatchDetails>>
where
    C: GenericClient + Sync,
{
    let rows = client
        .query(
            "SELECT id, name, hosting_tier, code_count, expires_at::text, revoked_at::text,
                    revoked_by_workos_user_id, created_by_workos_user_id,
                    created_at::text
               FROM launch_code_batches
              ORDER BY created_at DESC, id DESC",
            &[],
        )
        .await
        .map_err(store_error)?;
    let mut details = Vec::with_capacity(rows.len());
    for row in rows {
        details.push(
            postgres_launch_code_batch_details(client, launch_code_batch_from_row(&row)?).await?,
        );
    }
    Ok(details)
}

async fn postgres_launch_code_batch_details<C>(
    client: &C,
    batch: LaunchCodeBatch,
) -> CoreResult<LaunchCodeBatchDetails>
where
    C: GenericClient + Sync,
{
    let rows = client
        .query(
            "SELECT id, redeemed_customer_org_id, redeemed_at::text
               FROM launch_codes
              WHERE batch_id = $1
              ORDER BY id",
            &[&batch.id],
        )
        .await
        .map_err(store_error)?;
    let codes = rows
        .into_iter()
        .map(|row| LaunchCodeStatus {
            id: row.get("id"),
            redeemed_customer_org_id: row.get("redeemed_customer_org_id"),
            redeemed_at: row.get("redeemed_at"),
        })
        .collect();
    Ok(LaunchCodeBatchDetails { batch, codes })
}

fn launch_code_batch_from_row(row: &Row) -> CoreResult<LaunchCodeBatch> {
    let count: i32 = row.get("code_count");
    Ok(LaunchCodeBatch {
        id: row.get("id"),
        name: row.get("name"),
        hosting_tier: optional_hosting_tier_column(row, "hosting_tier")?,
        code_count: u32::try_from(count).map_err(|_| CoreError::InvalidLaunchCodeBatchSize)?,
        expires_at: row.get("expires_at"),
        revoked_at: row.get("revoked_at"),
        revoked_by_workos_user_id: row.get("revoked_by_workos_user_id"),
        created_by_workos_user_id: row.get("created_by_workos_user_id"),
        created_at: row.get("created_at"),
    })
}

struct LockedLaunchCode {
    record: LaunchCodeRecord,
    hosting_tier: Option<HostingTier>,
}

async fn lock_postgres_launch_code<C>(
    client: &C,
    launch_code: &str,
    now: &str,
) -> CoreResult<LockedLaunchCode>
where
    C: GenericClient + Sync,
{
    let code_hash = hash_launch_code(launch_code)?;
    parse_time(now)?;
    let row = client
        .query_opt(
            "SELECT code.id, code.batch_id, code.code_hash,
                    code.redeemed_customer_org_id,
                    code.redemption_idempotency_key, code.redeemed_at::text,
                    code.created_at::text,
                    batch.hosting_tier,
                    batch.revoked_at IS NOT NULL AS batch_revoked,
                    batch.expires_at <= $2::text::timestamptz AS batch_expired
              FROM launch_codes AS code
               JOIN launch_code_batches AS batch ON batch.id = code.batch_id
              WHERE code.code_hash = $1
              FOR UPDATE OF code, batch",
            &[&code_hash, &now],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::InvalidLaunchCode)?;
    let batch_revoked: bool = row.get("batch_revoked");
    let batch_expired: bool = row.get("batch_expired");
    let redeemed_customer_org_id: Option<String> = row.get("redeemed_customer_org_id");
    if redeemed_customer_org_id.is_none() && (batch_revoked || batch_expired) {
        return Err(CoreError::InvalidLaunchCode);
    }
    Ok(LockedLaunchCode {
        hosting_tier: optional_hosting_tier_column(&row, "hosting_tier")?,
        record: LaunchCodeRecord {
            id: row.get("id"),
            batch_id: row.get("batch_id"),
            code_hash: row.get("code_hash"),
            redeemed_customer_org_id,
            redemption_idempotency_key: row.get("redemption_idempotency_key"),
            redeemed_at: row.get("redeemed_at"),
            created_at: row.get("created_at"),
        },
    })
}

async fn redeem_postgres_launch_code<C>(
    client: &C,
    launch_code_id: &str,
    customer_org_id: &str,
    idempotency_key: &str,
    now: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let updated = client
        .execute(
            "UPDATE launch_codes
                SET redeemed_customer_org_id = $2,
                    redemption_idempotency_key = $3,
                    redeemed_at = $4::text::timestamptz
              WHERE id = $1
                AND redeemed_customer_org_id IS NULL
                AND redemption_idempotency_key IS NULL
                AND redeemed_at IS NULL",
            &[&launch_code_id, &customer_org_id, &idempotency_key, &now],
        )
        .await
        .map_err(store_error)?;
    if updated != 1 {
        return Err(CoreError::InvalidLaunchCode);
    }
    Ok(())
}

/// Find-or-create the linked user by their natural key. The conflict target is
/// `normalized_email` (UNIQUE), so an existing row keeps its already-minted
/// surrogate id and we only relink workos/status; a brand-new email gets a
/// fresh `new_user_id()`. The primary key is NEVER derived from the email.
async fn upsert_linked_user<C>(
    client: &C,
    email: &str,
    workos_user_id: &str,
    now: &str,
) -> CoreResult<CoreUser>
where
    C: GenericClient + Sync,
{
    let user_id = new_user_id()?;
    let row = client
        .query_one(
            "INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at)
             VALUES ($1, $2, 'linked', $3, $4::text::timestamptz, $4::text::timestamptz)
             ON CONFLICT (normalized_email) DO UPDATE SET
               link_status = 'linked',
               workos_user_id = EXCLUDED.workos_user_id,
               updated_at = EXCLUDED.updated_at
             RETURNING id, normalized_email, link_status, workos_user_id,
                       created_at::text, updated_at::text",
            &[&user_id, &email, &workos_user_id, &now],
        )
        .await
        .map_err(store_error)?;
    core_user_from_row(&row)
}

async fn ensure_personal_org_row<C>(
    client: &C,
    user: &CoreUser,
    billing_class: BillingClass,
    now: &str,
) -> CoreResult<CustomerOrganization>
where
    C: GenericClient + Sync,
{
    // Fresh surrogate id on insert; ON CONFLICT (owner_user_id) keeps the
    // existing org's id so the one-personal-org-per-owner invariant holds.
    let org_id = new_customer_org_id()?;
    let row = client
        .query_one(
            "INSERT INTO customer_orgs (id, owner_user_id, name, billing_class, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5::text::timestamptz, $5::text::timestamptz)
             ON CONFLICT (owner_user_id) DO UPDATE SET updated_at = customer_orgs.updated_at
             RETURNING id, owner_user_id, name, billing_class, created_at::text, updated_at::text",
            &[&org_id, &user.id, &user.email, &billing_class.as_str(), &now],
        )
        .await
        .map_err(store_error)?;
    customer_org_from_row(&row)
}

async fn grant_launch_code_agent_creation_entitlement_row<C>(
    client: &C,
    customer_org_id: &str,
    launch_code_id: &str,
    hosting_tier: HostingTier,
    now: &str,
) -> CoreResult<AgentCreationEntitlement>
where
    C: GenericClient + Sync,
{
    let id = agent_creation_entitlement_id_for(customer_org_id);
    let row = client
        .query_one(
            "INSERT INTO agent_creation_entitlements
               (id, customer_org_id, hosting_tier, allowed_new_agent_runtimes, launch_code, created_at, updated_at)
             VALUES ($1, $2, $3, 1, $4, $5::text::timestamptz, $5::text::timestamptz)
             ON CONFLICT (customer_org_id) DO UPDATE SET
               allowed_new_agent_runtimes = agent_creation_entitlements.allowed_new_agent_runtimes + 1,
               hosting_tier = EXCLUDED.hosting_tier,
               launch_code = EXCLUDED.launch_code,
               updated_at = EXCLUDED.updated_at
             RETURNING id, customer_org_id, hosting_tier, allowed_new_agent_runtimes, launch_code,
                       created_at::text, updated_at::text",
            &[
                &id,
                &customer_org_id,
                &hosting_tier.as_str(),
                &launch_code_id,
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    agent_creation_entitlement_from_row(&row)
}

async fn ensure_standard_agent_creation_entitlement_row<C>(
    client: &C,
    customer_org_id: &str,
    now: &str,
) -> CoreResult<AgentCreationEntitlement>
where
    C: GenericClient + Sync,
{
    let id = agent_creation_entitlement_id_for(customer_org_id);
    let row = client
        .query_one(
            "INSERT INTO agent_creation_entitlements
               (id, customer_org_id, hosting_tier, allowed_new_agent_runtimes, launch_code, created_at, updated_at)
             VALUES ($1, $2, 'standard', 1, NULL, $3::text::timestamptz, $3::text::timestamptz)
             ON CONFLICT (customer_org_id) DO UPDATE SET
               allowed_new_agent_runtimes = GREATEST(
                 agent_creation_entitlements.allowed_new_agent_runtimes,
                 EXCLUDED.allowed_new_agent_runtimes
               ),
               launch_code = agent_creation_entitlements.launch_code,
               updated_at = EXCLUDED.updated_at
             RETURNING id, customer_org_id, hosting_tier, allowed_new_agent_runtimes, launch_code,
                       created_at::text, updated_at::text",
            &[&id, &customer_org_id, &now],
        )
        .await
        .map_err(store_error)?;
    agent_creation_entitlement_from_row(&row)
}

async fn ensure_hosted_web_membership_row<C>(
    client: &C,
    user: &CoreUser,
    project_id: &str,
    now: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let identity_id = chat_identity_id_for_user(&user.id);
    client
        .execute(
            "INSERT INTO chat_identities (id, user_id, kind, device_id, created_at)
             VALUES ($1, $2, 'hosted_web', 'dashboard-bridge-v1', $3::text::timestamptz)
             ON CONFLICT (id) DO NOTHING",
            &[&identity_id, &user.id, &now],
        )
        .await
        .map_err(store_error)?;
    let membership_id = project_room_membership_id_for(project_id, &identity_id);
    client
        .execute(
            "INSERT INTO project_room_memberships (id, project_id, chat_identity_id, role, created_at)
             VALUES ($1, $2, $3, $4, $5::text::timestamptz)
             ON CONFLICT (id) DO NOTHING",
            &[
                &membership_id,
                &project_id,
                &identity_id,
                &ProjectMembershipRole::Owner.as_str(),
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

async fn upsert_project_row<C>(client: &C, project: &Project) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let placement_runner_class = project
        .placement
        .map(|placement| placement.runner_class.as_str());
    let runtime_resource_class = project
        .placement
        .map(|placement| placement.runtime_resource_class.as_str());
    client
        .execute(
            "INSERT INTO projects
               (id, customer_org_id, owner_user_id, display_name, import_candidate_id,
                hosting_tier, placement_runner_class, runtime_resource_class, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8,
                     $9::text::timestamptz, $10::text::timestamptz)
             ON CONFLICT (id) DO UPDATE SET
               display_name = EXCLUDED.display_name,
               hosting_tier = EXCLUDED.hosting_tier,
               placement_runner_class = EXCLUDED.placement_runner_class,
               runtime_resource_class = EXCLUDED.runtime_resource_class,
               updated_at = EXCLUDED.updated_at",
            &[
                &project.id,
                &project.customer_org_id,
                &project.owner_user_id,
                &project.display_name,
                &project.import_candidate_id,
                &project.hosting_tier.map(HostingTier::as_str),
                &placement_runner_class,
                &runtime_resource_class,
                &project.created_at,
                &project.updated_at,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

async fn upsert_agent_creation_request_row<C>(
    client: &C,
    request: &AgentCreationRequest,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let placement_runner_class = request
        .placement
        .map(|placement| placement.runner_class.as_str());
    let runtime_resource_class = request
        .placement
        .map(|placement| placement.runtime_resource_class.as_str());
    let runtime_spec = request
        .runtime_spec
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(json_error)?;
    client
        .execute(
            "INSERT INTO agent_creation_requests (
               id, customer_org_id, owner_user_id, project_id, idempotency_key, display_name,
               runner_class, hosting_tier, placement_runner_class, runtime_resource_class,
               desired_runtime_artifact_id, runtime_spec, profile_picture_url, status, requested_launch_code,
               agent_runtime_id, runner_id, lease_token,
               lease_expires_at, failure_message, created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::jsonb,
                     $13, $14, $15, $16, $17, $18, $19::text::timestamptz, $20,
                     $21::text::timestamptz, $22::text::timestamptz)
             ON CONFLICT (id) DO UPDATE SET
               status = EXCLUDED.status,
               display_name = EXCLUDED.display_name,
               runner_class = EXCLUDED.runner_class,
               hosting_tier = EXCLUDED.hosting_tier,
               placement_runner_class = EXCLUDED.placement_runner_class,
               runtime_resource_class = EXCLUDED.runtime_resource_class,
               desired_runtime_artifact_id = EXCLUDED.desired_runtime_artifact_id,
               runtime_spec = EXCLUDED.runtime_spec,
               profile_picture_url = EXCLUDED.profile_picture_url,
               agent_runtime_id = EXCLUDED.agent_runtime_id,
               runner_id = EXCLUDED.runner_id,
               lease_token = EXCLUDED.lease_token,
               lease_expires_at = EXCLUDED.lease_expires_at,
               failure_message = EXCLUDED.failure_message,
               updated_at = EXCLUDED.updated_at",
            &[
                &request.id,
                &request.customer_org_id,
                &request.owner_user_id,
                &request.project_id,
                &request.idempotency_key,
                &request.display_name,
                &request.runner_class.as_str(),
                &request.hosting_tier.map(HostingTier::as_str),
                &placement_runner_class,
                &runtime_resource_class,
                &request.desired_runtime_artifact_id,
                &runtime_spec,
                &request.profile_picture_url,
                &request.status.as_str(),
                &request.requested_launch_code,
                &request.agent_runtime_id,
                &request.runner_id,
                &request.lease_token,
                &request.lease_expires_at,
                &request.failure_message,
                &request.created_at,
                &request.updated_at,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

async fn upsert_agent_runtime_row<C>(client: &C, runtime: &AgentRuntime) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let host_facts = serde_json::to_value(&runtime.host_facts).map_err(json_error)?;
    let placement_runner_class = runtime
        .placement
        .map(|placement| placement.runner_class.as_str());
    let runtime_resource_class = runtime
        .placement
        .map(|placement| placement.runtime_resource_class.as_str());
    let provider_runtime_handle = runtime
        .provider_runtime_handle
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(json_error)?;
    let provider_runtime_handle_history =
        serde_json::to_value(&runtime.provider_runtime_handle_history).map_err(json_error)?;
    let runtime_capabilities = runtime
        .runtime_capabilities
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(json_error)?;
    client
        .execute(
            "INSERT INTO agent_runtimes (
               id, project_id, source_host_id, source_machine_id, source_import_key,
               runtime_artifact_id, state_schema_version, placement_runner_class,
               runtime_resource_class, provider_runtime_handle,
               provider_runtime_handle_history, contact_endpoint, runtime_capabilities,
               host_facts, created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::jsonb, $11::jsonb,
                     $12, $13::jsonb, $14::jsonb, $15::text::timestamptz,
                     $16::text::timestamptz)
             ON CONFLICT (id) DO UPDATE SET
               project_id = EXCLUDED.project_id,
               runtime_artifact_id = EXCLUDED.runtime_artifact_id,
               state_schema_version = EXCLUDED.state_schema_version,
               placement_runner_class = EXCLUDED.placement_runner_class,
               runtime_resource_class = EXCLUDED.runtime_resource_class,
               provider_runtime_handle = EXCLUDED.provider_runtime_handle,
               provider_runtime_handle_history = EXCLUDED.provider_runtime_handle_history,
               contact_endpoint = EXCLUDED.contact_endpoint,
               runtime_capabilities = EXCLUDED.runtime_capabilities,
               host_facts = EXCLUDED.host_facts,
               updated_at = EXCLUDED.updated_at",
            &[
                &runtime.id,
                &runtime.project_id,
                &runtime.source_host_id,
                &runtime.source_machine_id,
                &runtime.source_import_key,
                &runtime.runtime_artifact_id,
                &runtime.state_schema_version,
                &placement_runner_class,
                &runtime_resource_class,
                &provider_runtime_handle,
                &provider_runtime_handle_history,
                &runtime.contact_endpoint,
                &runtime_capabilities,
                &host_facts,
                &runtime.created_at,
                &runtime.updated_at,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

async fn upsert_runtime_relay_credential_row<C>(
    client: &C,
    credential: &RuntimeRelayCredential,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    client
        .execute(
            "INSERT INTO runtime_relay_credentials (agent_runtime_id, token_hash, created_at, updated_at)
             VALUES ($1, $2, $3::text::timestamptz, $4::text::timestamptz)
             ON CONFLICT (agent_runtime_id) DO UPDATE SET
               token_hash = EXCLUDED.token_hash,
               updated_at = EXCLUDED.updated_at",
            &[
                &credential.agent_runtime_id,
                &credential.token_hash,
                &credential.created_at,
                &credential.updated_at,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

async fn upsert_runtime_status_snapshot_row<C>(
    client: &C,
    snapshot: &RuntimeStatusSnapshot,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    client
        .execute(
            "INSERT INTO runtime_status_snapshots (
               agent_runtime_id, status, last_heartbeat_at, runtime_host,
               active_inference_profile, hermes_available, updated_at
             )
             VALUES ($1, $2, $3::text::timestamptz, $4, $5, $6, $7::text::timestamptz)
             ON CONFLICT (agent_runtime_id) DO UPDATE SET
               status = EXCLUDED.status,
               last_heartbeat_at = EXCLUDED.last_heartbeat_at,
               runtime_host = EXCLUDED.runtime_host,
               active_inference_profile = EXCLUDED.active_inference_profile,
               hermes_available = EXCLUDED.hermes_available,
               updated_at = EXCLUDED.updated_at",
            &[
                &snapshot.agent_runtime_id,
                &snapshot.status.as_str(),
                &snapshot.last_heartbeat_at,
                &snapshot.runtime_host,
                &snapshot.active_inference_profile,
                &snapshot.hermes_available,
                &snapshot.updated_at,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

/// Mirror `ensure_finite_private_limit_profile`: an existing profile is
/// returned; the DEFAULT profile is created on demand (with its weekly limit,
/// matching the in-memory spec); any other missing profile is an error.
async fn ensure_finite_private_limit_profile_row<C>(
    client: &C,
    id: &str,
    now: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    if client
        .query_opt(
            "SELECT id FROM finite_private_limit_profiles WHERE id = $1",
            &[&id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Ok(());
    }
    if id != crate::DEFAULT_FINITE_PRIVATE_LIMIT_PROFILE {
        return Err(CoreError::FinitePrivateLimitProfileNotFound);
    }
    client
        .execute(
            "INSERT INTO finite_private_limit_profiles (
               id, burst_window_seconds, burst_limit_units, weekly_limit_units, created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5::text::timestamptz, $5::text::timestamptz)
             ON CONFLICT (id) DO NOTHING",
            &[
                &crate::DEFAULT_FINITE_PRIVATE_LIMIT_PROFILE,
                &crate::DEFAULT_FINITE_PRIVATE_BURST_WINDOW_SECONDS,
                &crate::DEFAULT_FINITE_PRIVATE_BURST_LIMIT_UNITS,
                &crate::DEFAULT_FINITE_PRIVATE_WEEKLY_LIMIT_UNITS,
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

async fn approve_finite_private_grant_row<C>(
    client: &C,
    user: &CoreUser,
    limit_profile_id: &str,
    now: &str,
) -> CoreResult<FinitePrivateGrant>
where
    C: GenericClient + Sync,
{
    ensure_finite_private_limit_profile_row(client, limit_profile_id, now).await?;
    let grant_id = finite_private_grant_id_for_user(&user.id);
    let row = client
        .query_one(
            "INSERT INTO finite_private_grants (
               id, user_id, limit_profile_id, status, current_window_started_at,
               current_window_used_units, created_at, updated_at
             )
             VALUES ($1, $2, $3, 'active', NULL, 0, $4::text::timestamptz, $4::text::timestamptz)
             ON CONFLICT (user_id) DO UPDATE SET
               limit_profile_id = EXCLUDED.limit_profile_id,
               status = 'active',
               current_window_started_at = NULL,
               current_window_used_units = 0,
               updated_at = EXCLUDED.updated_at
             RETURNING id, user_id, limit_profile_id, status, current_window_started_at::text,
                       current_window_used_units, created_at::text, updated_at::text",
            &[&grant_id, &user.id, &limit_profile_id, &now],
        )
        .await
        .map_err(store_error)?;
    let grant = finite_private_grant_from_row(&row)?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.grant.approve",
            target_type: "grant",
            target_id: &grant.id,
            grant_id: Some(&grant.id),
            api_key_id: None,
            actor: None,
            metadata: json!({
            "userId": grant.user_id.clone(),
            "limitProfileId": grant.limit_profile_id.clone(),
            "verifiedEmail": user.email.clone()
            }),
            now,
        },
    )
    .await?;
    Ok(grant)
}

async fn issue_finite_private_api_key_row<C>(
    client: &C,
    grant: &FinitePrivateGrant,
    raw_key: &str,
    project_id: Option<String>,
    agent_runtime_id: Option<String>,
    now: &str,
) -> CoreResult<FinitePrivateApiKey>
where
    C: GenericClient + Sync,
{
    if grant.status != FinitePrivateGrantStatus::Active {
        return Err(CoreError::FinitePrivateGrantNotActive);
    }
    let key_hash = hash_finite_private_api_key(raw_key)?;
    let key_id = finite_private_api_key_id_for(&grant.id, &key_hash);
    let row = client
        .query_one(
            "INSERT INTO finite_private_api_keys (
               id, grant_id, project_id, agent_runtime_id, key_hash, status, created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, 'active', $6::text::timestamptz, $6::text::timestamptz)
             ON CONFLICT (key_hash) DO UPDATE SET
               status = 'active',
               project_id = EXCLUDED.project_id,
               agent_runtime_id = EXCLUDED.agent_runtime_id,
               updated_at = EXCLUDED.updated_at
             RETURNING id, grant_id, project_id, agent_runtime_id, key_hash, status,
                       created_at::text, updated_at::text",
            &[
                &key_id,
                &grant.id,
                &project_id,
                &agent_runtime_id,
                &key_hash,
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    let key = finite_private_api_key_from_row(&row)?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
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
            now,
        },
    )
    .await?;
    Ok(key)
}

async fn insert_finite_private_admin_audit_event<C>(
    client: &C,
    event: FinitePrivateAdminAuditInsert<'_>,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let actor = event.actor.unwrap_or("finite-saas-core");
    let id = crate::id_from_parts(
        "fp_audit",
        &[event.action, event.target_id, actor, event.now],
    );
    client
        .execute(
            "INSERT INTO finite_private_admin_audit_events (
               id, action, target_type, target_id, grant_id, api_key_id, actor, metadata, created_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb, $9::text::timestamptz)
            ON CONFLICT (id) DO NOTHING",
            &[
                &id,
                &event.action,
                &event.target_type,
                &event.target_id,
                &event.grant_id,
                &event.api_key_id,
                &actor,
                &event.metadata,
                &event.now,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

fn ensure_artifact_launchable(artifact: &RuntimeArtifact) -> CoreResult<()> {
    if artifact.promoted_at.is_none() {
        return Err(CoreError::RuntimeArtifactNotPromoted);
    }
    if artifact.retired_at.is_some() {
        return Err(CoreError::RuntimeArtifactRetired);
    }
    Ok(())
}

fn ensure_runtime_upgrade_target_compatible(
    runtime: &AgentRuntime,
    artifact: &RuntimeArtifact,
) -> CoreResult<()> {
    ensure_artifact_launchable(artifact)?;
    ensure_runtime_upgrade_target_material(runtime, artifact)
}

fn ensure_runtime_upgrade_target_material(
    runtime: &AgentRuntime,
    artifact: &RuntimeArtifact,
) -> CoreResult<()> {
    if artifact.kind != crate::RuntimeArtifactKind::OciImage
        || !runtime_artifact_reference_is_immutable_oci(&artifact.reference)
    {
        return Err(CoreError::RuntimeUpgradeUnsupported);
    }
    if runtime.state_schema_version.as_deref() != Some(artifact.state_schema_version.as_str()) {
        return Err(CoreError::RuntimeUpgradeStateSchemaIncompatible);
    }
    Ok(())
}

fn verify_agent_creation_lease(
    request: &AgentCreationRequest,
    runner_id: &str,
    lease_token: &str,
) -> CoreResult<()> {
    let runner_id =
        trim_to_option(Some(runner_id)).ok_or(CoreError::MissingAgentCreationRunnerId)?;
    let lease_token =
        trim_to_option(Some(lease_token)).ok_or(CoreError::MissingAgentCreationLeaseToken)?;
    if request.status != AgentCreationRequestStatus::Launching {
        return Err(CoreError::AgentCreationRequestNotLaunching);
    }
    if request.runner_id.as_deref() != Some(runner_id.as_str())
        || request.lease_token.as_deref() != Some(lease_token.as_str())
    {
        return Err(CoreError::AgentCreationRequestLeaseConflict);
    }
    Ok(())
}

async fn verify_agent_creation_lease_active<C>(
    client: &C,
    request: &AgentCreationRequest,
    runner_id: &str,
    lease_token: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    verify_agent_creation_lease(request, runner_id, lease_token)?;
    let active: bool = client
        .query_one(
            "SELECT COALESCE(lease_expires_at > CURRENT_TIMESTAMP, false)
             FROM agent_creation_requests WHERE id = $1",
            &[&request.id],
        )
        .await
        .map_err(store_error)?
        .get(0);
    if !active {
        return Err(CoreError::AgentCreationRequestLeaseConflict);
    }
    Ok(())
}

async fn select_provider_operation<C>(
    client: &C,
    request_id: &str,
) -> CoreResult<Option<ProviderOperationEnvelope>>
where
    C: GenericClient + Sync,
{
    let Some(header) = client
        .query_opt(
            "SELECT agent_creation_request_id, schema_name, correlation_id,
                    placement_runner_class, runtime_resource_class
             FROM agent_creation_provider_operations
             WHERE agent_creation_request_id = $1",
            &[&request_id],
        )
        .await
        .map_err(store_error)?
    else {
        return Ok(None);
    };
    let schema_name: String = header.get("schema_name");
    if schema_name != "provider_operation.v1" {
        return Err(CoreError::Store(format!(
            "unsupported provider operation schema {schema_name}"
        )));
    }
    let placement_runner_class: String = header.get("placement_runner_class");
    let runtime_resource_class: String = header.get("runtime_resource_class");
    let placement = RuntimePlacement {
        runner_class: parse_runner_class(&placement_runner_class).ok_or_else(|| {
            CoreError::Store(format!(
                "invalid provider operation runner class {placement_runner_class}"
            ))
        })?,
        runtime_resource_class: parse_runtime_resource_class(&runtime_resource_class).ok_or_else(
            || {
                CoreError::Store(format!(
                    "invalid provider operation resource class {runtime_resource_class}"
                ))
            },
        )?,
    };
    let rows = client
        .query(
            "SELECT sequence, transition, recorded_at::text
             FROM agent_creation_provider_operation_transitions
             WHERE agent_creation_request_id = $1
             ORDER BY sequence",
            &[&request_id],
        )
        .await
        .map_err(store_error)?;
    let mut transitions = Vec::with_capacity(rows.len());
    for (expected, row) in rows.into_iter().enumerate() {
        let sequence: i32 = row.get("sequence");
        if sequence != expected as i32 {
            return Err(CoreError::ProviderOperationTransitionConflict);
        }
        let value: Value = row.get("transition");
        transitions.push(ProviderOperationTransitionRecord {
            sequence: sequence as u32,
            transition: serde_json::from_value(value).map_err(json_error)?,
            recorded_at: row.get("recorded_at"),
        });
    }
    Ok(Some(ProviderOperationEnvelope::V1(ProviderOperationV1 {
        agent_creation_request_id: header.get("agent_creation_request_id"),
        correlation_id: header.get("correlation_id"),
        placement,
        transitions,
    })))
}

async fn persist_provider_operation_delta<C>(
    client: &C,
    previous_len: usize,
    operation: &ProviderOperationEnvelope,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let operation = operation.v1();
    let Some(last) = operation.transitions.last() else {
        return Err(CoreError::ProviderOperationTransitionConflict);
    };
    if operation.transitions.len() == previous_len {
        return Ok(());
    }
    if operation.transitions.len() != previous_len + 1 || last.sequence as usize != previous_len {
        return Err(CoreError::ProviderOperationTransitionConflict);
    }
    client
        .execute(
            "INSERT INTO agent_creation_provider_operations (
                 agent_creation_request_id, schema_name, correlation_id,
                 placement_runner_class, runtime_resource_class, created_at, updated_at
             ) VALUES ($1, 'provider_operation.v1', $2, $3, $4,
                       $5::text::timestamptz, $5::text::timestamptz)
             ON CONFLICT (agent_creation_request_id) DO UPDATE
             SET updated_at = EXCLUDED.updated_at",
            &[
                &operation.agent_creation_request_id,
                &operation.correlation_id,
                &operation.placement.runner_class.as_str(),
                &operation.placement.runtime_resource_class.as_str(),
                &last.recorded_at,
            ],
        )
        .await
        .map_err(store_error)?;
    let transition = serde_json::to_value(&last.transition).map_err(json_error)?;
    client
        .execute(
            "INSERT INTO agent_creation_provider_operation_transitions (
                 agent_creation_request_id, sequence, transition, recorded_at
             ) VALUES ($1, $2, $3, $4::text::timestamptz)",
            &[
                &operation.agent_creation_request_id,
                &(last.sequence as i32),
                &transition,
                &last.recorded_at,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

fn missing_request_project_error(request: &AgentCreationRequest) -> CoreError {
    CoreError::Store(format!(
        "agent creation request {} references missing project {}",
        request.id, request.project_id
    ))
}

async fn ensure_runtime_source_available<C>(
    client: &C,
    source_import_key: &str,
    project_id: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    if client
        .query_opt(
            "SELECT id FROM agent_runtimes
             WHERE source_import_key = $1 AND project_id <> $2
             FOR UPDATE",
            &[&source_import_key, &project_id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::Store(format!(
            "runtime source {source_import_key} is already attached to another project"
        )));
    }
    Ok(())
}

fn runtime_host_facts_from_register_input(
    input: &RegisterAgentCreationRuntimeInput,
    request: &AgentCreationRequest,
    source_host_id: &str,
) -> HostOwnedRuntimeFacts {
    HostOwnedRuntimeFacts {
        display_name: trim_to_option(input.display_name.as_deref())
            .unwrap_or_else(|| request.display_name.clone()),
        hostname: trim_to_option(input.hostname.as_deref()),
        runtime_host: trim_to_option(input.runtime_host.as_deref())
            .unwrap_or_else(|| source_host_id.to_string()),
        runtime_status: input
            .runtime_status
            .unwrap_or(RuntimeSummaryStatus::Unknown),
        active_inference_profile: trim_to_option(input.active_inference_profile.as_deref()),
        hermes_available: input.hermes_available,
        published_app_urls: input.published_app_urls.clone(),
    }
}

fn runtime_host_facts_from_complete_input(
    input: &CompleteAgentCreationRequestInput,
    request: &AgentCreationRequest,
    source_host_id: &str,
) -> HostOwnedRuntimeFacts {
    HostOwnedRuntimeFacts {
        display_name: trim_to_option(input.display_name.as_deref())
            .unwrap_or_else(|| request.display_name.clone()),
        hostname: trim_to_option(input.hostname.as_deref()),
        runtime_host: trim_to_option(input.runtime_host.as_deref())
            .unwrap_or_else(|| source_host_id.to_string()),
        runtime_status: input
            .runtime_status
            .unwrap_or(RuntimeSummaryStatus::Unknown),
        active_inference_profile: trim_to_option(input.active_inference_profile.as_deref()),
        hermes_available: input.hermes_available,
        published_app_urls: input.published_app_urls.clone(),
    }
}

async fn activate_project_runtime_link<C>(
    client: &C,
    project_id: &str,
    runtime_id: &str,
    now: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    client
        .execute(
            "UPDATE project_runtime_links SET active = false WHERE project_id = $1",
            &[&project_id],
        )
        .await
        .map_err(store_error)?;
    let link_id = project_runtime_link_id_for(project_id, runtime_id);
    client
        .execute(
            "INSERT INTO project_runtime_links (id, project_id, agent_runtime_id, active, created_at)
             VALUES ($1, $2, $3, true, $4::text::timestamptz)
             ON CONFLICT (id) DO UPDATE SET active = true",
            &[&link_id, &project_id, &runtime_id, &now],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

async fn update_agent_creation_runtime_registered<C>(
    client: &C,
    request_id: &str,
    runtime_id: &str,
    now: &str,
) -> CoreResult<AgentCreationRequest>
where
    C: GenericClient + Sync,
{
    let row = client
        .query_one(
            "UPDATE agent_creation_requests
             SET agent_runtime_id = $2,
                 failure_message = NULL,
                 updated_at = $3::text::timestamptz
             WHERE id = $1
             RETURNING id, customer_org_id, owner_user_id, project_id, idempotency_key,
                       display_name, runner_class, hosting_tier, placement_runner_class,
                       runtime_resource_class, desired_runtime_artifact_id, runtime_spec,
                       profile_picture_url,
                       status, requested_launch_code, agent_runtime_id,
                       runner_id, lease_token, lease_expires_at::text, failure_message,
                       created_at::text, updated_at::text",
            &[&request_id, &runtime_id, &now],
        )
        .await
        .map_err(store_error)?;
    agent_creation_request_from_row(&row)
}

async fn update_agent_creation_completed<C>(
    client: &C,
    request_id: &str,
    runtime_id: &str,
    now: &str,
) -> CoreResult<AgentCreationRequest>
where
    C: GenericClient + Sync,
{
    let row = client
        .query_one(
            "UPDATE agent_creation_requests
             SET status = 'running',
                 agent_runtime_id = $2,
                 lease_token = NULL,
                 lease_expires_at = NULL,
                 failure_message = NULL,
                 updated_at = $3::text::timestamptz
             WHERE id = $1
             RETURNING id, customer_org_id, owner_user_id, project_id, idempotency_key,
                       display_name, runner_class, hosting_tier, placement_runner_class,
                       runtime_resource_class, desired_runtime_artifact_id, runtime_spec,
                       profile_picture_url,
                       status, requested_launch_code, agent_runtime_id,
                       runner_id, lease_token, lease_expires_at::text, failure_message,
                       created_at::text, updated_at::text",
            &[&request_id, &runtime_id, &now],
        )
        .await
        .map_err(store_error)?;
    agent_creation_request_from_row(&row)
}

async fn delete_runtime_rows<C>(client: &C, runtime_id: &str) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    client
        .execute(
            "DELETE FROM project_runtime_links WHERE agent_runtime_id = $1",
            &[&runtime_id],
        )
        .await
        .map_err(store_error)?;
    client
        .execute(
            "DELETE FROM runtime_status_snapshots WHERE agent_runtime_id = $1",
            &[&runtime_id],
        )
        .await
        .map_err(store_error)?;
    client
        .execute(
            "DELETE FROM runtime_relay_credentials WHERE agent_runtime_id = $1",
            &[&runtime_id],
        )
        .await
        .map_err(store_error)?;
    client
        .execute("DELETE FROM agent_runtimes WHERE id = $1", &[&runtime_id])
        .await
        .map_err(store_error)?;
    Ok(())
}

fn runtime_control_request_from_row(row: &Row) -> CoreResult<RuntimeControlRequest> {
    let kind: String = row.get("kind");
    let status: String = row.get("status");
    Ok(RuntimeControlRequest {
        id: row.get("id"),
        project_id: row.get("project_id"),
        agent_runtime_id: row.get("agent_runtime_id"),
        source_host_id: row.get("source_host_id"),
        source_machine_id: row.get("source_machine_id"),
        requested_by_user_id: row.get("requested_by_user_id"),
        kind: parse_runtime_control_kind(&kind)
            .ok_or_else(|| CoreError::Store(format!("invalid runtime control kind {kind}")))?,
        target_runtime_artifact_id: row.get("target_runtime_artifact_id"),
        status: parse_runtime_control_request_status(&status).ok_or_else(|| {
            CoreError::Store(format!("invalid runtime control request status {status}"))
        })?,
        runner_id: row.get("runner_id"),
        lease_token: row.get("lease_token"),
        lease_expires_at: row.get("lease_expires_at"),
        failure_message: row.get("failure_message"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        completed_at: row.get("completed_at"),
    })
}

const RUNTIME_CONTROL_REQUEST_COLUMNS: &str = "id, project_id, agent_runtime_id, source_host_id,
    source_machine_id, requested_by_user_id, kind, target_runtime_artifact_id,
    status, runner_id, lease_token,
    lease_expires_at::text, failure_message, created_at::text, updated_at::text, completed_at::text";

async fn locked_runtime_control_request<C>(
    client: &C,
    request_id: &str,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT {RUNTIME_CONTROL_REQUEST_COLUMNS} FROM runtime_control_requests
         WHERE id = $1 FOR UPDATE"
    );
    let row = client
        .query_opt(&sql, &[&request_id])
        .await
        .map_err(store_error)?
        .ok_or(CoreError::RuntimeControlRequestNotFound)?;
    runtime_control_request_from_row(&row)
}

/// The active runtime for a project (its one `active` runtime link), resolved
/// with a single row-scoped join instead of scanning all links/runtimes.
async fn postgres_active_runtime_for_project<C>(
    client: &C,
    project_id: &str,
) -> CoreResult<Option<AgentRuntime>>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(
            "SELECT runtime.id, runtime.project_id, runtime.source_host_id,
                    runtime.source_machine_id, runtime.source_import_key,
                    runtime.runtime_artifact_id, runtime.state_schema_version,
                    runtime.placement_runner_class, runtime.runtime_resource_class,
                    runtime.provider_runtime_handle, runtime.provider_runtime_handle_history,
                    runtime.contact_endpoint, runtime.runtime_capabilities,
                    runtime.host_facts, runtime.created_at::text, runtime.updated_at::text
             FROM project_runtime_links AS link
             JOIN agent_runtimes AS runtime ON runtime.id = link.agent_runtime_id
             WHERE link.project_id = $1 AND link.active
             LIMIT 1
             FOR UPDATE OF runtime",
            &[&project_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| agent_runtime_from_row(&row))
        .transpose()
}

/// Row-scoped equivalent of `enqueue_runtime_control_request`: resolve the
/// project's active runtime, verify it supports host runtime-control, dedupe
/// against an in-flight request of the same kind, else insert a new request.
async fn postgres_enqueue_runtime_control_request<C>(
    client: &C,
    project: &Project,
    requested_by_user_id: &str,
    kind: RuntimeControlKind,
    target_runtime_artifact_id: Option<String>,
    now: &str,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    let runtime = postgres_active_runtime_for_project(client, &project.id)
        .await?
        .ok_or(CoreError::ProjectRuntimeNotFound)?;
    if !runtime.supports_runtime_control(kind) {
        return Err(CoreError::RuntimeControlUnsupported);
    }
    let artifact_id = runtime
        .runtime_artifact_id
        .as_deref()
        .ok_or(CoreError::RuntimeRestartUnsupported)?;
    select_runtime_artifact(client, artifact_id)
        .await?
        .ok_or(CoreError::RuntimeArtifactNotFound)?;

    let target_runtime_artifact_id = match kind {
        RuntimeControlKind::Upgrade => {
            let target_id = trim_to_option(target_runtime_artifact_id.as_deref())
                .ok_or(CoreError::MissingRuntimeArtifactId)?;
            let target = select_runtime_artifact(client, &target_id)
                .await?
                .ok_or(CoreError::RuntimeArtifactNotFound)?;
            ensure_runtime_upgrade_target_compatible(&runtime, &target)?;
            Some(target.id)
        }
        _ => None,
    };

    // Exactly one control operation may be active for a Runtime. The Runtime
    // row was locked above, serializing even the zero-existing-row case; the
    // partial unique index is a database-level backstop.
    let existing_sql = format!(
        "SELECT {RUNTIME_CONTROL_REQUEST_COLUMNS} FROM runtime_control_requests
         WHERE agent_runtime_id = $1 AND status IN ('requested', 'running')
         ORDER BY created_at, id
         LIMIT 1
         FOR UPDATE"
    );
    if let Some(row) = client
        .query_opt(&existing_sql, &[&runtime.id])
        .await
        .map_err(store_error)?
    {
        let existing = runtime_control_request_from_row(&row)?;
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
        id: crate::runtime_control_request_id_for(&runtime.id, kind, now),
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
        created_at: now.to_string(),
        updated_at: now.to_string(),
        completed_at: None,
    };
    let row = client
        .query_one(
            "INSERT INTO runtime_control_requests (
               id, project_id, agent_runtime_id, source_host_id, source_machine_id,
               requested_by_user_id, kind, target_runtime_artifact_id, status,
               runner_id, lease_token, lease_expires_at,
               failure_message, created_at, updated_at, completed_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'requested', NULL, NULL, NULL, NULL,
                     $9::text::timestamptz, $9::text::timestamptz, NULL)
             RETURNING id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                       requested_by_user_id, kind, target_runtime_artifact_id, status,
                       runner_id, lease_token,
                       lease_expires_at::text, failure_message, created_at::text,
                       updated_at::text, completed_at::text",
            &[
                &request.id,
                &request.project_id,
                &request.agent_runtime_id,
                &request.source_host_id,
                &request.source_machine_id,
                &request.requested_by_user_id,
                &request.kind.as_str(),
                &request.target_runtime_artifact_id,
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    runtime_control_request_from_row(&row)
}

async fn postgres_request_runtime_control<C>(
    client: &C,
    input: RequestRuntimeRestartInput,
    kind: RuntimeControlKind,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let verified_email = normalize_owner_email(Some(&input.verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let workos_user_id = input.workos_user_id.trim().to_string();
    if workos_user_id.is_empty() {
        return Err(CoreError::MissingWorkosUserId);
    }
    let user =
        ensure_grandfathered_linked_user(client, &verified_email, &workos_user_id, &now).await?;
    let project = select_project(client, &input.project_id)
        .await?
        .ok_or(CoreError::ProjectNotFound)?;
    if project.owner_user_id != user.id {
        return Err(CoreError::ProjectNotFound);
    }
    postgres_enqueue_runtime_control_request(client, &project, &user.id, kind, None, &now).await
}

async fn postgres_admin_request_runtime_control<C>(
    client: &C,
    input: AdminRuntimeControlInput,
    kind: RuntimeControlKind,
    target_runtime_artifact_id: Option<String>,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let admin_workos_user_id = input.admin_workos_user_id.trim().to_string();
    if admin_workos_user_id.is_empty() {
        return Err(CoreError::MissingWorkosUserId);
    }
    let admin_user =
        ensure_grandfathered_linked_user(client, &admin_email, &admin_workos_user_id, &now).await?;
    let project = select_project(client, &input.project_id)
        .await?
        .ok_or(CoreError::ProjectNotFound)?;
    let request = postgres_enqueue_runtime_control_request(
        client,
        &project,
        &admin_user.id,
        kind,
        target_runtime_artifact_id,
        &now,
    )
    .await?;
    let action = match kind {
        RuntimeControlKind::Restart => "runtime.admin_restart",
        RuntimeControlKind::RecoverKnownGoodChatRuntime => "runtime.admin_recover_known_good_chat",
        RuntimeControlKind::Upgrade => "runtime.admin_upgrade",
        RuntimeControlKind::Stop => "runtime.admin_stop",
        RuntimeControlKind::Destroy => "runtime.admin_destroy",
    };
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action,
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
            now: &now,
        },
    )
    .await?;
    Ok(request)
}

async fn postgres_admin_request_runtime_upgrade<C>(
    client: &C,
    input: AdminRuntimeUpgradeInput,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    postgres_admin_request_runtime_control(
        client,
        AdminRuntimeControlInput {
            admin_verified_email: input.admin_verified_email,
            admin_workos_user_id: input.admin_workos_user_id,
            project_id: input.project_id,
            now: input.now,
        },
        RuntimeControlKind::Upgrade,
        Some(input.target_runtime_artifact_id),
    )
    .await
}

/// Partitioned claim: a runner leases only requests routable to it. When the
/// runner declares a `source_host_id`, the claim is scoped to that host via the
/// `runtime_control_requests_pending_idx` (status, source_host_id, created_at,
/// id) — never a global claim across all source hosts. `FOR UPDATE SKIP LOCKED`
/// keeps concurrent runners off each other's rows.
async fn postgres_lease_runtime_control_request<C>(
    client: &C,
    input: LeaseRuntimeControlRequestInput,
    runtime_environment: &BTreeMap<String, String>,
) -> CoreResult<Option<RuntimeControlLease>>
where
    C: GenericClient + Sync,
{
    validate_runtime_spec_environment(runtime_environment)?;
    let now = input.now.unwrap_or(current_time_iso()?);
    let now_time = parse_time(&now)?;
    let runner_id =
        trim_to_option(Some(&input.runner_id)).ok_or(CoreError::MissingAgentCreationRunnerId)?;
    let lease_token = trim_to_option(Some(&input.lease_token))
        .ok_or(CoreError::MissingAgentCreationLeaseToken)?;
    let lease_seconds = input
        .lease_seconds
        .unwrap_or(crate::DEFAULT_AGENT_CREATION_LEASE_SECONDS);
    if !(1..=crate::MAX_AGENT_CREATION_LEASE_SECONDS).contains(&lease_seconds) {
        return Err(CoreError::InvalidAgentCreationLeaseDuration);
    }
    let Some(capacity) = input.runner_capacity.as_ref() else {
        return Ok(None);
    };
    capacity.validate_runtime_capability_policy()?;
    if !capacity.accepts_runtime_control() {
        return Ok(None);
    }
    let source_host_id = input
        .source_host_id
        .as_deref()
        .map(normalize_source_host_id)
        .transpose()?;
    let runner_classes = capacity
        .runner_classes
        .iter()
        .map(|runner_class| runner_class.as_str().to_owned())
        .collect::<Vec<_>>();
    let supported_control_kinds = [
        RuntimeControlKind::Restart,
        RuntimeControlKind::RecoverKnownGoodChatRuntime,
        RuntimeControlKind::Upgrade,
        RuntimeControlKind::Stop,
        RuntimeControlKind::Destroy,
    ]
    .into_iter()
    .filter(|kind| capacity.supports_runtime_control(*kind))
    .map(|kind| kind.as_str().to_owned())
    .collect::<Vec<_>>();
    let lease_expires_at = (now_time + Duration::seconds(lease_seconds)).format(&Rfc3339)?;
    loop {
        let Some(row) = client
            .query_opt(
            "WITH candidate AS (
                SELECT request.id
                FROM runtime_control_requests AS request
                JOIN agent_runtimes AS runtime ON runtime.id = request.agent_runtime_id
                WHERE (
                        request.status = 'requested'
                        OR (
                          request.status = 'running'
                          AND (request.lease_expires_at IS NULL OR request.lease_expires_at <= $4::text::timestamptz)
                        )
                      )
                  AND ($5::text IS NULL OR request.source_host_id = $5)
                  AND runtime.placement_runner_class = ANY($6::text[])
                  AND request.kind = ANY($7::text[])
                  AND runtime.runtime_capabilities->>'schema' = 'runtime_capabilities.v1'
                  AND CASE request.kind
                        WHEN 'restart' THEN
                          runtime.runtime_capabilities->'capabilities'->'restart' = 'true'::jsonb
                        WHEN 'recover_known_good_chat_runtime' THEN
                          runtime.runtime_capabilities->'capabilities'->'recover_known_good_chat' = 'true'::jsonb
                        WHEN 'upgrade' THEN
                          runtime.runtime_capabilities->'capabilities'->'runtime_upgrade' = 'true'::jsonb
                        WHEN 'stop' THEN
                          runtime.runtime_capabilities->'capabilities'->'stop' = 'true'::jsonb
                        WHEN 'destroy' THEN
                          runtime.runtime_capabilities->'capabilities'->'runtime_retirement' = 'true'::jsonb
                        ELSE false
                      END
                ORDER BY request.created_at, request.id
                FOR UPDATE SKIP LOCKED
                LIMIT 1
             )
             UPDATE runtime_control_requests AS request
             SET status = 'running',
                 runner_id = $1,
                 lease_token = $2,
                 lease_expires_at = $3::text::timestamptz,
                 failure_message = NULL,
                 updated_at = $4::text::timestamptz
             FROM candidate
             WHERE request.id = candidate.id
             RETURNING request.id, request.project_id, request.agent_runtime_id,
                       request.source_host_id, request.source_machine_id,
                       request.requested_by_user_id, request.kind,
                       request.target_runtime_artifact_id, request.status,
                       request.runner_id, request.lease_token, request.lease_expires_at::text,
                       request.failure_message, request.created_at::text,
                       request.updated_at::text, request.completed_at::text",
            &[
                &runner_id,
                &lease_token,
                &lease_expires_at,
                &now,
                &source_host_id,
                &runner_classes,
                &supported_control_kinds,
            ],
            )
            .await
            .map_err(store_error)?
        else {
            return Ok(None);
        };
        let request = runtime_control_request_from_row(&row)?;
        let runtime = select_agent_runtime(client, &request.agent_runtime_id)
            .await?
            .ok_or(CoreError::ProjectRuntimeNotFound)?;
        let target_result = async {
            if request.kind != RuntimeControlKind::Upgrade {
                return Ok(None);
            }
            let artifact_id = request
                .target_runtime_artifact_id
                .as_deref()
                .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?;
            let artifact = select_runtime_artifact(client, artifact_id)
                .await?
                .ok_or(CoreError::RuntimeArtifactNotFound)?;
            ensure_runtime_upgrade_target_compatible(&runtime, &artifact)?;
            Ok(Some(artifact))
        }
        .await;
        let target_runtime_artifact = match target_result {
            Ok(target) => target,
            Err(error) if runtime_upgrade_prelease_rejection_is_terminal(&error) => {
                client
                    .execute(
                        "UPDATE runtime_control_requests
                         SET status = 'failed', runner_id = NULL, lease_token = NULL,
                             lease_expires_at = NULL, failure_message = $2,
                             updated_at = $3::text::timestamptz,
                             completed_at = $3::text::timestamptz
                         WHERE id = $1",
                        &[
                            &request.id,
                            &format!("runtime upgrade target rejected before lease: {error}"),
                            &now,
                        ],
                    )
                    .await
                    .map_err(store_error)?;
                continue;
            }
            Err(error) => return Err(error),
        };
        let runtime_spec = if let Some(row) = client
            .query_opt(
                "SELECT id, project_id, runner_class, runtime_spec
                 FROM agent_creation_requests
                 WHERE agent_runtime_id = $1 AND status = 'running'
                 ORDER BY created_at DESC, id DESC
                 LIMIT 1",
                &[&runtime.id],
            )
            .await
            .map_err(store_error)?
        {
            let placement = runtime.placement.ok_or(CoreError::RuntimeSpecMismatch)?;
            let current_artifact_id = runtime
                .runtime_artifact_id
                .as_deref()
                .ok_or(CoreError::RuntimeSpecMismatch)?;
            let current_artifact = select_runtime_artifact(client, current_artifact_id)
                .await?
                .ok_or(CoreError::RuntimeArtifactNotFound)?;
            let current_spec = if let Some(value) = row.get::<_, Option<Value>>("runtime_spec") {
                serde_json::from_value(value).map_err(json_error)?
            } else {
                let creation_id: String = row.get("id");
                let creation_project_id: String = row.get("project_id");
                let creation_runner_class: String = row.get("runner_class");
                let project = select_project(client, &runtime.project_id)
                    .await?
                    .ok_or(CoreError::ProjectNotFound)?;
                if placement.runner_class != crate::RunnerClass::Kata
                    || project.placement != Some(placement)
                    || creation_project_id != runtime.project_id
                    || parse_runner_class(&creation_runner_class) != Some(crate::RunnerClass::Kata)
                    || current_artifact.promoted_at.is_none()
                    || runtime.state_schema_version.as_deref()
                        != Some(current_artifact.state_schema_version.as_str())
                {
                    return Err(CoreError::RuntimeSpecMismatch);
                }
                let synthesized = build_runtime_spec_v1(
                    RuntimeSpecIdentity {
                        operation_id: &creation_id,
                        project_id: &runtime.project_id,
                        agent_runtime_id: &runtime.id,
                        placement,
                    },
                    &current_artifact,
                    // Pre-RuntimeSpec Kata launches used source_machine_id as
                    // their durable-state directory. Preserve that proven
                    // mount identity instead of inventing the Core surrogate
                    // Runtime id during expand-generation synthesis.
                    &runtime.source_machine_id,
                    runtime_environment.clone(),
                    vec![FINITE_PRIVATE_SECRET_REFERENCE.to_string()],
                    RuntimeBootIntent::Normal,
                )?;
                let value = serde_json::to_value(&synthesized).map_err(json_error)?;
                client
                    .execute(
                        "UPDATE agent_creation_requests
                         SET desired_runtime_artifact_id = $2, runtime_spec = $3,
                             updated_at = $4::text::timestamptz
                         WHERE id = $1 AND runtime_spec IS NULL",
                        &[&creation_id, &current_artifact.id, &value, &now],
                    )
                    .await
                    .map_err(store_error)?;
                synthesized
            };
            let desired_artifact = target_runtime_artifact
                .as_ref()
                .unwrap_or(&current_artifact);
            let boot_intent = match request.kind {
                RuntimeControlKind::RecoverKnownGoodChatRuntime => {
                    RuntimeBootIntent::RecoverKnownGood
                }
                RuntimeControlKind::Restart
                | RuntimeControlKind::Upgrade
                | RuntimeControlKind::Stop
                | RuntimeControlKind::Destroy => RuntimeBootIntent::Normal,
            };
            Some(runtime_operation_spec_v1(
                &current_spec,
                RuntimeSpecIdentity {
                    operation_id: &request.id,
                    project_id: &runtime.project_id,
                    agent_runtime_id: &runtime.id,
                    placement,
                },
                &current_artifact,
                desired_artifact,
                boot_intent,
            )?)
        } else {
            None
        };
        return Ok(Some(RuntimeControlLease {
            request,
            runtime,
            runtime_spec,
            target_runtime_artifact,
        }));
    }
}

fn verify_runtime_control_lease(
    request: &RuntimeControlRequest,
    runner_id: &str,
    lease_token: &str,
) -> CoreResult<()> {
    let runner_id =
        trim_to_option(Some(runner_id)).ok_or(CoreError::MissingAgentCreationRunnerId)?;
    let lease_token =
        trim_to_option(Some(lease_token)).ok_or(CoreError::MissingAgentCreationLeaseToken)?;
    if request.status != RuntimeControlRequestStatus::Running {
        return Err(CoreError::RuntimeControlRequestNotRunning);
    }
    if request.runner_id.as_deref() != Some(runner_id.as_str())
        || request.lease_token.as_deref() != Some(lease_token.as_str())
    {
        return Err(CoreError::RuntimeControlRequestLeaseConflict);
    }
    Ok(())
}

/// Apply the completed runtime status to both the runtime's host facts and its
/// status snapshot (if one exists), touching only that runtime's two rows.
struct RuntimeUpgradeCompletion {
    runtime_artifact_id: String,
    state_schema_version: String,
    runtime_host: String,
    published_app_urls: Vec<String>,
    runtime_spec: Option<RuntimeSpecEnvelope>,
    runtime_capabilities: Option<RuntimeCapabilitiesEnvelope>,
}

async fn apply_runtime_control_completion<C>(
    client: &C,
    agent_runtime_id: &str,
    status: RuntimeSummaryStatus,
    destroy: bool,
    upgrade: Option<&RuntimeUpgradeCompletion>,
    now: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    if let Some(mut runtime) = select_agent_runtime(client, agent_runtime_id).await? {
        runtime.host_facts.runtime_status = status;
        if let Some(upgrade) = upgrade {
            runtime.runtime_artifact_id = Some(upgrade.runtime_artifact_id.clone());
            runtime.state_schema_version = Some(upgrade.state_schema_version.clone());
            runtime.host_facts.runtime_host = upgrade.runtime_host.clone();
            runtime.host_facts.published_app_urls = upgrade.published_app_urls.clone();
            runtime.host_facts.hermes_available = Some(true);
            if let Some(capabilities) = upgrade.runtime_capabilities.as_ref() {
                runtime.runtime_capabilities = Some(capabilities.clone());
            }
        }
        if destroy {
            runtime.host_facts.hermes_available = Some(false);
            runtime.host_facts.published_app_urls.clear();
        }
        runtime.updated_at = now.to_string();
        upsert_agent_runtime_row(client, &runtime).await?;
    }
    if let Some(upgrade) = upgrade {
        client
            .execute(
                "UPDATE runtime_status_snapshots
                 SET status = $2, runtime_host = $3, hermes_available = TRUE,
                     updated_at = $4::text::timestamptz
                 WHERE agent_runtime_id = $1",
                &[
                    &agent_runtime_id,
                    &status.as_str(),
                    &upgrade.runtime_host,
                    &now,
                ],
            )
            .await
            .map_err(store_error)?;
    } else if destroy {
        client
            .execute(
                "UPDATE runtime_status_snapshots
                 SET status = $2, hermes_available = FALSE, updated_at = $3::text::timestamptz
                 WHERE agent_runtime_id = $1",
                &[&agent_runtime_id, &status.as_str(), &now],
            )
            .await
            .map_err(store_error)?;
    } else {
        client
            .execute(
                "UPDATE runtime_status_snapshots
                 SET status = $2, updated_at = $3::text::timestamptz
                 WHERE agent_runtime_id = $1",
                &[&agent_runtime_id, &status.as_str(), &now],
            )
            .await
            .map_err(store_error)?;
    }
    Ok(())
}

async fn postgres_complete_runtime_control_request<C>(
    client: &C,
    input: CompleteRuntimeControlRequestInput,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let locked = locked_runtime_control_request(client, &input.request_id).await?;
    verify_runtime_control_lease(&locked, &input.runner_id, &input.lease_token)?;
    let upgrade = if locked.kind == RuntimeControlKind::Upgrade {
        let target_id = locked
            .target_runtime_artifact_id
            .as_deref()
            .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?;
        let reported_id = trim_to_option(input.runtime_artifact_id.as_deref())
            .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?;
        let target = select_runtime_artifact(client, target_id)
            .await?
            .ok_or(CoreError::RuntimeArtifactNotFound)?;
        let runtime = select_agent_runtime(client, &locked.agent_runtime_id)
            .await?
            .ok_or(CoreError::ProjectRuntimeNotFound)?;
        validate_runtime_capabilities_artifact_policy(
            input.runtime_capabilities.as_ref(),
            runtime.placement,
            &target,
        )?;
        // A target may be retired after the runner leased and swapped it.
        // Immutable material remains authoritative for committing the actual
        // compute state; lifecycle policy is enforced at request and lease.
        ensure_runtime_upgrade_target_material(&runtime, &target)?;
        let state_schema_version = trim_to_option(input.state_schema_version.as_deref())
            .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?;
        let runtime_host = trim_to_option(input.runtime_host.as_deref())
            .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?;
        let published_app_urls = input
            .published_app_urls
            .clone()
            .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?;
        if reported_id != target.id || state_schema_version != target.state_schema_version {
            return Err(CoreError::RuntimeUpgradeCompletionMismatch);
        }
        let runtime_spec = if let Some(row) = client
            .query_opt(
                "SELECT runtime_spec
                 FROM agent_creation_requests
                 WHERE agent_runtime_id = $1 AND runtime_spec IS NOT NULL
                 ORDER BY created_at DESC, id DESC
                 LIMIT 1",
                &[&runtime.id],
            )
            .await
            .map_err(store_error)?
        {
            let value: Value = row.get("runtime_spec");
            let current_spec: RuntimeSpecEnvelope =
                serde_json::from_value(value).map_err(json_error)?;
            let placement = runtime.placement.ok_or(CoreError::RuntimeSpecMismatch)?;
            let current_artifact_id = runtime
                .runtime_artifact_id
                .as_deref()
                .ok_or(CoreError::RuntimeSpecMismatch)?;
            let current_artifact = select_runtime_artifact(client, current_artifact_id)
                .await?
                .ok_or(CoreError::RuntimeArtifactNotFound)?;
            Some(runtime_operation_spec_v1(
                &current_spec,
                RuntimeSpecIdentity {
                    operation_id: &locked.id,
                    project_id: &runtime.project_id,
                    agent_runtime_id: &runtime.id,
                    placement,
                },
                &current_artifact,
                &target,
                RuntimeBootIntent::Normal,
            )?)
        } else {
            None
        };
        Some(RuntimeUpgradeCompletion {
            runtime_artifact_id: reported_id,
            state_schema_version,
            runtime_host,
            published_app_urls,
            runtime_spec,
            runtime_capabilities: input.runtime_capabilities.clone(),
        })
    } else {
        if input.runtime_artifact_id.is_some()
            || input.state_schema_version.is_some()
            || input.runtime_host.is_some()
            || input.published_app_urls.is_some()
            || input.runtime_capabilities.is_some()
        {
            return Err(CoreError::RuntimeUpgradeCompletionMismatch);
        }
        None
    };
    let row = client
        .query_one(
            "UPDATE runtime_control_requests
             SET status = 'succeeded',
                 lease_token = NULL,
                 lease_expires_at = NULL,
                 failure_message = NULL,
                 updated_at = $2::text::timestamptz,
                 completed_at = $2::text::timestamptz
             WHERE id = $1
             RETURNING id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                       requested_by_user_id, kind, target_runtime_artifact_id, status,
                       runner_id, lease_token,
                       lease_expires_at::text, failure_message, created_at::text,
                       updated_at::text, completed_at::text",
            &[&input.request_id, &now],
        )
        .await
        .map_err(store_error)?;
    let request = runtime_control_request_from_row(&row)?;
    let completed_status = match request.kind {
        RuntimeControlKind::Restart
        | RuntimeControlKind::RecoverKnownGoodChatRuntime
        | RuntimeControlKind::Upgrade => RuntimeSummaryStatus::Online,
        RuntimeControlKind::Stop | RuntimeControlKind::Destroy => RuntimeSummaryStatus::Offline,
    };
    let destroy = request.kind == RuntimeControlKind::Destroy;
    apply_runtime_control_completion(
        client,
        &request.agent_runtime_id,
        completed_status,
        destroy,
        upgrade.as_ref(),
        &now,
    )
    .await?;
    if let Some(upgrade) = upgrade.as_ref()
        && let Some(runtime_spec) = upgrade.runtime_spec.as_ref()
    {
        let runtime_spec = serde_json::to_value(runtime_spec).map_err(json_error)?;
        client
            .execute(
                "UPDATE agent_creation_requests
                 SET desired_runtime_artifact_id = $2, runtime_spec = $3,
                     updated_at = $4::text::timestamptz
                 WHERE agent_runtime_id = $1",
                &[
                    &request.agent_runtime_id,
                    &upgrade.runtime_artifact_id,
                    &runtime_spec,
                    &now,
                ],
            )
            .await
            .map_err(store_error)?;
    }
    if destroy {
        postgres_offboard_destroyed_runtime(client, &request, &now).await?;
    }
    Ok(request)
}

async fn postgres_fail_runtime_control_request<C>(
    client: &C,
    input: FailRuntimeControlRequestInput,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let failure_message = trim_to_option(Some(&input.failure_message))
        .ok_or(CoreError::MissingRuntimeControlFailureMessage)?;
    let locked = locked_runtime_control_request(client, &input.request_id).await?;
    verify_runtime_control_lease(&locked, &input.runner_id, &input.lease_token)?;
    let row = client
        .query_one(
            "UPDATE runtime_control_requests
             SET status = 'failed',
                 lease_token = NULL,
                 lease_expires_at = NULL,
                 failure_message = $2,
                 updated_at = $3::text::timestamptz,
                 completed_at = $3::text::timestamptz
             WHERE id = $1
             RETURNING id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                       requested_by_user_id, kind, target_runtime_artifact_id, status,
                       runner_id, lease_token,
                       lease_expires_at::text, failure_message, created_at::text,
                       updated_at::text, completed_at::text",
            &[&input.request_id, &failure_message, &now],
        )
        .await
        .map_err(store_error)?;
    let request = runtime_control_request_from_row(&row)?;
    // A failed control action leaves the box in an unknown/stale state.
    if let Some(mut runtime) = select_agent_runtime(client, &request.agent_runtime_id).await? {
        runtime.host_facts.runtime_status = RuntimeSummaryStatus::Stale;
        runtime.updated_at = now.clone();
        upsert_agent_runtime_row(client, &runtime).await?;
    }
    client
        .execute(
            "UPDATE runtime_status_snapshots
             SET status = 'stale', updated_at = $2::text::timestamptz
             WHERE agent_runtime_id = $1",
            &[&request.agent_runtime_id, &now],
        )
        .await
        .map_err(store_error)?;
    Ok(request)
}

/// Row-scoped `offboard_destroyed_runtime`: hide the normal project from its
/// room members, deactivate the runtime's links, drop its relay credential,
/// revoke every active Finite Private key bound to the runtime or its project,
/// and audit the revocation. Project, membership, runtime, and link rows remain
/// retained for recovery and audit.
async fn postgres_offboard_destroyed_runtime<C>(
    client: &C,
    request: &RuntimeControlRequest,
    now: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    client
        .execute(
            "UPDATE project_room_memberships AS membership
             SET archived_at = $2::text::timestamptz
             WHERE membership.project_id = $1
               AND membership.archived_at IS NULL
               AND EXISTS (
                 SELECT 1
                 FROM projects AS project
                 WHERE project.id = $1
                   AND project.import_candidate_id IS NULL
               )",
            &[&request.project_id, &now],
        )
        .await
        .map_err(store_error)?;
    client
        .execute(
            "UPDATE project_runtime_links SET active = FALSE WHERE agent_runtime_id = $1",
            &[&request.agent_runtime_id],
        )
        .await
        .map_err(store_error)?;
    client
        .execute(
            "DELETE FROM runtime_relay_credentials WHERE agent_runtime_id = $1",
            &[&request.agent_runtime_id],
        )
        .await
        .map_err(store_error)?;
    let revoked_rows = client
        .query(
            "UPDATE finite_private_api_keys
             SET status = 'revoked', updated_at = $3::text::timestamptz
             WHERE status = 'active'
               AND (agent_runtime_id = $1 OR project_id = $2)
             RETURNING id",
            &[&request.agent_runtime_id, &request.project_id, &now],
        )
        .await
        .map_err(store_error)?;
    let revoked_api_key_ids: Vec<String> = revoked_rows.iter().map(|row| row.get("id")).collect();
    if !revoked_api_key_ids.is_empty() {
        insert_finite_private_admin_audit_event(
            client,
            FinitePrivateAdminAuditInsert {
                action: "finite_private.runtime.destroy_revoke_keys",
                target_type: "agent_runtime",
                target_id: &request.agent_runtime_id,
                grant_id: None,
                api_key_id: None,
                actor: None,
                metadata: json!({
                    "projectId": request.project_id.clone(),
                    "revokedApiKeyIds": revoked_api_key_ids,
                }),
                now,
            },
        )
        .await?;
    }
    Ok(())
}

/// Find-or-create a linked user by natural key (email), then ensure their
/// personal org exists — the Postgres equivalent of
/// `ensure_linked_user_with_billing_class`. Enforces the WorkOS-id-uniqueness
/// guard. The billing class only takes effect when the org is first created.
async fn ensure_linked_user_row<C>(
    client: &C,
    email: &str,
    workos_user_id: &str,
    billing_class: BillingClass,
    now: &str,
) -> CoreResult<CoreUser>
where
    C: GenericClient + Sync,
{
    if client
        .query_opt(
            "SELECT id FROM users WHERE workos_user_id = $1 AND normalized_email <> $2",
            &[&workos_user_id, &email],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::WorkosUserConflict);
    }
    let user = upsert_linked_user(client, email, workos_user_id, now).await?;
    ensure_personal_org_row(client, &user, billing_class, now).await?;
    Ok(user)
}

/// `ensure_linked_user` (Grandfathered default) for the import/runtime-control
/// paths that do not carry billing intent.
async fn ensure_grandfathered_linked_user<C>(
    client: &C,
    email: &str,
    workos_user_id: &str,
    now: &str,
) -> CoreResult<CoreUser>
where
    C: GenericClient + Sync,
{
    ensure_linked_user_row(
        client,
        email,
        workos_user_id,
        BillingClass::Grandfathered,
        now,
    )
    .await
}

async fn select_source_host_relay<C>(
    client: &C,
    source_host_id: &str,
) -> CoreResult<Option<SourceHostRelayEndpoint>>
where
    C: GenericClient + Sync,
{
    Ok(client
        .query_opt(
            "SELECT source_host_id, url, admin_token, created_at::text, updated_at::text
             FROM source_host_relays WHERE source_host_id = $1",
            &[&source_host_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| SourceHostRelayEndpoint {
            source_host_id: row.get("source_host_id"),
            url: row.get("url"),
            admin_token: row.get("admin_token"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        }))
}

async fn postgres_upsert_source_host_relay_endpoint<C>(
    client: &C,
    input: UpsertSourceHostRelayEndpointInput,
) -> CoreResult<SourceHostRelayEndpoint>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let source_host_id = normalize_source_host_id(&input.source_host_id)?;
    let url = crate::normalize_source_host_relay_url(&input.url)?;
    let admin_token = input.admin_token.trim();
    if admin_token.is_empty() {
        return Err(CoreError::MissingSourceHostRelayAdminToken);
    }
    let row = client
        .query_one(
            "INSERT INTO source_host_relays (source_host_id, url, admin_token, created_at, updated_at)
             VALUES ($1, $2, $3, $4::text::timestamptz, $4::text::timestamptz)
             ON CONFLICT (source_host_id) DO UPDATE SET
               url = EXCLUDED.url,
               admin_token = EXCLUDED.admin_token,
               updated_at = EXCLUDED.updated_at
             RETURNING source_host_id, url, admin_token, created_at::text, updated_at::text",
            &[&source_host_id, &url, &admin_token, &now],
        )
        .await
        .map_err(store_error)?;
    Ok(SourceHostRelayEndpoint {
        source_host_id: row.get("source_host_id"),
        url: row.get("url"),
        admin_token: row.get("admin_token"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

async fn postgres_upsert_runtime_artifact<C>(
    client: &C,
    input: UpsertRuntimeArtifactInput,
) -> CoreResult<RuntimeArtifact>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let id = trim_to_option(Some(&input.id)).ok_or(CoreError::MissingRuntimeArtifactId)?;
    let reference =
        trim_to_option(Some(&input.reference)).ok_or(CoreError::MissingRuntimeArtifactReference)?;
    let version_label = trim_to_option(Some(&input.version_label))
        .ok_or(CoreError::MissingRuntimeArtifactVersionLabel)?;
    let state_schema_version = trim_to_option(Some(&input.state_schema_version))
        .ok_or(CoreError::MissingRuntimeArtifactStateSchemaVersion)?;
    // Lock the existing row (if any) so created_at/promoted_at/retired_at are
    // preserved deterministically under concurrent upserts.
    let existing = client
        .query_opt(
            "SELECT id, kind, reference, version_label, source_git_sha, finitec_version,
                    hermes_source_ref, finite_platform_plugin_ref, state_schema_version,
                    base_image, recover_known_good_chat,
                    created_at::text, promoted_at::text, retired_at::text
             FROM runtime_artifacts WHERE id = $1 FOR UPDATE",
            &[&id],
        )
        .await
        .map_err(store_error)?
        .map(|row| runtime_artifact_from_row(&row))
        .transpose()?;
    let existing_created_at = existing
        .as_ref()
        .map(|artifact| artifact.created_at.clone());
    let existing_promoted_at = existing
        .as_ref()
        .and_then(|artifact| artifact.promoted_at.clone());
    let existing_retired_at = existing
        .as_ref()
        .and_then(|artifact| artifact.retired_at.clone());
    let created_at = existing_created_at.unwrap_or_else(|| now.clone());
    let promoted_at = if input.promoted {
        existing_promoted_at.or_else(|| Some(now.clone()))
    } else {
        existing_promoted_at
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
        recover_known_good_chat: input.recover_known_good_chat,
        created_at,
        promoted_at,
        retired_at: existing_retired_at,
    };
    if let Some(existing) = existing.as_ref() {
        let referenced: bool = client
            .query_one(
                "SELECT EXISTS (
                   SELECT 1 FROM agent_runtimes WHERE runtime_artifact_id = $1
                 ) AS referenced",
                &[&id],
            )
            .await
            .map_err(store_error)?
            .get("referenced");
        if (existing.promoted_at.is_some() || referenced)
            && !runtime_artifact_material_matches(existing, &artifact)
        {
            return Err(CoreError::RuntimeArtifactImmutable);
        }
    }
    let row = client
        .query_one(
            "INSERT INTO runtime_artifacts (
               id, kind, reference, version_label, source_git_sha, finitec_version,
               hermes_source_ref, finite_platform_plugin_ref, state_schema_version,
               base_image, recover_known_good_chat, created_at, promoted_at, retired_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                     $11, $12::text::timestamptz, $13::text::timestamptz,
                     $14::text::timestamptz)
             ON CONFLICT (id) DO UPDATE SET
               kind = EXCLUDED.kind,
               reference = EXCLUDED.reference,
               version_label = EXCLUDED.version_label,
               source_git_sha = EXCLUDED.source_git_sha,
               finitec_version = EXCLUDED.finitec_version,
               hermes_source_ref = EXCLUDED.hermes_source_ref,
               finite_platform_plugin_ref = EXCLUDED.finite_platform_plugin_ref,
               state_schema_version = EXCLUDED.state_schema_version,
               base_image = EXCLUDED.base_image,
               recover_known_good_chat = EXCLUDED.recover_known_good_chat,
               promoted_at = EXCLUDED.promoted_at,
               retired_at = EXCLUDED.retired_at
             RETURNING id, kind, reference, version_label, source_git_sha, finitec_version,
                       hermes_source_ref, finite_platform_plugin_ref, state_schema_version,
                       base_image, recover_known_good_chat,
                       created_at::text, promoted_at::text, retired_at::text",
            &[
                &artifact.id,
                &artifact.kind.as_str(),
                &artifact.reference,
                &artifact.version_label,
                &artifact.source_git_sha,
                &artifact.finitec_version,
                &artifact.hermes_source_ref,
                &artifact.finite_platform_plugin_ref,
                &artifact.state_schema_version,
                &artifact.base_image,
                &artifact.recover_known_good_chat,
                &artifact.created_at,
                &artifact.promoted_at,
                &artifact.retired_at,
            ],
        )
        .await
        .map_err(store_error)?;
    runtime_artifact_from_row(&row)
}

async fn postgres_relay_events_for_runtime<C>(
    client: &C,
    relay_token: &str,
) -> CoreResult<RelayEventsOutput>
where
    C: GenericClient + Sync,
{
    let token_hash = runtime_relay_token_hash(relay_token)?;
    let row = client
        .query_opt(
            "SELECT runtime.source_machine_id
             FROM runtime_relay_credentials AS credential
             JOIN agent_runtimes AS runtime ON runtime.id = credential.agent_runtime_id
             WHERE credential.token_hash = $1",
            &[&token_hash],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::InvalidRuntimeRelayToken)?;
    Ok(RelayEventsOutput {
        machine_id: row.get("source_machine_id"),
        events: Vec::new(),
    })
}

async fn postgres_admin_runtime_overviews<C>(client: &C) -> CoreResult<Vec<AdminRuntimeOverview>>
where
    C: GenericClient + Sync,
{
    let rows = client
        .query(
            "SELECT runtime.id AS agent_runtime_id, runtime.project_id, runtime.source_host_id,
                    runtime.source_machine_id, runtime.runtime_artifact_id, runtime.host_facts,
                    runtime.updated_at::text AS runtime_updated_at,
                    project.display_name AS project_display_name,
                    owner.normalized_email AS owner_email,
                    snapshot.status AS snapshot_status,
                    snapshot.last_heartbeat_at::text AS last_heartbeat_at,
                    snapshot.updated_at::text AS status_updated_at,
                    snapshot.hermes_available AS snapshot_hermes_available,
                    artifact.version_label AS runtime_artifact_version_label,
                    runtime.runtime_capabilities,
                    EXISTS (
                      SELECT 1 FROM project_runtime_links link
                      WHERE link.agent_runtime_id = runtime.id AND link.active
                    ) AS runtime_link_active,
                    (
                      SELECT COUNT(*) FROM finite_private_api_keys key
                      WHERE key.status = 'active'
                        AND (key.agent_runtime_id = runtime.id OR key.project_id = runtime.project_id)
                    )::BIGINT AS active_finite_private_key_count
             FROM agent_runtimes AS runtime
             LEFT JOIN projects AS project ON project.id = runtime.project_id
             LEFT JOIN users AS owner ON owner.id = project.owner_user_id
             LEFT JOIN runtime_status_snapshots AS snapshot ON snapshot.agent_runtime_id = runtime.id
             LEFT JOIN runtime_artifacts AS artifact ON artifact.id = runtime.runtime_artifact_id
             ORDER BY runtime.source_host_id, runtime.source_machine_id, runtime.id",
            &[],
        )
        .await
        .map_err(store_error)?;
    rows.iter()
        .map(|row| {
            let host_facts: HostOwnedRuntimeFacts = json_column(row, "host_facts")?;
            let snapshot_status: Option<String> = row.get("snapshot_status");
            let snapshot_status = snapshot_status
                .as_deref()
                .map(|value| {
                    parse_runtime_summary_status(value)
                        .ok_or_else(|| CoreError::Store(format!("invalid runtime status {value}")))
                })
                .transpose()?;
            let runtime_capabilities: Option<RuntimeCapabilitiesEnvelope> =
                optional_json_column(row, "runtime_capabilities")?
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(json_error)?;
            let project_display_name: Option<String> = row.get("project_display_name");
            let snapshot_hermes: Option<bool> = row.get("snapshot_hermes_available");
            Ok(AdminRuntimeOverview {
                project_id: row.get("project_id"),
                project_display_name: project_display_name
                    .unwrap_or_else(|| host_facts.display_name.clone()),
                owner_email: row.get("owner_email"),
                agent_runtime_id: row.get("agent_runtime_id"),
                source_host_id: row.get("source_host_id"),
                source_machine_id: row.get("source_machine_id"),
                runtime_artifact_id: row.get("runtime_artifact_id"),
                runtime_artifact_version_label: row.get("runtime_artifact_version_label"),
                runtime_status: snapshot_status.unwrap_or(host_facts.runtime_status),
                last_heartbeat_at: row.get("last_heartbeat_at"),
                status_updated_at: row.get("status_updated_at"),
                runtime_updated_at: row.get("runtime_updated_at"),
                hermes_available: snapshot_hermes.or(host_facts.hermes_available),
                published_app_urls: host_facts.published_app_urls.clone(),
                active_finite_private_key_count: row.get("active_finite_private_key_count"),
                runtime_link_active: row.get("runtime_link_active"),
                runtime_capabilities: runtime_capabilities
                    .as_ref()
                    .map(|capabilities| *capabilities.v1()),
            })
        })
        .collect()
}

fn import_candidate_from_row(row: &Row) -> CoreResult<ProjectImportCandidate> {
    let status: String = row.get("status");
    Ok(ProjectImportCandidate {
        id: row.get("id"),
        source_host_id: row.get("source_host_id"),
        source_machine_id: row.get("source_machine_id"),
        source_import_key: row.get("source_import_key"),
        owner_email: row.get("owner_email"),
        latest_host_owner_email: row.get("latest_host_owner_email"),
        pending_user_id: row.get("pending_user_id"),
        customer_org_id: row.get("customer_org_id"),
        status: parse_import_candidate_status(&status)
            .ok_or_else(|| CoreError::Store(format!("invalid import candidate status {status}")))?,
        project_id: row.get("project_id"),
        agent_runtime_id: row.get("agent_runtime_id"),
        claimed_by_user_id: row.get("claimed_by_user_id"),
        host_facts: json_column(row, "host_facts")?,
        known_external_channel_participants: json_column(
            row,
            "known_external_channel_participants",
        )?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

const IMPORT_CANDIDATE_COLUMNS: &str = "id, source_host_id, source_machine_id, source_import_key,
    owner_email, latest_host_owner_email, pending_user_id, customer_org_id, status, project_id,
    agent_runtime_id, claimed_by_user_id, host_facts, known_external_channel_participants,
    created_at::text, updated_at::text";

async fn select_import_candidate_by_source_import_key<C>(
    client: &C,
    source_import_key: &str,
) -> CoreResult<Option<ProjectImportCandidate>>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT {IMPORT_CANDIDATE_COLUMNS} FROM project_import_candidates
         WHERE source_import_key = $1 FOR UPDATE"
    );
    client
        .query_opt(&sql, &[&source_import_key])
        .await
        .map_err(store_error)?
        .map(|row| import_candidate_from_row(&row))
        .transpose()
}

async fn select_import_candidate<C>(
    client: &C,
    candidate_id: &str,
) -> CoreResult<Option<ProjectImportCandidate>>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT {IMPORT_CANDIDATE_COLUMNS} FROM project_import_candidates
         WHERE id = $1 FOR UPDATE"
    );
    client
        .query_opt(&sql, &[&candidate_id])
        .await
        .map_err(store_error)?
        .map(|row| import_candidate_from_row(&row))
        .transpose()
}

/// Find-or-create a PENDING user by natural key (email). Mirrors
/// `ensure_pending_user`: an existing row (pending or linked) keeps its
/// surrogate id; a brand-new email gets a fresh one. Never derives id from PII.
async fn ensure_pending_user_row<C>(client: &C, email: &str, now: &str) -> CoreResult<CoreUser>
where
    C: GenericClient + Sync,
{
    if let Some(existing) = select_user_by_email(client, email).await? {
        return Ok(existing);
    }
    let user_id = new_user_id()?;
    let row = client
        .query_one(
            "INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at)
             VALUES ($1, $2, 'pending', NULL, $3::text::timestamptz, $3::text::timestamptz)
             ON CONFLICT (normalized_email) DO UPDATE SET updated_at = users.updated_at
             RETURNING id, normalized_email, link_status, workos_user_id,
                       created_at::text, updated_at::text",
            &[&user_id, &email, &now],
        )
        .await
        .map_err(store_error)?;
    core_user_from_row(&row)
}

async fn upsert_import_candidate_row<C>(
    client: &C,
    candidate: &ProjectImportCandidate,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let host_facts = serde_json::to_value(&candidate.host_facts).map_err(json_error)?;
    let participants =
        serde_json::to_value(&candidate.known_external_channel_participants).map_err(json_error)?;
    client
        .execute(
            "INSERT INTO project_import_candidates (
               id, source_host_id, source_machine_id, source_import_key, owner_email,
               latest_host_owner_email, pending_user_id, customer_org_id, status,
               project_id, agent_runtime_id, claimed_by_user_id, host_facts,
               known_external_channel_participants, created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13::jsonb, $14::jsonb,
                     $15::text::timestamptz, $16::text::timestamptz)
             ON CONFLICT (id) DO UPDATE SET
               latest_host_owner_email = EXCLUDED.latest_host_owner_email,
               status = EXCLUDED.status,
               project_id = EXCLUDED.project_id,
               agent_runtime_id = EXCLUDED.agent_runtime_id,
               claimed_by_user_id = EXCLUDED.claimed_by_user_id,
               host_facts = EXCLUDED.host_facts,
               known_external_channel_participants = EXCLUDED.known_external_channel_participants,
               updated_at = EXCLUDED.updated_at",
            &[
                &candidate.id,
                &candidate.source_host_id,
                &candidate.source_machine_id,
                &candidate.source_import_key,
                &candidate.owner_email,
                &candidate.latest_host_owner_email,
                &candidate.pending_user_id,
                &candidate.customer_org_id,
                &candidate.status.as_str(),
                &candidate.project_id,
                &candidate.agent_runtime_id,
                &candidate.claimed_by_user_id,
                &host_facts,
                &participants,
                &candidate.created_at,
                &candidate.updated_at,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}

async fn postgres_reconcile_existing_host_imports<C>(
    client: &C,
    records: &[ExistingHostProjectImport],
    options: ReconcileExistingHostImportsOptions,
) -> CoreResult<ReconcileExistingHostImportsReport>
where
    C: GenericClient + Sync,
{
    let now = options.now.unwrap_or(current_time_iso()?);
    let allowlist = options
        .allowlisted_owner_emails
        .into_iter()
        .filter_map(|email| normalize_owner_email(Some(&email)))
        .collect::<std::collections::BTreeSet<_>>();
    let mut report = ReconcileExistingHostImportsReport {
        created_candidates: Vec::new(),
        updated_candidates: Vec::new(),
        skipped_records: Vec::new(),
    };

    for record in records {
        let source_key = source_import_key(&record.source_host_id, &record.source_machine_id);
        let owner_email = match normalize_owner_email(record.owner_email.as_deref()) {
            Some(email) => email,
            None => {
                report.skipped_records.push(crate::SkippedImportRecord {
                    source_import_key: source_key,
                    reason: crate::SkippedImportReason::MissingOwnerEmail,
                });
                continue;
            }
        };

        // Resolve the candidate by its natural key (source_import_key UNIQUE),
        // FOR UPDATE, instead of a deterministic-id lookup.
        if let Some(existing) =
            select_import_candidate_by_source_import_key(client, &source_key).await?
        {
            let host_facts =
                serde_json::to_value(crate::host_facts_from_record(record)).map_err(json_error)?;
            let participants = serde_json::to_value(&record.known_external_channel_participants)
                .map_err(json_error)?;
            client
                .execute(
                    "UPDATE project_import_candidates
                     SET latest_host_owner_email = $2,
                         host_facts = $3::jsonb,
                         known_external_channel_participants = $4::jsonb,
                         updated_at = $5::text::timestamptz
                     WHERE id = $1",
                    &[&existing.id, &owner_email, &host_facts, &participants, &now],
                )
                .await
                .map_err(store_error)?;
            // Keep a claimed candidate's runtime host facts in sync.
            if let Some(runtime_id) = existing.agent_runtime_id.as_deref() {
                client
                    .execute(
                        "UPDATE agent_runtimes
                         SET host_facts = $2::jsonb, updated_at = $3::text::timestamptz
                         WHERE id = $1",
                        &[&runtime_id, &host_facts, &now],
                    )
                    .await
                    .map_err(store_error)?;
            }
            report.updated_candidates.push(existing.id);
            continue;
        }

        if !allowlist.contains(&owner_email) {
            report.skipped_records.push(crate::SkippedImportRecord {
                source_import_key: source_key,
                reason: crate::SkippedImportReason::OwnerNotAllowlisted,
            });
            continue;
        }

        let user = ensure_pending_user_row(client, &owner_email, &now).await?;
        let org = ensure_personal_org_row(client, &user, BillingClass::Grandfathered, &now).await?;
        let candidate = ProjectImportCandidate {
            id: crate::new_import_candidate_id()?,
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
            status: crate::ImportCandidateStatus::Pending,
            project_id: None,
            agent_runtime_id: None,
            claimed_by_user_id: None,
            host_facts: crate::host_facts_from_record(record),
            known_external_channel_participants: record.known_external_channel_participants.clone(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        upsert_import_candidate_row(client, &candidate).await?;
        report.created_candidates.push(candidate.id);
    }

    Ok(report)
}

async fn postgres_claim_project_imports<C>(
    client: &C,
    input: ClaimProjectImportsInput,
) -> CoreResult<ClaimProjectImportsResult>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let verified_email = normalize_owner_email(Some(&input.verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let workos_user_id = input.workos_user_id.trim().to_string();
    if workos_user_id.is_empty() {
        return Err(CoreError::MissingWorkosUserId);
    }
    let user =
        ensure_grandfathered_linked_user(client, &verified_email, &workos_user_id, &now).await?;
    let mut result = ClaimProjectImportsResult::default();
    let selected_candidate_ids = input
        .selected_candidate_ids
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();

    for candidate_id in selected_candidate_ids {
        let Some(candidate) = select_import_candidate(client, &candidate_id).await? else {
            result.missing_candidate_ids.push(candidate_id);
            continue;
        };
        if candidate.owner_email != verified_email || candidate.pending_user_id != user.id {
            result.denied_candidate_ids.push(candidate.id);
            continue;
        }
        if candidate.status == crate::ImportCandidateStatus::Claimed {
            if let Some(project_id) = candidate.project_id {
                ensure_hosted_web_membership_row(client, &user, &project_id, &now).await?;
                result.already_claimed_project_ids.push(project_id);
            }
            continue;
        }

        // Fresh surrogate ids for the claimed project and its runtime; the
        // candidate is resolved by its natural key, never rederived.
        let project_id = new_self_service_project_id()?;
        let runtime_id = new_agent_runtime_id()?;
        let project = Project {
            id: project_id.clone(),
            customer_org_id: candidate.customer_org_id.clone(),
            owner_user_id: user.id.clone(),
            display_name: candidate.host_facts.display_name.clone(),
            import_candidate_id: Some(candidate.id.clone()),
            hosting_tier: Some(HostingTier::Standard),
            placement: Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        upsert_project_row(client, &project).await?;
        let runtime = AgentRuntime {
            id: runtime_id.clone(),
            project_id: project_id.clone(),
            source_host_id: candidate.source_host_id.clone(),
            source_machine_id: candidate.source_machine_id.clone(),
            source_import_key: candidate.source_import_key.clone(),
            runtime_artifact_id: None,
            state_schema_version: None,
            placement: project.placement,
            provider_runtime_handle: None,
            provider_runtime_handle_history: Vec::new(),
            contact_endpoint: None,
            runtime_capabilities: None,
            host_facts: candidate.host_facts.clone(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        upsert_agent_runtime_row(client, &runtime).await?;
        activate_project_runtime_link(client, &project_id, &runtime_id, &now).await?;
        client
            .execute(
                "UPDATE project_import_candidates
                 SET status = 'claimed',
                     project_id = $2,
                     agent_runtime_id = $3,
                     claimed_by_user_id = $4,
                     updated_at = $5::text::timestamptz
                 WHERE id = $1",
                &[&candidate.id, &project_id, &runtime_id, &user.id, &now],
            )
            .await
            .map_err(store_error)?;
        ensure_hosted_web_membership_row(client, &user, &project_id, &now).await?;
        result.claimed_project_ids.push(project_id);
    }

    Ok(result)
}

async fn postgres_claimable_candidates_for_email<C>(
    client: &C,
    email: Option<&str>,
) -> CoreResult<Vec<ProjectImportCandidate>>
where
    C: GenericClient + Sync,
{
    let Some(normalized) = normalize_owner_email(email) else {
        return Ok(Vec::new());
    };
    let sql = format!(
        "SELECT {IMPORT_CANDIDATE_COLUMNS} FROM project_import_candidates
         WHERE status = 'pending' AND owner_email = $1"
    );
    client
        .query(&sql, &[&normalized])
        .await
        .map_err(store_error)?
        .iter()
        .map(import_candidate_from_row)
        .collect()
}

/// RFC3339 rendering for a TIMESTAMPTZ column so stored strings round-trip
/// through `parse_time` (the Finite Private timestamps are parsed, not just
/// echoed).
fn rfc3339_col(expr: &str) -> String {
    format!("to_char({expr} AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')")
}

fn finite_private_limit_profile_from_row(row: &Row) -> FinitePrivateLimitProfile {
    FinitePrivateLimitProfile {
        id: row.get("id"),
        burst_window_seconds: row.get("burst_window_seconds"),
        burst_limit_units: row.get("burst_limit_units"),
        weekly_limit_units: row.get("weekly_limit_units"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn finite_private_reservation_from_row(row: &Row) -> CoreResult<FinitePrivateReservation> {
    let status: String = row.get("status");
    let settlement_kind: Option<String> = row.get("settlement_kind");
    let settlement_kind = match settlement_kind.as_deref() {
        Some(value) => Some(
            crate::parse_finite_private_settlement_kind(value).ok_or_else(|| {
                CoreError::Store(format!("invalid finite private settlement kind {value}"))
            })?,
        ),
        None => None,
    };
    Ok(FinitePrivateReservation {
        id: row.get("id"),
        request_id: row.get("request_id"),
        api_key_id: row.get("api_key_id"),
        grant_id: row.get("grant_id"),
        endpoint: row.get("endpoint"),
        model: row.get("model"),
        estimated_usage_units: row.get("estimated_usage_units"),
        reserved_usage_units: row.get("reserved_usage_units"),
        settled_usage_units: row.get("settled_usage_units"),
        settlement_kind,
        status: parse_finite_private_reservation_status(&status).ok_or_else(|| {
            CoreError::Store(format!(
                "invalid finite private reservation status {status}"
            ))
        })?,
        usage_formula_version: row.get("usage_formula_version"),
        upstream_status: row.get("upstream_status"),
        upstream_error_class: row.get("upstream_error_class"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

async fn select_finite_private_grant<C>(
    client: &C,
    grant_id: &str,
    for_update: bool,
) -> CoreResult<Option<FinitePrivateGrant>>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT id, user_id, limit_profile_id, status,
                CASE WHEN current_window_started_at IS NULL THEN NULL
                     ELSE {started} END AS current_window_started_at,
                current_window_used_units,
                {created} AS created_at, {updated} AS updated_at
         FROM finite_private_grants WHERE id = $1{lock}",
        started = rfc3339_col("current_window_started_at"),
        created = rfc3339_col("created_at"),
        updated = rfc3339_col("updated_at"),
        lock = if for_update { " FOR UPDATE" } else { "" },
    );
    client
        .query_opt(&sql, &[&grant_id])
        .await
        .map_err(store_error)?
        .map(|row| finite_private_grant_from_row(&row))
        .transpose()
}

async fn select_finite_private_limit_profile<C>(
    client: &C,
    id: &str,
) -> CoreResult<Option<FinitePrivateLimitProfile>>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT id, burst_window_seconds, burst_limit_units, weekly_limit_units,
                {created} AS created_at, {updated} AS updated_at
         FROM finite_private_limit_profiles WHERE id = $1",
        created = rfc3339_col("created_at"),
        updated = rfc3339_col("updated_at"),
    );
    Ok(client
        .query_opt(&sql, &[&id])
        .await
        .map_err(store_error)?
        .as_ref()
        .map(finite_private_limit_profile_from_row))
}

async fn select_finite_private_reservation<C>(
    client: &C,
    reservation_id: &str,
    for_update: bool,
) -> CoreResult<Option<FinitePrivateReservation>>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT id, request_id, api_key_id, grant_id, endpoint, model,
                estimated_usage_units, reserved_usage_units, settled_usage_units,
                settlement_kind, status, usage_formula_version, upstream_status,
                upstream_error_class, {created} AS created_at, {updated} AS updated_at
         FROM finite_private_reservations WHERE id = $1{lock}",
        created = rfc3339_col("created_at"),
        updated = rfc3339_col("updated_at"),
        lock = if for_update { " FOR UPDATE" } else { "" },
    );
    client
        .query_opt(&sql, &[&reservation_id])
        .await
        .map_err(store_error)?
        .map(|row| finite_private_reservation_from_row(&row))
        .transpose()
}

/// Resolve the (active api key, active grant) pair for a presented raw key by
/// its hash. An empty/invalid/revoked key or grant yields `None` (a denial),
/// never an error — mirroring `finite_private_key_and_grant`.
async fn postgres_finite_private_key_and_grant<C>(
    client: &C,
    presented_api_key: &str,
) -> CoreResult<Option<(FinitePrivateApiKey, FinitePrivateGrant)>>
where
    C: GenericClient + Sync,
{
    let key_hash = match hash_finite_private_api_key(presented_api_key) {
        Ok(hash) => hash,
        Err(CoreError::MissingFinitePrivateApiKey) => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(row) = client
        .query_opt(
            "SELECT id, grant_id, project_id, agent_runtime_id, key_hash, status,
                    created_at::text, updated_at::text
             FROM finite_private_api_keys WHERE key_hash = $1",
            &[&key_hash],
        )
        .await
        .map_err(store_error)?
    else {
        return Ok(None);
    };
    let api_key = finite_private_api_key_from_row(&row)?;
    if api_key.status != FinitePrivateApiKeyStatus::Active {
        return Ok(None);
    }
    let Some(grant) = select_finite_private_grant(client, &api_key.grant_id, false).await? else {
        return Ok(None);
    };
    if grant.status != FinitePrivateGrantStatus::Active {
        return Ok(None);
    }
    Ok(Some((api_key, grant)))
}

/// Weekly usage for a grant across the rolling window, summed over its own
/// reservations only (row-scoped by grant_id). Returns the used units and the
/// reset instant (earliest in-window reservation + one week).
async fn postgres_finite_private_weekly_usage<C>(
    client: &C,
    grant_id: &str,
    window_start: &str,
    now: &str,
) -> CoreResult<(i64, Option<String>)>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT
           COALESCE(SUM(COALESCE(settled_usage_units, reserved_usage_units)), 0)::BIGINT AS used,
           CASE WHEN MIN(created_at) IS NULL THEN NULL ELSE {earliest} END AS earliest
         FROM finite_private_reservations
         WHERE grant_id = $1
           AND status <> 'denied'
           AND created_at >= $2::text::timestamptz
           AND created_at <= $3::text::timestamptz",
        earliest = rfc3339_col("MIN(created_at)"),
    );
    let row = client
        .query_one(&sql, &[&grant_id, &window_start, &now])
        .await
        .map_err(store_error)?;
    let used_units: i64 = row.get("used");
    let earliest: Option<String> = row.get("earliest");
    let reset_at = earliest
        .map(|earliest| {
            let parsed = parse_time(&earliest)?;
            (parsed + Duration::seconds(crate::FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS))
                .format(&Rfc3339)
                .map_err(CoreError::from)
        })
        .transpose()?;
    Ok((used_units, reset_at))
}

async fn postgres_approve_finite_private_grant<C>(
    client: &C,
    input: ApproveFinitePrivateGrantInput,
) -> CoreResult<FinitePrivateGrant>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let verified_email = normalize_owner_email(Some(&input.verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let limit_profile_id = trim_to_option(input.limit_profile_id.as_deref())
        .unwrap_or_else(|| crate::DEFAULT_FINITE_PRIVATE_LIMIT_PROFILE.to_string());
    let user = match trim_to_option(input.workos_user_id.as_deref()) {
        Some(workos_user_id) => {
            ensure_grandfathered_linked_user(client, &verified_email, &workos_user_id, &now).await?
        }
        None => ensure_pending_user_row(client, &verified_email, &now).await?,
    };
    approve_finite_private_grant_row(client, &user, &limit_profile_id, &now).await
}

async fn postgres_issue_finite_private_api_key<C>(
    client: &C,
    input: IssueFinitePrivateApiKeyInput,
) -> CoreResult<FinitePrivateApiKey>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let grant_id =
        trim_to_option(Some(&input.grant_id)).ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let grant = select_finite_private_grant(client, &grant_id, true)
        .await?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    issue_finite_private_api_key_row(
        client,
        &grant,
        &input.raw_key,
        trim_to_option(input.project_id.as_deref()),
        trim_to_option(input.agent_runtime_id.as_deref()),
        &now,
    )
    .await
}

async fn postgres_revoke_finite_private_grant<C>(
    client: &C,
    input: RevokeFinitePrivateGrantInput,
) -> CoreResult<FinitePrivateGrant>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let grant_id =
        trim_to_option(Some(&input.grant_id)).ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let row = client
        .query_opt(
            "UPDATE finite_private_grants
             SET status = 'revoked', updated_at = $2::text::timestamptz
             WHERE id = $1
             RETURNING id, user_id, limit_profile_id, status, current_window_started_at::text,
                       current_window_used_units, created_at::text, updated_at::text",
            &[&grant_id, &now],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let grant = finite_private_grant_from_row(&row)?;
    // Revoke every key under the grant (the in-memory model bumps them all).
    let revoked = client
        .query(
            "UPDATE finite_private_api_keys
             SET status = 'revoked', updated_at = $2::text::timestamptz
             WHERE grant_id = $1
             RETURNING id",
            &[&grant_id, &now],
        )
        .await
        .map_err(store_error)?;
    let revoked_api_key_ids: Vec<String> = revoked.iter().map(|row| row.get("id")).collect();
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.grant.revoke",
            target_type: "grant",
            target_id: &grant.id,
            grant_id: Some(&grant.id),
            api_key_id: None,
            actor: None,
            metadata: json!({ "revokedApiKeyIds": revoked_api_key_ids }),
            now: &now,
        },
    )
    .await?;
    Ok(grant)
}

async fn postgres_reset_finite_private_usage_window<C>(
    client: &C,
    input: ResetFinitePrivateUsageWindowInput,
) -> CoreResult<FinitePrivateGrant>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let grant_id =
        trim_to_option(Some(&input.grant_id)).ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let row = client
        .query_opt(
            "UPDATE finite_private_grants
             SET current_window_started_at = NULL,
                 current_window_used_units = 0,
                 updated_at = $2::text::timestamptz
             WHERE id = $1
             RETURNING id, user_id, limit_profile_id, status, current_window_started_at::text,
                       current_window_used_units, created_at::text, updated_at::text",
            &[&grant_id, &now],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let grant = finite_private_grant_from_row(&row)?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.grant.reset_window",
            target_type: "grant",
            target_id: &grant.id,
            grant_id: Some(&grant.id),
            api_key_id: None,
            actor: None,
            metadata: json!({}),
            now: &now,
        },
    )
    .await?;
    Ok(grant)
}

async fn postgres_rotate_finite_private_api_key<C>(
    client: &C,
    input: RotateFinitePrivateApiKeyInput,
) -> CoreResult<FinitePrivateApiKey>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let key_id =
        trim_to_option(Some(&input.key_id)).ok_or(CoreError::InvalidFinitePrivateApiKey)?;
    let old_row = client
        .query_opt(
            "SELECT id, grant_id, project_id, agent_runtime_id, key_hash, status,
                    created_at::text, updated_at::text
             FROM finite_private_api_keys WHERE id = $1 FOR UPDATE",
            &[&key_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::InvalidFinitePrivateApiKey)?;
    let old_key = finite_private_api_key_from_row(&old_row)?;
    let new_key_hash = hash_finite_private_api_key(&input.raw_key)?;
    if new_key_hash == old_key.key_hash {
        return Err(CoreError::InvalidFinitePrivateApiKey);
    }
    let grant = select_finite_private_grant(client, &old_key.grant_id, true)
        .await?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let new_key = issue_finite_private_api_key_row(
        client,
        &grant,
        &input.raw_key,
        old_key.project_id.clone(),
        old_key.agent_runtime_id.clone(),
        &now,
    )
    .await?;
    postgres_revoke_finite_private_api_key(
        client,
        RevokeFinitePrivateApiKeyInput {
            key_id: old_key.id.clone(),
            now: Some(now.clone()),
        },
    )
    .await?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.api_key.rotate",
            target_type: "api_key",
            target_id: &new_key.id,
            grant_id: Some(&new_key.grant_id),
            api_key_id: Some(&new_key.id),
            actor: None,
            metadata: json!({ "oldApiKeyId": old_key.id }),
            now: &now,
        },
    )
    .await?;
    Ok(new_key)
}

async fn postgres_admin_issue_finite_private_friend_key<C>(
    client: &C,
    input: AdminIssueFinitePrivateFriendKeyInput,
) -> CoreResult<AdminIssuedFinitePrivateKey>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let grant = postgres_approve_finite_private_grant(
        client,
        ApproveFinitePrivateGrantInput {
            verified_email: input.friend_email,
            workos_user_id: None,
            limit_profile_id: input.limit_profile_id,
            now: Some(now.clone()),
        },
    )
    .await?;
    let api_key = postgres_issue_finite_private_api_key(
        client,
        IssueFinitePrivateApiKeyInput {
            grant_id: grant.id.clone(),
            raw_key: input.raw_key,
            project_id: None,
            agent_runtime_id: None,
            now: Some(now.clone()),
        },
    )
    .await?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.friend_key.admin_issue",
            target_type: "api_key",
            target_id: &api_key.id,
            grant_id: Some(&grant.id),
            api_key_id: Some(&api_key.id),
            actor: Some(&admin_email),
            metadata: json!({ "limitProfileId": grant.limit_profile_id.clone() }),
            now: &now,
        },
    )
    .await?;
    Ok(AdminIssuedFinitePrivateKey { grant, api_key })
}

async fn postgres_admin_rotate_finite_private_api_key<C>(
    client: &C,
    input: AdminRotateFinitePrivateApiKeyInput,
) -> CoreResult<FinitePrivateApiKey>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let old_key_id = input.key_id.trim().to_string();
    let key = postgres_rotate_finite_private_api_key(
        client,
        RotateFinitePrivateApiKeyInput {
            key_id: input.key_id,
            raw_key: input.raw_key,
            now: Some(now.clone()),
        },
    )
    .await?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.api_key.admin_rotate",
            target_type: "api_key",
            target_id: &key.id,
            grant_id: Some(&key.grant_id),
            api_key_id: Some(&key.id),
            actor: Some(&admin_email),
            metadata: json!({ "oldApiKeyId": old_key_id }),
            now: &now,
        },
    )
    .await?;
    Ok(key)
}

async fn postgres_admin_revoke_finite_private_api_key<C>(
    client: &C,
    input: AdminRevokeFinitePrivateApiKeyInput,
) -> CoreResult<FinitePrivateApiKey>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let key = postgres_revoke_finite_private_api_key(
        client,
        RevokeFinitePrivateApiKeyInput {
            key_id: input.key_id,
            now: Some(now.clone()),
        },
    )
    .await?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.api_key.admin_revoke",
            target_type: "api_key",
            target_id: &key.id,
            grant_id: Some(&key.grant_id),
            api_key_id: Some(&key.id),
            actor: Some(&admin_email),
            metadata: json!({}),
            now: &now,
        },
    )
    .await?;
    Ok(key)
}

async fn postgres_admin_reset_finite_private_usage_window<C>(
    client: &C,
    input: AdminResetFinitePrivateUsageWindowInput,
) -> CoreResult<FinitePrivateGrant>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let grant = postgres_reset_finite_private_usage_window(
        client,
        ResetFinitePrivateUsageWindowInput {
            grant_id: input.grant_id,
            now: Some(now.clone()),
        },
    )
    .await?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.grant.admin_window_reset",
            target_type: "grant",
            target_id: &grant.id,
            grant_id: Some(&grant.id),
            api_key_id: None,
            actor: Some(&admin_email),
            metadata: json!({}),
            now: &now,
        },
    )
    .await?;
    Ok(grant)
}

async fn postgres_reserve_finite_private_usage<C>(
    client: &C,
    input: ReserveFinitePrivateUsageInput,
) -> CoreResult<FinitePrivateUsageDecision>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let now_time = parse_time(&now)?;
    let request_id = trim_to_option(Some(&input.request_id)).unwrap_or_else(|| {
        crate::id_from_parts("fp_request", &[&now, &input.endpoint, &input.model])
    });
    let dashboard_url = trim_to_option(Some(&input.dashboard_url))
        .unwrap_or_else(|| "https://finite.computer/dashboard".to_string());
    if input.estimated_usage_units <= 0
        || input.estimated_prompt_tokens < 0
        || input.estimated_completion_tokens < 0
    {
        return Err(CoreError::InvalidFinitePrivateUsageEstimate);
    }
    let Some((api_key, _)) =
        postgres_finite_private_key_and_grant(client, &input.presented_api_key).await?
    else {
        return Ok(crate::finite_private_denial(
            request_id,
            dashboard_url,
            "Finite Private API key is invalid or revoked.",
            "invalid_api_key",
            None,
            None,
        ));
    };
    // Re-read the grant FOR UPDATE to serialize concurrent reservations.
    let grant = select_finite_private_grant(client, &api_key.grant_id, true)
        .await?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let profile = select_finite_private_limit_profile(client, &grant.limit_profile_id)
        .await?
        .ok_or(CoreError::FinitePrivateLimitProfileNotFound)?;

    let reservation_id = crate::finite_private_reservation_id_for(&api_key.id, &request_id);
    let window_start = (now_time - Duration::seconds(crate::FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS))
        .format(&Rfc3339)?;
    let (weekly_used_units, weekly_reset_at) =
        postgres_finite_private_weekly_usage(client, &grant.id, &window_start, &now).await?;

    if let Some(existing) =
        select_finite_private_reservation(client, &reservation_id, false).await?
    {
        return Ok(crate::finite_private_allow_decision(
            existing.id,
            &profile,
            profile.burst_limit_units - grant.current_window_used_units,
            crate::finite_private_window_reset_at(&grant, &profile, now_time)?,
            profile
                .weekly_limit_units
                .map(|limit| limit - weekly_used_units),
            weekly_reset_at,
        ));
    }

    let (window_started_at, current_used_units, reset_at) =
        crate::finite_private_active_window(&grant, &profile, now_time)?;
    let remaining_before = profile.burst_limit_units - current_used_units;
    if input.estimated_usage_units > remaining_before {
        let retry_after = (parse_time(&reset_at)? - now_time).whole_seconds().max(0);
        return Ok(crate::finite_private_denial(
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
                (now_time + Duration::seconds(crate::FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS))
                    .format(&Rfc3339)
                    .unwrap_or_else(|_| now.clone())
            });
            let retry_after = (parse_time(&reset_at)? - now_time).whole_seconds().max(0);
            return Ok(crate::finite_private_denial(
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
    client
        .execute(
            "UPDATE finite_private_grants
             SET current_window_started_at = $2::text::timestamptz,
                 current_window_used_units = $3,
                 updated_at = $4::text::timestamptz
             WHERE id = $1",
            &[&grant.id, &window_started_at, &new_used_units, &now],
        )
        .await
        .map_err(store_error)?;
    let endpoint = crate::trim_or_fallback(&input.endpoint, "/v1/chat/completions");
    let model = crate::trim_or_fallback(&input.model, "kimi-k2-6");
    let usage_formula_version =
        crate::trim_or_fallback(&input.usage_formula_version, "2026-05-26.v1");
    client
        .execute(
            "INSERT INTO finite_private_reservations (
               id, request_id, api_key_id, grant_id, endpoint, model,
               estimated_usage_units, reserved_usage_units, settled_usage_units,
               settlement_kind, status, usage_formula_version, upstream_status,
               upstream_error_class, created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7, NULL, NULL, 'reserved', $8, NULL, NULL,
                     $9::text::timestamptz, $9::text::timestamptz)",
            &[
                &reservation_id,
                &request_id,
                &api_key.id,
                &grant.id,
                &endpoint,
                &model,
                &input.estimated_usage_units,
                &usage_formula_version,
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(crate::finite_private_allow_decision(
        reservation_id,
        &profile,
        profile.burst_limit_units - new_used_units,
        reset_at,
        profile
            .weekly_limit_units
            .map(|limit| limit - (weekly_used_units + input.estimated_usage_units)),
        weekly_reset_at.or_else(|| {
            profile.weekly_limit_units.map(|_| {
                (now_time + Duration::seconds(crate::FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS))
                    .format(&Rfc3339)
                    .unwrap_or_else(|_| now.clone())
            })
        }),
    ))
}

async fn postgres_settle_finite_private_reservation<C>(
    client: &C,
    input: SettleFinitePrivateReservationInput,
) -> CoreResult<SettleFinitePrivateReservationResult>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let reservation_id = trim_to_option(Some(&input.reservation_id))
        .ok_or(CoreError::FinitePrivateReservationNotFound)?;
    let request_id = trim_to_option(Some(&input.request_id))
        .ok_or(CoreError::FinitePrivateReservationNotFound)?;
    let existing = select_finite_private_reservation(client, &reservation_id, true)
        .await?
        .ok_or(CoreError::FinitePrivateReservationNotFound)?;
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
    let delta = settled_units - existing.reserved_usage_units;
    // Adjust the grant's burst usage by the settle delta (clamped at 0).
    client
        .execute(
            "UPDATE finite_private_grants
             SET current_window_used_units = GREATEST(current_window_used_units + $2, 0),
                 updated_at = $3::text::timestamptz
             WHERE id = $1",
            &[&existing.grant_id, &delta, &now],
        )
        .await
        .map_err(store_error)?;
    let usage_formula_version = crate::trim_or_fallback(
        &input.usage_formula_version,
        &existing.usage_formula_version,
    );
    let upstream_error_class = trim_to_option(input.upstream_error_class.as_deref());
    client
        .execute(
            "UPDATE finite_private_reservations
             SET status = 'settled',
                 settled_usage_units = $2,
                 settlement_kind = $3,
                 usage_formula_version = $4,
                 upstream_status = $5,
                 upstream_error_class = $6,
                 updated_at = $7::text::timestamptz
             WHERE id = $1",
            &[
                &reservation_id,
                &settled_units,
                &input.settlement.as_str(),
                &usage_formula_version,
                &input.upstream_status,
                &upstream_error_class,
                &now,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(SettleFinitePrivateReservationResult {
        settled: true,
        reservation_id,
    })
}

async fn postgres_finite_private_admin_audit_events<C>(
    client: &C,
) -> CoreResult<Vec<FinitePrivateAdminAuditEvent>>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT id, action, target_type, target_id, grant_id, api_key_id, actor, metadata,
                {created} AS created_at
         FROM finite_private_admin_audit_events
         ORDER BY created_at, id",
        created = rfc3339_col("created_at"),
    );
    client
        .query(&sql, &[])
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            Ok(FinitePrivateAdminAuditEvent {
                id: row.get("id"),
                action: row.get("action"),
                target_type: row.get("target_type"),
                target_id: row.get("target_id"),
                grant_id: row.get("grant_id"),
                api_key_id: row.get("api_key_id"),
                actor: row.get("actor"),
                metadata: json_column(row, "metadata")?,
                created_at: row.get("created_at"),
            })
        })
        .collect()
}

async fn postgres_finite_private_admin_state<C>(client: &C) -> CoreResult<FinitePrivateAdminState>
where
    C: GenericClient + Sync,
{
    let grant_sql = format!(
        "SELECT id, user_id, limit_profile_id, status,
                CASE WHEN current_window_started_at IS NULL THEN NULL
                     ELSE {started} END AS current_window_started_at,
                current_window_used_units, {created} AS created_at, {updated} AS updated_at
         FROM finite_private_grants
         ORDER BY created_at, id",
        started = rfc3339_col("current_window_started_at"),
        created = rfc3339_col("created_at"),
        updated = rfc3339_col("updated_at"),
    );
    let grants = client
        .query(&grant_sql, &[])
        .await
        .map_err(store_error)?
        .iter()
        .map(finite_private_grant_from_row)
        .collect::<CoreResult<Vec<_>>>()?;
    let key_sql = format!(
        "SELECT id, grant_id, project_id, agent_runtime_id, key_hash, status,
                {created} AS created_at, {updated} AS updated_at
         FROM finite_private_api_keys
         ORDER BY created_at, id",
        created = rfc3339_col("created_at"),
        updated = rfc3339_col("updated_at"),
    );
    let api_keys = client
        .query(&key_sql, &[])
        .await
        .map_err(store_error)?
        .iter()
        .map(finite_private_api_key_from_row)
        .collect::<CoreResult<Vec<_>>>()?;
    let admin_audit_events = postgres_finite_private_admin_audit_events(client).await?;
    Ok(FinitePrivateAdminState {
        grants,
        api_keys,
        admin_audit_events,
    })
}

fn json_column<T: DeserializeOwned>(row: &Row, name: &str) -> CoreResult<T> {
    let value: Value = row.get(name);
    serde_json::from_value(value).map_err(json_error)
}

fn optional_json_column(row: &Row, name: &str) -> CoreResult<Option<Value>> {
    Ok(row.get(name))
}

/// Convert a Postgres error into a structured `CoreError::Database`, preserving
/// the `as_db_error()` fields (SQLSTATE code, constraint, table, column, DETAIL)
/// that `error.to_string()` used to flatten into the useless string "db error".
/// The detail is log-only; the user-facing message stays generic.
fn store_error(error: tokio_postgres::Error) -> CoreError {
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
mod tests {
    use super::*;
    use crate::{
        FinitePrivateApiKeyStatus, RuntimeArtifactKind, RuntimeCapabilitiesEnvelope,
        RuntimeCapabilitiesV1,
    };
    use futures_util::FutureExt;
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn kata_runtime_capabilities() -> RuntimeCapabilitiesEnvelope {
        RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
            restart: true,
            recover_known_good_chat: false,
            runtime_upgrade: true,
            stop: true,
            runtime_retirement: false,
        })
    }

    /// Ephemeral-Postgres-per-test harness.
    ///
    /// The Postgres-gated tests used to SHARE one database. Because the
    /// agent-creation lease queue is still global (Phase 2 territory — the
    /// `WHERE status = 'requested' ... ORDER BY created_at` scan in
    /// `postgres_lease_agent_creation_request` picks the oldest row across ALL
    /// orgs), a leftover request from one test could be leased by another,
    /// forcing a process-wide mutex and per-test "drain the queue" cleanup.
    ///
    /// Instead, each test now gets its OWN freshly-created database, migrated
    /// from the schema, and dropped afterward. `FC_CORE_POSTGRES_TEST_URL`
    /// names the maintenance connection; the harness `CREATE DATABASE`s an
    /// isolated database under it (the default `postgres` superuser has
    /// CREATEDB). Tests are fully independent, run in parallel, and leak no
    /// state — re-running the whole suite twice against the same server is
    /// clean because the databases are torn down (and uniquely named besides).
    struct TestDb {
        store: CoreStore,
        url: String,
    }

    impl std::ops::Deref for TestDb {
        type Target = CoreStore;
        fn deref(&self) -> &CoreStore {
            &self.store
        }
    }

    static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    async fn issue_test_launch_code(store: &CoreStore, _now: &str) -> String {
        store
            .issue_launch_code_batch(IssueLaunchCodeBatchInput {
                name: "Postgres test batch".to_string(),
                code_count: 1,
                expires_in_hours: Some(crate::launch_codes::MAX_LAUNCH_CODE_BATCH_HOURS),
                hosting_tier: None,
                created_by_workos_user_id: "workos-test-operator".to_string(),
                now: None,
            })
            .await
            .unwrap()
            .codes[0]
            .code
            .clone()
    }

    /// Swap the database name in a `postgres://user:pass@host:port/db?query`
    /// URL, preserving auth, host, and any query string.
    fn replace_database(url: &str, db_name: &str) -> String {
        let (base, query) = match url.split_once('?') {
            Some((base, query)) => (base, Some(query)),
            None => (url, None),
        };
        let scheme_end = base.find("://").map(|idx| idx + 3).unwrap_or(0);
        let new_base = match base[scheme_end..].find('/') {
            Some(rel) => format!("{}/{db_name}", &base[..scheme_end + rel]),
            None => format!("{base}/{db_name}"),
        };
        match query {
            Some(query) => format!("{new_base}?{query}"),
            None => new_base,
        }
    }

    /// Run `test` against an isolated, migrated Postgres database. The database
    /// is dropped afterward even if the test body panics (the panic is
    /// re-raised so the test still fails). Returns without running when
    /// `FC_CORE_POSTGRES_TEST_URL` is unset, matching the previous gating.
    async fn with_isolated_postgres<F, Fut>(test: F)
    where
        F: FnOnce(TestDb) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let Ok(admin_url) = std::env::var("FC_CORE_POSTGRES_TEST_URL") else {
            return;
        };

        // Maintenance connection used only to CREATE/DROP the per-test database.
        let (admin, admin_conn) = tokio_postgres::connect(&admin_url, NoTls).await.unwrap();
        let admin_conn = tokio::spawn(async move {
            let _ = admin_conn.await;
        });

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let db_name = format!(
            "fc_test_{unique}_{}",
            TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        admin
            .execute(&format!("CREATE DATABASE \"{db_name}\""), &[])
            .await
            .unwrap();

        let url = replace_database(&admin_url, &db_name);
        let store = CoreStore::connect_postgres(&url).await.unwrap();
        store.migrate().await.unwrap();
        // Every creation test exercises the current Core contract: a request
        // is bound to an exact, promoted OCI artifact before it can lease.
        // Keep this older than test-specific promotions so focused artifact
        // tests still select their own fixture.
        store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: "artifact-postgres-fixture".to_string(),
                kind: crate::RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/agent-runtime:postgres-fixture@sha256:{}",
                    "f".repeat(64)
                ),
                version_label: "postgres-fixture".to_string(),
                source_git_sha: None,
                finitec_version: None,
                hermes_source_ref: None,
                finite_platform_plugin_ref: None,
                state_schema_version: "state-v1".to_string(),
                base_image: Some("python:3.11-trixie".to_string()),
                recover_known_good_chat: false,
                promoted: true,
                now: Some("2000-01-01T00:00:00Z".to_string()),
            })
            .await
            .unwrap();

        // Capture panics so the database is always torn down, then re-raise.
        let outcome = std::panic::AssertUnwindSafe(test(TestDb { store, url }))
            .catch_unwind()
            .await;

        // FORCE terminates any lingering connection (Postgres 13+), so teardown
        // never races the store/raw clients the test opened.
        let _ = admin
            .execute(
                &format!("DROP DATABASE IF EXISTS \"{db_name}\" WITH (FORCE)"),
                &[],
            )
            .await;
        drop(admin);
        admin_conn.abort();

        if let Err(panic) = outcome {
            std::panic::resume_unwind(panic);
        }
    }

    fn postgres_method_body(source: &str, method_name: &str) -> String {
        let signature = format!("    pub async fn {method_name}");
        let start = source
            .find(&signature)
            .unwrap_or_else(|| panic!("missing Postgres method {method_name}"));
        let rest = &source[start + signature.len()..];
        let end = rest
            .find("\n    pub async fn ")
            .or_else(|| rest.find("\n}\n\nfn "))
            .unwrap_or(rest.len());
        rest[..end].to_string()
    }

    /// Structural guard: with the global lock + full-state rewrite DELETED
    /// (Phase 2c), no Postgres store method can round-trip through a whole-DB
    /// snapshot — because the machinery no longer exists. This asserts both:
    /// (a) NO Postgres store method references the machinery, and (b) the
    /// machinery functions themselves are gone from the file entirely, so
    /// PERSISTENCE.md anti-patterns #1 (global advisory lock) and #2
    /// (load-all → mutate → persist-all) are physically impossible to
    /// reintroduce here without re-adding the deleted code.
    #[test]
    fn postgres_store_never_uses_full_state_persistence() {
        let source = include_str!("store.rs");

        // (b) The machinery is deleted: no definitions remain. The needles are
        // assembled from split fragments so this test's own source (it is
        // `include_str!`'d above) does not match itself.
        let deleted = [
            concat!("async fn ", "lock_state<C>"),
            concat!("async fn ", "load_state<C>"),
            concat!("async fn ", "persist_state<C>"),
            concat!("async fn ", "delete_missing_rows<C>"),
            concat!("pg_advisory", "_xact_lock"),
        ];
        for def in deleted {
            assert!(
                !source.contains(def),
                "full-state machinery `{def}` must be deleted, not merely unused"
            );
        }

        // (a) Belt-and-suspenders: scan every Postgres store method body and
        // assert none calls the (now non-existent) full-state helpers. Bound the
        // scan to the production code (exclude this test module's own literals).
        let impl_start = source
            .find("impl PostgresCoreStore {")
            .expect("missing Postgres store impl");
        let test_start = source[impl_start..]
            .find("#[cfg(test)]")
            .map(|idx| impl_start + idx)
            .unwrap_or(source.len());
        let impl_src = &source[impl_start..test_start];
        let mut rest = impl_src;
        while let Some(idx) = rest.find("    pub async fn ") {
            rest = &rest[idx + "    pub async fn ".len()..];
            let name_end = rest.find('(').unwrap_or(rest.len());
            let method_name = rest[..name_end].trim().to_string();
            let body = postgres_method_body(impl_src, &method_name);
            assert!(
                !body.contains("lock_state(")
                    && !body.contains("load_state(")
                    && !body.contains("persist_state("),
                "{method_name} must stay on row-scoped SQL helpers, not full-state persistence"
            );
        }
    }

    #[tokio::test]
    async fn postgres_launch_codes_are_one_time_metadata_only_and_idempotent() {
        with_isolated_postgres(|store| async move {
            let issued = store
                .issue_launch_code_batch(IssueLaunchCodeBatchInput {
                    name: "Internal canary".to_string(),
                    code_count: 3,
                    expires_in_hours: Some(1),
                    hosting_tier: None,
                    created_by_workos_user_id: "workos_operator".to_string(),
                    now: Some("2026-07-10T12:00:00Z".to_string()),
                })
                .await
                .unwrap();
            let batch_id = issued.batch.id.clone();
            let plaintext = issued.codes[0].code.clone();
            let unused = issued.codes[1].code.clone();
            let expiring = issued.codes[2].code.clone();

            let later = store.list_launch_code_batches().await.unwrap();
            let later_json = serde_json::to_string(&later).unwrap();
            assert!(!later_json.contains(&plaintext));
            assert!(!later_json.contains(&unused));
            assert!(serde_json::to_string(&issued).unwrap().contains(&plaintext));

            let created = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "canary@finite.vip".to_string(),
                    workos_user_id: "workos_canary".to_string(),
                    display_name: "Canary Agent".to_string(),
                    launch_code: plaintext.clone(),
                    idempotency_key: "canary-request".to_string(),
                    now: Some("2026-07-10T12:30:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_ne!(
                created.request.requested_launch_code.as_deref(),
                Some(plaintext.as_str())
            );

            store
                .revoke_launch_code_batch(RevokeLaunchCodeBatchInput {
                    batch_id: batch_id.clone(),
                    revoked_by_workos_user_id: "workos_operator".to_string(),
                    now: Some("2026-07-10T12:45:00Z".to_string()),
                })
                .await
                .unwrap();

            // Exact retries remain idempotent after both revocation and expiry.
            let replay = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "canary@finite.vip".to_string(),
                    workos_user_id: "workos_canary".to_string(),
                    display_name: "Ignored retry name".to_string(),
                    launch_code: plaintext.clone(),
                    idempotency_key: "canary-request".to_string(),
                    now: Some("2026-07-10T14:00:00Z".to_string()),
                })
                .await
                .unwrap();
            assert!(replay.reused);
            assert_eq!(replay.request.id, created.request.id);

            for (email, workos_id, key, code) in [
                (
                    "canary@finite.vip",
                    "workos_canary",
                    "different-request",
                    plaintext.as_str(),
                ),
                (
                    "other@finite.vip",
                    "workos_other",
                    "other-request",
                    plaintext.as_str(),
                ),
                (
                    "unused@finite.vip",
                    "workos_unused",
                    "unused-request",
                    unused.as_str(),
                ),
                (
                    "expired@finite.vip",
                    "workos_expired",
                    "expired-request",
                    expiring.as_str(),
                ),
            ] {
                let error = store
                    .request_agent_creation(RequestAgentCreationInput {
                        verified_email: email.to_string(),
                        workos_user_id: workos_id.to_string(),
                        display_name: "Rejected Agent".to_string(),
                        launch_code: code.to_string(),
                        idempotency_key: key.to_string(),
                        now: Some("2026-07-10T14:00:00Z".to_string()),
                    })
                    .await
                    .unwrap_err();
                assert!(matches!(error, CoreError::InvalidLaunchCode));
            }

            let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });
            let row = raw
                .query_one(
                    "SELECT code_hash,
                            (SELECT launch_code FROM agent_creation_entitlements
                              WHERE customer_org_id = $1) AS entitlement_code,
                            (SELECT requested_launch_code FROM agent_creation_requests
                              WHERE id = $2) AS request_code
                       FROM launch_codes WHERE id = $3",
                    &[
                        &created.request.customer_org_id,
                        &created.request.id,
                        &issued.codes[0].id,
                    ],
                )
                .await
                .unwrap();
            let code_hash: String = row.get("code_hash");
            let entitlement_code: Option<String> = row.get("entitlement_code");
            let request_code: Option<String> = row.get("request_code");
            assert_ne!(code_hash, plaintext);
            assert_eq!(
                entitlement_code.as_deref(),
                Some(issued.codes[0].id.as_str())
            );
            assert_eq!(request_code.as_deref(), Some(issued.codes[0].id.as_str()));
            drop(raw);
            connection.abort();
        })
        .await;
    }

    #[tokio::test]
    async fn postgres_launch_code_redemption_serializes_with_revocation() {
        with_isolated_postgres(|store| async move {
            let issued = store
                .issue_launch_code_batch(IssueLaunchCodeBatchInput {
                    name: "Revocation race".to_string(),
                    code_count: 1,
                    expires_in_hours: Some(24),
                    hosting_tier: None,
                    created_by_workos_user_id: "workos_operator".to_string(),
                    now: Some("2026-07-10T12:00:00Z".to_string()),
                })
                .await
                .unwrap();
            let batch_id = issued.batch.id.clone();
            let plaintext = issued.codes[0].code.clone();

            // Hold an uncommitted batch revocation. Redemption must block on
            // the batch row, then observe the committed revocation and fail.
            let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });
            let mut raw = raw;
            let tx = raw.transaction().await.unwrap();
            tx.execute(
                "UPDATE launch_code_batches
                    SET revoked_at = '2026-07-10T12:05:00Z'::timestamptz,
                        revoked_by_workos_user_id = 'workos_operator'
                  WHERE id = $1",
                &[&batch_id],
            )
            .await
            .unwrap();

            let competing = CoreStore::connect_postgres(&store.url).await.unwrap();
            let redeem = tokio::spawn(async move {
                competing
                    .request_agent_creation(RequestAgentCreationInput {
                        verified_email: "race@finite.vip".to_string(),
                        workos_user_id: "workos_race".to_string(),
                        display_name: "Race Agent".to_string(),
                        launch_code: plaintext,
                        idempotency_key: "race-request".to_string(),
                        now: Some("2026-07-10T12:10:00Z".to_string()),
                    })
                    .await
            });
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            assert!(!redeem.is_finished(), "redemption must wait for batch lock");
            tx.commit().await.unwrap();
            let error = redeem.await.unwrap().unwrap_err();
            assert!(matches!(error, CoreError::InvalidLaunchCode));

            let redeemed: i64 = raw
                .query_one(
                    "SELECT COUNT(*) FROM launch_codes
                      WHERE batch_id = $1 AND redeemed_at IS NOT NULL",
                    &[&batch_id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(redeemed, 0);
            drop(raw);
            connection.abort();
        })
        .await;
    }

    #[tokio::test]
    async fn postgres_launch_code_concurrent_redemption_has_one_winner() {
        with_isolated_postgres(|store| async move {
            let issued = store
                .issue_launch_code_batch(IssueLaunchCodeBatchInput {
                    name: "Concurrent redemption".to_string(),
                    code_count: 1,
                    expires_in_hours: Some(24),
                    hosting_tier: None,
                    created_by_workos_user_id: "workos_operator".to_string(),
                    now: Some("2026-07-10T12:00:00Z".to_string()),
                })
                .await
                .unwrap();
            let plaintext = issued.codes[0].code.clone();
            let first = CoreStore::connect_postgres(&store.url).await.unwrap();
            let second = CoreStore::connect_postgres(&store.url).await.unwrap();
            let (first_result, second_result) = tokio::join!(
                first.request_agent_creation(RequestAgentCreationInput {
                    verified_email: "first@finite.vip".to_string(),
                    workos_user_id: "workos_first".to_string(),
                    display_name: "First Agent".to_string(),
                    launch_code: plaintext.clone(),
                    idempotency_key: "first-request".to_string(),
                    now: Some("2026-07-10T12:30:00Z".to_string()),
                }),
                second.request_agent_creation(RequestAgentCreationInput {
                    verified_email: "second@finite.vip".to_string(),
                    workos_user_id: "workos_second".to_string(),
                    display_name: "Second Agent".to_string(),
                    launch_code: plaintext,
                    idempotency_key: "second-request".to_string(),
                    now: Some("2026-07-10T12:30:00Z".to_string()),
                }),
            );
            let successes = [first_result.as_ref(), second_result.as_ref()]
                .into_iter()
                .filter(|result| result.is_ok())
                .count();
            assert_eq!(successes, 1);
            let failures = [first_result, second_result]
                .into_iter()
                .filter_map(Result::err)
                .collect::<Vec<_>>();
            assert_eq!(failures.len(), 1);
            assert!(matches!(failures[0], CoreError::InvalidLaunchCode));

            let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });
            let redeemed: i64 = raw
                .query_one(
                    "SELECT COUNT(*) FROM launch_codes WHERE redeemed_at IS NOT NULL",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            let requests: i64 = raw
                .query_one("SELECT COUNT(*) FROM agent_creation_requests", &[])
                .await
                .unwrap()
                .get(0);
            assert_eq!(redeemed, 1);
            assert_eq!(requests, 1);
            drop(raw);
            connection.abort();
        })
        .await;
    }

    #[tokio::test]
    async fn postgres_fresh_launch_code_tops_up_exhausted_org_once() {
        with_isolated_postgres(|store| async move {
            let first_code = issue_test_launch_code(&store, "2026-07-10T12:00:00Z").await;
            let input = |launch_code: String, idempotency_key: &str, display_name: &str| {
                RequestAgentCreationInput {
                    verified_email: "top-up@finite.vip".to_string(),
                    workos_user_id: "workos_top_up".to_string(),
                    display_name: display_name.to_string(),
                    launch_code,
                    idempotency_key: idempotency_key.to_string(),
                    now: Some("2026-07-10T12:30:00Z".to_string()),
                }
            };
            store
                .request_agent_creation(input(first_code, "first-request", "First Agent"))
                .await
                .unwrap();

            let second_code = issue_test_launch_code(&store, "2026-07-10T13:00:00Z").await;
            let second = store
                .request_agent_creation(input(
                    second_code.clone(),
                    "second-request",
                    "Second Agent",
                ))
                .await
                .expect("a fresh code adds one creation to an exhausted org");
            assert!(!second.reused);

            let retry = store
                .request_agent_creation(input(second_code, "second-request", "Second Agent"))
                .await
                .unwrap();
            assert!(retry.reused);

            let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });
            let entitlement: i32 = raw
                .query_one(
                    "SELECT allowed_new_agent_runtimes FROM agent_creation_entitlements",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            let requests: i64 = raw
                .query_one("SELECT COUNT(*) FROM agent_creation_requests", &[])
                .await
                .unwrap()
                .get(0);
            assert_eq!(entitlement, 2, "the retry must not increment twice");
            assert_eq!(requests, 2);
            drop(raw);
            connection.abort();
        })
        .await;
    }

    #[tokio::test]
    async fn postgres_runtime_upgrade_migration_reapplies_and_rescue_refuses_active_work() {
        with_isolated_postgres(|store| async move {
            let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });

            raw.batch_execute(
                "ALTER TABLE runtime_control_requests
                   DROP CONSTRAINT runtime_control_requests_kind_check;
                 ALTER TABLE runtime_control_requests
                   ADD CONSTRAINT runtime_control_requests_kind_check
                   CHECK (kind IN ('restart', 'recover_known_good_chat_runtime', 'stop', 'destroy'));",
            )
            .await
            .unwrap();
            raw.batch_execute(include_str!("../migrations/0002_runtime_upgrade.sql"))
                .await
                .unwrap();
            let oid_before: u32 = raw
                .query_one(
                    "SELECT oid FROM pg_constraint
                     WHERE conrelid = 'runtime_control_requests'::regclass
                       AND conname = 'runtime_control_requests_kind_check'",
                    &[],
                )
                .await
                .unwrap()
                .get("oid");
            raw.batch_execute(include_str!("../migrations/0002_runtime_upgrade.sql"))
                .await
                .unwrap();
            let constraint = raw
                .query_one(
                    "SELECT oid, pg_get_constraintdef(oid) AS definition
                     FROM pg_constraint
                     WHERE conrelid = 'runtime_control_requests'::regclass
                       AND conname = 'runtime_control_requests_kind_check'",
                    &[],
                )
                .await
                .unwrap();
            assert_eq!(constraint.get::<_, u32>("oid"), oid_before);
            assert!(constraint.get::<_, String>("definition").contains("upgrade"));

            raw.batch_execute(
                r#"
                INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at)
                VALUES ('rescue-user', 'rescue@finite.vip', 'linked', 'workos-rescue', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                INSERT INTO customer_orgs (id, owner_user_id, name, billing_class, created_at, updated_at)
                VALUES ('rescue-org', 'rescue-user', 'Rescue', 'grandfathered', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, created_at, updated_at)
                VALUES ('rescue-project', 'rescue-org', 'rescue-user', 'Rescue', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                INSERT INTO runtime_artifacts (id, kind, reference, version_label, state_schema_version, created_at, promoted_at)
                VALUES ('rescue-artifact', 'oci_image', 'image@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'v1', 'state-v1', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                INSERT INTO agent_runtimes (
                  id, project_id, source_host_id, source_machine_id, source_import_key,
                  runtime_artifact_id, state_schema_version, host_facts, created_at, updated_at
                ) VALUES (
                  'rescue-runtime', 'rescue-project', 'rescue-host', 'rescue-machine',
                  'rescue-host/rescue-machine', 'rescue-artifact', 'state-v1', '{}'::jsonb,
                  CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
                );
                INSERT INTO runtime_control_requests (
                  id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                  requested_by_user_id, kind, target_runtime_artifact_id, status,
                  created_at, updated_at
                ) VALUES (
                  'rescue-request', 'rescue-project', 'rescue-runtime', 'rescue-host',
                  'rescue-machine', 'rescue-user', 'upgrade', 'rescue-artifact', 'requested',
                  CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
                );
                "#,
            )
            .await
            .unwrap();

            let active_error = raw
                .batch_execute(crate::RUNTIME_UPGRADE_ROLLBACK_RESCUE_SQL)
                .await
                .unwrap_err();
            let db_error = active_error
                .as_db_error()
                .expect("rollback rescue refusal must be a PostgreSQL error");
            assert_eq!(
                db_error.code(),
                &tokio_postgres::error::SqlState::RAISE_EXCEPTION
            );
            assert_eq!(
                db_error.message(),
                "runtime upgrade rollback rescue refused: active upgrade requests still exist"
            );
            raw.batch_execute("ROLLBACK").await.unwrap();
            assert_eq!(
                raw.query_one(
                    "SELECT kind FROM runtime_control_requests WHERE id = 'rescue-request'",
                    &[],
                )
                .await
                .unwrap()
                .get::<_, String>("kind"),
                "upgrade"
            );

            raw.execute(
                "UPDATE runtime_control_requests
                 SET status = 'succeeded', completed_at = CURRENT_TIMESTAMP
                 WHERE id = 'rescue-request'",
                &[],
            )
            .await
            .unwrap();
            raw.batch_execute(crate::RUNTIME_UPGRADE_ROLLBACK_RESCUE_SQL)
                .await
                .unwrap();
            assert_eq!(
                raw.query_one(
                    "SELECT kind FROM runtime_control_requests WHERE id = 'rescue-request'",
                    &[],
                )
                .await
                .unwrap()
                .get::<_, String>("kind"),
                "restart"
            );
            let audit_count: i64 = raw
                .query_one(
                    "SELECT count(*) FROM finite_private_admin_audit_events
                     WHERE action = 'runtime.upgrade.rollback_rescue'
                       AND target_id = 'rescue-request'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(audit_count, 1);
            drop(raw);
            connection.abort();
        })
        .await;
    }

    #[tokio::test]
    async fn postgres_failed_launch_atomically_revokes_its_provisioned_key() {
        with_isolated_postgres(|store| async move {
            let launch_code = issue_test_launch_code(&store, "2026-05-28T11:00:00Z").await;
            let created = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "failed-launch-key@finite.vip".to_string(),
                    workos_user_id: "workos_failed_launch_key".to_string(),
                    display_name: "Failed Launch Agent".to_string(),
                    launch_code,
                    idempotency_key: "failed-launch-key-submit".to_string(),
                    now: Some("2026-05-28T11:01:00Z".to_string()),
                })
                .await
                .unwrap();
            let lease = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-failed-launch-key".to_string(),
                    source_host_id: None,
                    lease_token: "lease-failed-launch-key".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: Some("2026-05-28T11:02:00Z".to_string()),
                })
                .await
                .unwrap()
                .expect("failed-launch request should lease");
            assert_eq!(lease.request.id, created.request.id);
            let provisioned = store
                .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                    request_id: lease.request.id.clone(),
                    runner_id: "runner-failed-launch-key".to_string(),
                    lease_token: "lease-failed-launch-key".to_string(),
                    source_host_id: Some("failed-launch-host".to_string()),
                    source_machine_id: Some("failed-launch-agent".to_string()),
                    now: Some("2026-05-28T11:03:00Z".to_string()),
                })
                .await
                .unwrap();

            let failed = store
                .fail_agent_creation_request(FailAgentCreationRequestInput {
                    request_id: lease.request.id,
                    runner_id: "runner-failed-launch-key".to_string(),
                    lease_token: "lease-failed-launch-key".to_string(),
                    failure_message: "runtime did not become ready".to_string(),
                    provisioned_finite_private_api_key_id: Some(provisioned.api_key.id.clone()),
                    now: Some("2026-05-28T11:04:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(failed.status, AgentCreationRequestStatus::Failed);

            let admin_state = store.finite_private_admin_state().await.unwrap();
            let key = admin_state
                .api_keys
                .iter()
                .find(|key| key.id == provisioned.api_key.id)
                .expect("provisioned key remains in metadata");
            assert_eq!(key.status, FinitePrivateApiKeyStatus::Revoked);
        })
        .await;
    }

    #[tokio::test]
    async fn postgres_provider_operation_ledger_replays_and_crosses_runtime_boundaries() {
        with_isolated_postgres(|store| async move {
            let launch_code = issue_test_launch_code(&store, "unused").await;
            let created = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "provider-ledger@finite.vip".to_string(),
                    workos_user_id: "workos_provider_ledger".to_string(),
                    display_name: "Provider Ledger".to_string(),
                    launch_code,
                    idempotency_key: "provider-ledger-create".to_string(),
                    now: None,
                })
                .await
                .unwrap();
            let request_id = created.request.id;
            let first = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "ledger-runner-a".to_string(),
                    lease_token: "ledger-token-a".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    source_host_id: None,
                    now: None,
                })
                .await
                .unwrap()
                .unwrap();
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
            let reserved = store
                .record_provider_operation_transition(input(
                    "ledger-runner-a",
                    "ledger-token-a",
                    "opaque-ledger-correlation",
                    ProviderOperationTransition::CorrelationReserved,
                ))
                .await
                .unwrap();
            let replay = store
                .record_provider_operation_transition(input(
                    "ledger-runner-a",
                    "ledger-token-a",
                    "opaque-ledger-correlation",
                    ProviderOperationTransition::CorrelationReserved,
                ))
                .await
                .unwrap();
            assert_eq!(replay, reserved);
            let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });
            raw.execute(
                "UPDATE agent_creation_requests
                 SET lease_expires_at = CURRENT_TIMESTAMP - interval '1 second'
                 WHERE id = $1",
                &[&request_id],
            )
            .await
            .unwrap();
            let expired_failure = store
                .fail_agent_creation_request(FailAgentCreationRequestInput {
                    request_id: request_id.clone(),
                    runner_id: "ledger-runner-a".to_string(),
                    lease_token: "ledger-token-a".to_string(),
                    failure_message: "stale worker failure".to_string(),
                    provisioned_finite_private_api_key_id: None,
                    now: None,
                })
                .await
                .unwrap_err();
            assert!(
                matches!(
                    expired_failure,
                    CoreError::AgentCreationRequestLeaseConflict
                ),
                "unexpected expired failure result: {expired_failure:?}"
            );
            let intact = raw
                .query_one(
                    "SELECT request.status,
                            (SELECT count(*)
                             FROM agent_creation_provider_operation_transitions transition
                             WHERE transition.agent_creation_request_id = request.id)
                     FROM agent_creation_requests request WHERE request.id = $1",
                    &[&request_id],
                )
                .await
                .unwrap();
            assert_eq!(intact.get::<_, String>(0), "launching");
            assert_eq!(intact.get::<_, i64>(1), 1);
            assert!(matches!(
                store
                    .record_provider_operation_transition(input(
                        "ledger-runner-a",
                        "wrong-token",
                        "opaque-ledger-correlation",
                        ProviderOperationTransition::Provisioned {
                            provider_facts: json!({}),
                        },
                    ))
                    .await,
                Err(CoreError::AgentCreationRequestLeaseConflict)
            ));
            let second = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "ledger-runner-b".to_string(),
                    lease_token: "ledger-token-b".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    source_host_id: None,
                    now: None,
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(second.request.id, first.request.id);
            assert_eq!(second.provider_operation.unwrap().v1().transitions.len(), 1);
            store
                .record_provider_operation_transition(input(
                    "ledger-runner-b",
                    "ledger-token-b",
                    "opaque-ledger-correlation",
                    ProviderOperationTransition::ProvisionStarted,
                ))
                .await
                .unwrap();
            assert!(matches!(
                store
                    .fail_agent_creation_request(FailAgentCreationRequestInput {
                        request_id: request_id.clone(),
                        runner_id: "ledger-runner-b".to_string(),
                        lease_token: "ledger-token-b".to_string(),
                        failure_message: "crashed after provider mutation started".to_string(),
                        provisioned_finite_private_api_key_id: None,
                        now: None,
                    })
                    .await,
                Err(CoreError::ProviderOperationBoundaryNotReached)
            ));
            assert!(matches!(
                store
                    .cancel_agent_creation_request(CancelAgentCreationRequestInput {
                        request_id: request_id.clone(),
                        now: None,
                    })
                    .await,
                Err(CoreError::ProviderOperationBoundaryNotReached)
            ));
            let started = raw
                .query_one(
                    "SELECT status,
                            (SELECT count(*)
                             FROM agent_creation_provider_operation_transitions transition
                             WHERE transition.agent_creation_request_id = request.id)
                     FROM agent_creation_requests request WHERE request.id = $1",
                    &[&request_id],
                )
                .await
                .unwrap();
            assert_eq!(started.get::<_, String>(0), "launching");
            assert_eq!(started.get::<_, i64>(1), 2);
            store
                .record_provider_operation_transition(input(
                    "ledger-runner-b",
                    "ledger-token-b",
                    "opaque-ledger-correlation",
                    ProviderOperationTransition::Provisioned {
                        provider_facts: json!({"provider_id": "opaque-ledger-runtime"}),
                    },
                ))
                .await
                .unwrap();
            let provisioned_key = store
                .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                    request_id: request_id.clone(),
                    runner_id: "ledger-runner-b".to_string(),
                    lease_token: "ledger-token-b".to_string(),
                    source_host_id: Some("ledger-host".to_string()),
                    source_machine_id: Some("ledger-machine".to_string()),
                    now: None,
                })
                .await
                .unwrap();
            assert!(matches!(
                store
                    .fail_agent_creation_request(FailAgentCreationRequestInput {
                        request_id: request_id.clone(),
                        runner_id: "ledger-runner-b".to_string(),
                        lease_token: "ledger-token-b".to_string(),
                        failure_message: "must remain resumable".to_string(),
                        provisioned_finite_private_api_key_id: Some(
                            provisioned_key.api_key.id.clone(),
                        ),
                        now: None,
                    })
                    .await,
                Err(CoreError::ProviderOperationBoundaryNotReached)
            ));
            assert_eq!(
                store
                    .finite_private_admin_state()
                    .await
                    .unwrap()
                    .api_keys
                    .into_iter()
                    .find(|key| key.id == provisioned_key.api_key.id)
                    .unwrap()
                    .status,
                FinitePrivateApiKeyStatus::Active
            );
            assert!(matches!(
                store
                    .cancel_agent_creation_request(CancelAgentCreationRequestInput {
                        request_id: request_id.clone(),
                        now: None,
                    })
                    .await,
                Err(CoreError::ProviderOperationBoundaryNotReached)
            ));
            assert_eq!(
                store
                    .finite_private_admin_state()
                    .await
                    .unwrap()
                    .api_keys
                    .into_iter()
                    .find(|key| key.id == provisioned_key.api_key.id)
                    .unwrap()
                    .status,
                FinitePrivateApiKeyStatus::Active
            );
            store
                .record_provider_operation_transition(input(
                    "ledger-runner-b",
                    "ledger-token-b",
                    "opaque-ledger-correlation",
                    ProviderOperationTransition::CommitStarted,
                ))
                .await
                .unwrap();

            let handle = crate::ProviderRuntimeHandleEnvelope::V1(crate::ProviderRuntimeHandleV1 {
                runner_class: crate::RunnerClass::Kata,
                opaque: json!({"sandbox_id": "opaque-ledger-runtime"}),
            });
            let completed = store
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: request_id.clone(),
                    runner_id: "ledger-runner-b".to_string(),
                    lease_token: "ledger-token-b".to_string(),
                    source_host_id: "ledger-host".to_string(),
                    source_machine_id: "ledger-machine".to_string(),
                    runtime_artifact_id: Some("artifact-postgres-fixture".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
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
                    now: None,
                })
                .await
                .unwrap();
            assert_eq!(
                completed.provider_operation.unwrap().v1().transitions.len(),
                6
            );
            let sequences = raw
                .query(
                    "SELECT sequence FROM agent_creation_provider_operation_transitions
                     WHERE agent_creation_request_id = $1 ORDER BY sequence",
                    &[&request_id],
                )
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.get::<_, i32>(0))
                .collect::<Vec<_>>();
            assert_eq!(sequences, vec![0, 1, 2, 3, 4, 5]);

            let current_code = issue_test_launch_code(&store, "unused").await;
            let current = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "provider-ledger-current@finite.vip".to_string(),
                    workos_user_id: "workos_provider_ledger_current".to_string(),
                    display_name: "Current Failure".to_string(),
                    launch_code: current_code,
                    idempotency_key: "provider-ledger-current".to_string(),
                    now: None,
                })
                .await
                .unwrap();
            store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "ledger-current".to_string(),
                    lease_token: "ledger-current-token".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    source_host_id: None,
                    now: None,
                })
                .await
                .unwrap()
                .unwrap();
            store
                .record_provider_operation_transition(RecordProviderOperationTransitionInput {
                    request_id: current.request.id.clone(),
                    runner_id: "ledger-current".to_string(),
                    lease_token: "ledger-current-token".to_string(),
                    correlation_id: "current-failure-correlation".to_string(),
                    placement,
                    transition: ProviderOperationTransition::CorrelationReserved,
                })
                .await
                .unwrap();
            let abandoned_key = store
                .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                    request_id: current.request.id.clone(),
                    runner_id: "ledger-current".to_string(),
                    lease_token: "ledger-current-token".to_string(),
                    source_host_id: None,
                    source_machine_id: None,
                    now: None,
                })
                .await
                .unwrap();
            let failed = store
                .fail_agent_creation_request(FailAgentCreationRequestInput {
                    request_id: current.request.id.clone(),
                    runner_id: "ledger-current".to_string(),
                    lease_token: "ledger-current-token".to_string(),
                    failure_message: "failed before provider mutation".to_string(),
                    provisioned_finite_private_api_key_id: None,
                    now: None,
                })
                .await
                .unwrap();
            assert_eq!(failed.status, AgentCreationRequestStatus::Failed);
            let cancelled = store
                .cancel_agent_creation_request(CancelAgentCreationRequestInput {
                    request_id: current.request.id,
                    now: None,
                })
                .await
                .unwrap();
            assert_eq!(cancelled.status, AgentCreationRequestStatus::Cancelled);
            assert_eq!(
                store
                    .finite_private_admin_state()
                    .await
                    .unwrap()
                    .api_keys
                    .into_iter()
                    .find(|key| key.id == abandoned_key.api_key.id)
                    .unwrap()
                    .status,
                FinitePrivateApiKeyStatus::Revoked
            );
            drop(raw);
            connection.abort();
        })
        .await;
    }

    #[tokio::test]
    async fn postgres_row_native_create_lease_complete_and_visible_reads() {
        with_isolated_postgres(|store| async move {
            let launch_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
            store
                .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                    id: "artifact-row-native-v1".to_string(),
                    kind: RuntimeArtifactKind::OciImage,
                    reference: format!(
                        "ghcr.io/finitecomputer/finite-agent-runtime:row-native-v1@sha256:{}",
                        "1".repeat(64)
                    ),
                    version_label: "row-native-v1".to_string(),
                    source_git_sha: None,
                    finitec_version: None,
                    hermes_source_ref: None,
                    finite_platform_plugin_ref: None,
                    state_schema_version: "state-v1".to_string(),
                    base_image: Some("python:3.11-trixie".to_string()),
                    recover_known_good_chat: false,
                    promoted: true,
                    now: Some("2026-05-28T12:00:00Z".to_string()),
                })
                .await
                .unwrap();

            let create = RequestAgentCreationInput {
                verified_email: "row-native@finite.vip".to_string(),
                workos_user_id: "workos_row_native".to_string(),
                display_name: "Row Native Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: "browser-submit-row-native".to_string(),
                now: Some("2026-05-28T12:01:00Z".to_string()),
            };
            let (first, second) = tokio::join!(
                store.request_agent_creation(create.clone()),
                store.request_agent_creation(create)
            );
            let first = first.unwrap();
            let second = second.unwrap();
            assert_eq!(first.request.id, second.request.id);
            assert!(first.reused ^ second.reused);

            let lease = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-row-native-1".to_string(),
                    source_host_id: None,
                    lease_token: "lease-row-native-1".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: Some("2026-05-28T12:02:00Z".to_string()),
                })
                .await
                .unwrap()
                .expect("row-native request should lease");
            assert_eq!(lease.request.status, AgentCreationRequestStatus::Launching);

            let visible_before = store
                .visible_projects_for_workos_user("workos_row_native")
                .await
                .unwrap();
            assert_eq!(visible_before.len(), 1);
            assert!(visible_before[0].runtime.is_none());

            let provisioned = store
                .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                    request_id: lease.request.id.clone(),
                    runner_id: "runner-row-native-1".to_string(),
                    lease_token: "lease-row-native-1".to_string(),
                    source_host_id: Some("row-native-host".to_string()),
                    source_machine_id: Some("row-native-agent-001".to_string()),
                    now: Some("2026-05-28T12:02:15Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(provisioned.grant.status, FinitePrivateGrantStatus::Active);
            assert_eq!(
                provisioned.api_key.status,
                FinitePrivateApiKeyStatus::Active
            );

            let runtime_token = "runtime-row-native-token";
            store
                .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                    request_id: lease.request.id.clone(),
                    runner_id: "runner-row-native-1".to_string(),
                    lease_token: "lease-row-native-1".to_string(),
                    source_host_id: "row-native-host".to_string(),
                    source_machine_id: "row-native-agent-001".to_string(),
                    runtime_artifact_id: Some("artifact-row-native-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: None,
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    runtime_relay_token_hash: runtime_relay_token_hash(runtime_token).unwrap(),
                    display_name: Some("Row Native Agent".to_string()),
                    hostname: None,
                    runtime_host: Some("row-native-host".to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Unknown),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: Some("2026-05-28T12:02:30Z".to_string()),
                })
                .await
                .unwrap();

            let heartbeat = store.record_runtime_heartbeat(runtime_token).await.unwrap();
            assert_eq!(heartbeat.machine_id, "row-native-agent-001");
            let observed = store
                .runtime_heartbeat_for_machine("row-native-agent-001")
                .await
                .unwrap();
            assert_eq!(observed.machine_id, "row-native-agent-001");

            let completed = store
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: lease.request.id.clone(),
                    runner_id: "runner-row-native-1".to_string(),
                    lease_token: "lease-row-native-1".to_string(),
                    source_host_id: "row-native-host".to_string(),
                    source_machine_id: "row-native-agent-001".to_string(),
                    runtime_artifact_id: Some("artifact-row-native-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: None,
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("Row Native Agent".to_string()),
                    hostname: None,
                    runtime_host: Some("row-native-host".to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: Some("2026-05-28T12:03:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(
                completed.request.status,
                AgentCreationRequestStatus::Running
            );

            let visible_after = store
                .visible_projects_for_workos_user("workos_row_native")
                .await
                .unwrap();
            assert_eq!(visible_after.len(), 1);
            assert_eq!(
                visible_after[0].runtime.as_ref().unwrap().source_machine_id,
                "row-native-agent-001"
            );
            let requests = store
                .agent_creation_requests_for_workos_user("workos_row_native")
                .await
                .unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].status, AgentCreationRequestStatus::Running);
        })
        .await;
    }

    #[tokio::test]
    async fn postgres_admin_ops_runtime_overview_and_finite_private_lifecycle() {
        with_isolated_postgres(|store| async move {
            let launch_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
            // Unique-per-run identifiers keep this test idempotent against an
            // accumulating test database.
            let run = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
                .to_string();
            let owner_email = format!("admin-ops-owner-{run}@finite.vip");
            let admin_email = format!("admin-ops-admin-{run}@finite.vip");
            let friend_email = format!("admin-ops-friend-{run}@finite.vip");
            let machine_id = format!("admin-ops-agent-{run}");

            store
                .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                    id: "artifact-admin-ops-v1".to_string(),
                    kind: RuntimeArtifactKind::OciImage,
                    reference: format!(
                        "ghcr.io/finitecomputer/finite-agent-runtime:admin-ops-v1@sha256:{}",
                        "2".repeat(64)
                    ),
                    version_label: "admin-ops-v1".to_string(),
                    source_git_sha: None,
                    finitec_version: None,
                    hermes_source_ref: None,
                    finite_platform_plugin_ref: None,
                    state_schema_version: "state-v1".to_string(),
                    base_image: Some("python:3.11-trixie".to_string()),
                    recover_known_good_chat: false,
                    promoted: true,
                    now: None,
                })
                .await
                .unwrap();

            let created = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: owner_email.clone(),
                    workos_user_id: format!("workos_admin_ops_owner_{run}"),
                    display_name: "Admin Ops Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: format!("admin-ops-{run}"),
                    now: None,
                })
                .await
                .unwrap();
            let lease = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: format!("runner-admin-ops-{run}"),
                    source_host_id: None,
                    lease_token: format!("lease-admin-ops-{run}"),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: None,
                })
                .await
                .unwrap()
                .expect("admin ops request should lease");
            assert_eq!(lease.request.id, created.request.id);
            let completed = store
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: lease.request.id.clone(),
                    runner_id: format!("runner-admin-ops-{run}"),
                    lease_token: format!("lease-admin-ops-{run}"),
                    source_host_id: "admin-ops-host".to_string(),
                    source_machine_id: machine_id.clone(),
                    runtime_artifact_id: Some("artifact-admin-ops-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: None,
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("Admin Ops Agent".to_string()),
                    hostname: None,
                    runtime_host: Some("admin-ops-host".to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: None,
                })
                .await
                .unwrap();
            let runtime_id = completed.request.agent_runtime_id.clone().unwrap();
            let project_id = completed.project.id.clone();

            // Provisioned-boxes overview reads back through Postgres state.
            let overviews = store.admin_runtime_overviews().await.unwrap();
            let overview = overviews
                .iter()
                .find(|overview| overview.agent_runtime_id == runtime_id)
                .expect("new runtime should appear in the admin overview");
            assert_eq!(overview.project_id, project_id);
            assert_eq!(overview.owner_email.as_deref(), Some(owner_email.as_str()));
            assert_eq!(
                overview.runtime_artifact_version_label.as_deref(),
                Some("admin-ops-v1")
            );
            assert_eq!(
                overview.runtime_capabilities,
                Some(*kata_runtime_capabilities().v1())
            );
            assert!(overview.runtime_link_active);

            // Admin restart persists a leasable control request.
            let restart = store
                .admin_request_runtime_restart(AdminRuntimeControlInput {
                    admin_verified_email: admin_email.clone(),
                    admin_workos_user_id: format!("workos_admin_ops_admin_{run}"),
                    project_id: project_id.clone(),
                    now: None,
                })
                .await
                .unwrap();
            assert_eq!(restart.agent_runtime_id, runtime_id);
            let control_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("runner-admin-ops-{run}"),
                    lease_token: format!("control-lease-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some("admin-ops-host".to_string()),
                    runner_capacity: Some(crate::RunnerLeaseCapacity {
                        runner_classes: vec![crate::RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..crate::RunnerLeaseCapacity::default()
                    }),
                    now: None,
                })
                .await
                .unwrap()
                .expect("admin restart should lease");
            assert_eq!(control_lease.request.id, restart.id);
            store
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: restart.id.clone(),
                    runner_id: format!("runner-admin-ops-{run}"),
                    lease_token: format!("control-lease-{run}"),
                    runtime_artifact_id: None,
                    state_schema_version: None,
                    runtime_capabilities: None,
                    runtime_host: None,
                    published_app_urls: None,
                    now: None,
                })
                .await
                .unwrap();

            // Friend key issue, rotate, and window reset persist round trips.
            let raw_key = format!("fpk_live_admin_ops_test_{run}");
            let issued = store
                .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                    admin_verified_email: admin_email.clone(),
                    friend_email: friend_email.clone(),
                    limit_profile_id: None,
                    raw_key: raw_key.clone(),
                    now: None,
                })
                .await
                .unwrap();
            assert_eq!(issued.grant.status, FinitePrivateGrantStatus::Active);
            assert_eq!(issued.api_key.status, FinitePrivateApiKeyStatus::Active);
            assert_ne!(issued.api_key.key_hash, raw_key);

            let rotated = store
                .admin_rotate_finite_private_api_key(AdminRotateFinitePrivateApiKeyInput {
                    admin_verified_email: admin_email.clone(),
                    key_id: issued.api_key.id.clone(),
                    raw_key: format!("fpk_live_admin_ops_rotated_{run}"),
                    now: None,
                })
                .await
                .unwrap();
            assert_ne!(rotated.id, issued.api_key.id);

            let admin_state = store.finite_private_admin_state().await.unwrap();
            let old_key = admin_state
                .api_keys
                .iter()
                .find(|key| key.id == issued.api_key.id)
                .unwrap();
            assert_eq!(old_key.status, FinitePrivateApiKeyStatus::Revoked);
            let new_key = admin_state
                .api_keys
                .iter()
                .find(|key| key.id == rotated.id)
                .unwrap();
            assert_eq!(new_key.status, FinitePrivateApiKeyStatus::Active);

            let revoked = store
                .admin_revoke_finite_private_api_key(AdminRevokeFinitePrivateApiKeyInput {
                    admin_verified_email: admin_email.clone(),
                    key_id: rotated.id.clone(),
                    now: None,
                })
                .await
                .unwrap();
            assert_eq!(revoked.status, FinitePrivateApiKeyStatus::Revoked);

            let reset = store
                .admin_reset_finite_private_usage_window(AdminResetFinitePrivateUsageWindowInput {
                    admin_verified_email: admin_email.clone(),
                    grant_id: issued.grant.id.clone(),
                    now: None,
                })
                .await
                .unwrap();
            assert_eq!(reset.current_window_used_units, 0);
            assert!(reset.current_window_started_at.is_none());

            // Every admin action is durably audited with the admin actor.
            let events = store.finite_private_admin_audit_events().await.unwrap();
            let admin_actions = events
                .iter()
                .filter(|event| event.actor == admin_email)
                .map(|event| event.action.clone())
                .collect::<Vec<_>>();
            for expected in [
                "runtime.admin_restart",
                "finite_private.friend_key.admin_issue",
                "finite_private.api_key.admin_rotate",
                "finite_private.api_key.admin_revoke",
                "finite_private.grant.admin_window_reset",
            ] {
                assert!(
                    admin_actions.contains(&expected.to_string()),
                    "missing Postgres audit action {expected}"
                );
            }
        })
        .await;
    }

    /// Row-scoped runtime-control lifecycle against Postgres: restart drives the
    /// runtime back Online, and destroy offboards it (link deactivated, relay
    /// credential dropped, and every Finite Private key bound to the runtime or
    /// project revoked) — all without the deleted full-state rewrite. Also
    /// exercises the enqueue dedup and the source-host-partitioned control lease.
    #[tokio::test]
    async fn postgres_runtime_control_lifecycle_row_scoped() {
        with_isolated_postgres(|store| async move {
            let launch_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
            let run = "rc-lifecycle";
            let email = format!("{run}@finite.vip");
            let workos = format!("workos_{run}");
            let host = "rchost";
            let machine = "rc-agent-001";

            store
                .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                    id: "artifact-rc-v1".to_string(),
                    kind: RuntimeArtifactKind::OciImage,
                    reference: format!(
                        "ghcr.io/finitecomputer/finite-agent-runtime:rc-v1@sha256:{}",
                        "3".repeat(64)
                    ),
                    version_label: "rc-v1".to_string(),
                    source_git_sha: None,
                    finitec_version: None,
                    hermes_source_ref: None,
                    finite_platform_plugin_ref: None,
                    state_schema_version: "state-v1".to_string(),
                    base_image: None,
                    recover_known_good_chat: false,
                    promoted: true,
                    now: None,
                })
                .await
                .unwrap();
            store
                .request_agent_creation_configured(
                    RequestAgentCreationInput {
                        verified_email: email.clone(),
                        workos_user_id: workos.clone(),
                        display_name: "RC Agent".to_string(),
                        launch_code: launch_code.clone(),
                        idempotency_key: format!("{run}-submit"),
                        now: None,
                    },
                    AgentCreationConfiguration {
                        placement: Some(RuntimePlacement::for_hosting_tier(HostingTier::Standard)),
                        profile_picture_url: None,
                    },
                )
                .await
                .unwrap();
            let lease = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: format!("runner-{run}"),
                    source_host_id: None,
                    lease_token: format!("lease-{run}"),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: None,
                })
                .await
                .unwrap()
                .expect("request should lease");
            // A Finite Private key bound to the runtime, to prove destroy revokes it.
            let provisioned = store
                .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                    request_id: lease.request.id.clone(),
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("lease-{run}"),
                    source_host_id: Some(host.to_string()),
                    source_machine_id: Some(machine.to_string()),
                    now: None,
                })
                .await
                .unwrap();
            let completed = store
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: lease.request.id.clone(),
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("lease-{run}"),
                    source_host_id: host.to_string(),
                    source_machine_id: machine.to_string(),
                    runtime_artifact_id: Some("artifact-rc-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: None,
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("RC Agent".to_string()),
                    hostname: None,
                    runtime_host: Some(host.to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: None,
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: None,
                })
                .await
                .unwrap();
            let project_id = completed.project.id.clone();
            let runtime_id = completed.request.agent_runtime_id.clone().unwrap();
            let unrelated_project_id = format!("project-unrelated-{run}");
            let unrelated_membership_id = format!("membership-unrelated-{run}");
            let (raw, raw_connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let raw_connection = tokio::spawn(async move {
                let _ = raw_connection.await;
            });
            raw.execute(
                "INSERT INTO projects (
                   id, customer_org_id, owner_user_id, display_name, created_at, updated_at
                 )
                 SELECT $2, customer_org_id, owner_user_id, 'Unrelated Agent',
                        CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
                 FROM projects WHERE id = $1",
                &[&project_id, &unrelated_project_id],
            )
            .await
            .unwrap();
            raw.execute(
                "INSERT INTO project_room_memberships (
                   id, project_id, chat_identity_id, role, created_at
                 )
                 SELECT $2, $3, chat_identity_id, role, CURRENT_TIMESTAMP
                 FROM project_room_memberships
                 WHERE project_id = $1 AND archived_at IS NULL
                 LIMIT 1",
                &[&project_id, &unrelated_membership_id, &unrelated_project_id],
            )
            .await
            .unwrap();
            drop(raw);
            raw_connection.abort();
            let visible_before_destroy = store
                .visible_projects_for_workos_user(&workos)
                .await
                .unwrap()
                .into_iter()
                .map(|visible| visible.project.id)
                .collect::<BTreeSet<_>>();
            assert_eq!(
                visible_before_destroy,
                BTreeSet::from([project_id.clone(), unrelated_project_id.clone()])
            );

            let exact_artifact_retry = UpsertRuntimeArtifactInput {
                id: "artifact-rc-v1".to_string(),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/finite-agent-runtime:rc-v1@sha256:{}",
                    "3".repeat(64)
                ),
                version_label: "rc-v1".to_string(),
                source_git_sha: None,
                finitec_version: None,
                hermes_source_ref: None,
                finite_platform_plugin_ref: None,
                state_schema_version: "state-v1".to_string(),
                base_image: None,
                recover_known_good_chat: false,
                promoted: true,
                now: None,
            };
            store
                .upsert_runtime_artifact(exact_artifact_retry.clone())
                .await
                .unwrap();
            let mut material_mutation = exact_artifact_retry;
            material_mutation.version_label = "mutated-in-place".to_string();
            assert!(matches!(
                store
                    .upsert_runtime_artifact(material_mutation)
                    .await
                    .unwrap_err(),
                CoreError::RuntimeArtifactImmutable
            ));

            // Restart: enqueue is deduped (same in-flight request), leased only by
            // the runtime's own source host, and completion drives it Online.
            let restart = store
                .request_runtime_restart(RequestRuntimeRestartInput {
                    verified_email: email.clone(),
                    workos_user_id: workos.clone(),
                    project_id: project_id.clone(),
                    now: None,
                })
                .await
                .unwrap();
            let restart_again = store
                .request_runtime_restart(RequestRuntimeRestartInput {
                    verified_email: email.clone(),
                    workos_user_id: workos.clone(),
                    project_id: project_id.clone(),
                    now: None,
                })
                .await
                .unwrap();
            assert_eq!(restart.id, restart_again.id, "enqueue must dedup in-flight");

            // A runner on a DIFFERENT host must not claim this request.
            let other_host_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("runner-other-{run}"),
                    lease_token: format!("ctl-other-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some("someotherhost".to_string()),
                    runner_capacity: None,
                    now: None,
                })
                .await
                .unwrap();
            assert!(other_host_lease.is_none(), "partitioned by source host");

            let wrong_class_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("phala-runner-{run}"),
                    lease_token: format!("ctl-phala-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some(host.to_string()),
                    runner_capacity: Some(crate::RunnerLeaseCapacity {
                        runner_classes: vec![crate::RunnerClass::Phala],
                        ..crate::RunnerLeaseCapacity::default()
                    }),
                    now: None,
                })
                .await
                .unwrap();
            assert!(
                wrong_class_lease.is_none(),
                "Phala worker must not claim Kata control work"
            );

            let unspecified_class_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("unspecified-runner-{run}"),
                    lease_token: format!("ctl-unspecified-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some(host.to_string()),
                    runner_capacity: Some(crate::RunnerLeaseCapacity::default()),
                    now: None,
                })
                .await
                .unwrap();
            assert!(
                unspecified_class_lease.is_none(),
                "empty advertised class set supports nothing"
            );

            let control_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("ctl-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some(host.to_string()),
                    runner_capacity: Some(crate::RunnerLeaseCapacity {
                        draining: true,
                        runner_classes: vec![crate::RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..crate::RunnerLeaseCapacity::default()
                    }),
                    now: None,
                })
                .await
                .unwrap()
                .expect("draining host runner should still lease its own control request");
            assert_eq!(control_lease.request.id, restart.id);
            store
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: restart.id.clone(),
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("ctl-{run}"),
                    runtime_artifact_id: None,
                    state_schema_version: None,
                    runtime_capabilities: None,
                    runtime_host: None,
                    published_app_urls: None,
                    now: None,
                })
                .await
                .unwrap();
            let overview_online = store
                .admin_runtime_overviews()
                .await
                .unwrap()
                .into_iter()
                .find(|o| o.agent_runtime_id == runtime_id)
                .unwrap();
            assert_eq!(overview_online.runtime_status, RuntimeSummaryStatus::Online);
            assert!(overview_online.runtime_link_active);

            // Upgrade: target is an explicit promoted, digest-pinned artifact;
            // the lease carries it and completion updates artifact/endpoint
            // facts without offboarding the Runtime or revoking its key.
            store
                .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                    id: "artifact-rc-v2".to_string(),
                    kind: RuntimeArtifactKind::OciImage,
                    reference: format!(
                        "ghcr.io/finitecomputer/agent-runtime:v2@sha256:{}",
                        "b".repeat(64)
                    ),
                    version_label: "v2".to_string(),
                    source_git_sha: Some("git-v2".to_string()),
                    finitec_version: None,
                    hermes_source_ref: Some("0.18.2".to_string()),
                    finite_platform_plugin_ref: Some("plugin-v2".to_string()),
                    state_schema_version: "state-v1".to_string(),
                    base_image: None,
                    recover_known_good_chat: true,
                    promoted: true,
                    now: None,
                })
                .await
                .unwrap();
            let upgrade = store
                .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                    admin_verified_email: format!("admin-{run}@finite.vip"),
                    admin_workos_user_id: format!("admin-workos-{run}"),
                    project_id: project_id.clone(),
                    target_runtime_artifact_id: "artifact-rc-v2".to_string(),
                    now: None,
                })
                .await
                .unwrap();
            let conflicting_stop = store
                .request_runtime_stop(RequestRuntimeStopInput {
                    verified_email: email.clone(),
                    workos_user_id: workos.clone(),
                    project_id: project_id.clone(),
                    now: None,
                })
                .await
                .unwrap_err();
            assert!(matches!(
                conflicting_stop,
                CoreError::RuntimeControlOperationConflict
            ));

            let (raw, raw_connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let raw_connection = tokio::spawn(async move {
                let _ = raw_connection.await;
            });
            raw.execute(
                "UPDATE runtime_artifacts
                 SET retired_at = GREATEST(clock_timestamp(), promoted_at)
                 WHERE id = 'artifact-rc-v2'",
                &[],
            )
            .await
            .unwrap();
            raw.execute(
                "INSERT INTO agent_runtimes (
                   id, project_id, source_host_id, source_machine_id, source_import_key,
                   runtime_artifact_id, state_schema_version,
                   placement_runner_class, runtime_resource_class, runtime_capabilities,
                   host_facts, created_at, updated_at
                 )
                 SELECT 'runtime-healthy-behind-poison', project_id, source_host_id,
                        'healthy-behind-poison', 'rchost/healthy-behind-poison',
                        runtime_artifact_id, state_schema_version,
                        placement_runner_class, runtime_resource_class, runtime_capabilities,
                        host_facts,
                        CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
                 FROM agent_runtimes WHERE id = $1",
                &[&runtime_id],
            )
            .await
            .unwrap();
            raw.execute(
                "INSERT INTO runtime_control_requests (
                   id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                   requested_by_user_id, kind, status, created_at, updated_at
                 )
                 SELECT 'runtime_ctl_healthy_behind_poison', $1,
                        'runtime-healthy-behind-poison', $2, 'healthy-behind-poison',
                        owner_user_id, 'restart', 'requested',
                        CURRENT_TIMESTAMP + INTERVAL '1 second',
                        CURRENT_TIMESTAMP + INTERVAL '1 second'
                 FROM projects WHERE id = $1",
                &[&project_id, &host],
            )
            .await
            .unwrap();
            let healthy_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("ctl-retired-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some(host.to_string()),
                    runner_capacity: Some(crate::RunnerLeaseCapacity {
                        runner_classes: vec![crate::RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..crate::RunnerLeaseCapacity::default()
                    }),
                    now: None,
                })
                .await
                .unwrap()
                .expect("poisoned upgrade must not starve a healthy request");
            assert_eq!(
                healthy_lease.request.id,
                "runtime_ctl_healthy_behind_poison"
            );
            let poisoned = raw
                .query_one(
                    "SELECT status, failure_message
                     FROM runtime_control_requests WHERE id = $1",
                    &[&upgrade.id],
                )
                .await
                .unwrap();
            assert_eq!(poisoned.get::<_, String>("status"), "failed");
            assert!(
                poisoned
                    .get::<_, Option<String>>("failure_message")
                    .unwrap_or_default()
                    .contains("retired")
            );
            raw.execute(
                "UPDATE runtime_artifacts SET retired_at = NULL WHERE id = 'artifact-rc-v2'",
                &[],
            )
            .await
            .unwrap();
            raw.execute(
                "UPDATE agent_creation_requests SET runtime_spec = NULL
                 WHERE agent_runtime_id = $1",
                &[&runtime_id],
            )
            .await
            .unwrap();
            let upgrade = store
                .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                    admin_verified_email: format!("admin-{run}@finite.vip"),
                    admin_workos_user_id: format!("admin-workos-{run}"),
                    project_id: project_id.clone(),
                    target_runtime_artifact_id: "artifact-rc-v2".to_string(),
                    now: Some("2026-07-10T12:00:01Z".to_string()),
                })
                .await
                .unwrap();
            let upgrade_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("ctl-upgrade-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some(host.to_string()),
                    runner_capacity: Some(crate::RunnerLeaseCapacity {
                        runner_classes: vec![crate::RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..crate::RunnerLeaseCapacity::default()
                    }),
                    now: None,
                })
                .await
                .unwrap()
                .expect("upgrade should lease");
            assert_eq!(upgrade_lease.request.id, upgrade.id);
            assert_eq!(
                upgrade_lease
                    .target_runtime_artifact
                    .as_ref()
                    .map(|artifact| artifact.id.as_str()),
                Some("artifact-rc-v2")
            );
            raw.execute(
                "UPDATE runtime_artifacts
                 SET retired_at = GREATEST(clock_timestamp(), promoted_at)
                 WHERE id = 'artifact-rc-v2'",
                &[],
            )
            .await
            .unwrap();
            store
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: upgrade.id.clone(),
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("ctl-upgrade-{run}"),
                    runtime_artifact_id: Some("artifact-rc-v2".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                        RuntimeCapabilitiesV1 {
                            recover_known_good_chat: true,
                            ..*kata_runtime_capabilities().v1()
                        },
                    )),
                    runtime_host: Some("http://127.0.0.1:41002".to_string()),
                    published_app_urls: Some(vec!["http://127.0.0.1:41002/contact".to_string()]),
                    now: None,
                })
                .await
                .unwrap();
            let refreshed_capabilities: Value = raw
                .query_one(
                    "SELECT runtime_capabilities FROM agent_runtimes WHERE id = $1",
                    &[&runtime_id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(
                refreshed_capabilities["capabilities"]["recover_known_good_chat"],
                true
            );
            let upgraded_spec: Value = raw
                .query_one(
                    "SELECT runtime_spec FROM agent_creation_requests
                     WHERE agent_runtime_id = $1",
                    &[&runtime_id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(upgraded_spec["spec"]["runtimeArtifactId"], "artifact-rc-v2");
            assert_eq!(
                upgraded_spec["spec"]["durableStateId"], machine,
                "legacy synthesis preserves the source-machine /data directory"
            );
            drop(raw);
            raw_connection.abort();
            let upgraded = store
                .admin_runtime_overviews()
                .await
                .unwrap()
                .into_iter()
                .find(|overview| overview.agent_runtime_id == runtime_id)
                .unwrap();
            assert_eq!(
                upgraded.runtime_artifact_id.as_deref(),
                Some("artifact-rc-v2")
            );
            assert!(upgraded.runtime_link_active);
            let key_before_destroy = store
                .finite_private_admin_state()
                .await
                .unwrap()
                .api_keys
                .into_iter()
                .find(|key| key.id == provisioned.api_key.id)
                .unwrap();
            assert_eq!(key_before_destroy.status, FinitePrivateApiKeyStatus::Active);

            // Retirement remains outside the authorized product surface even
            // after a capability-refreshing Upgrade.
            let destroy_error = store
                .request_runtime_destroy(RequestRuntimeDestroyInput {
                    verified_email: email.clone(),
                    workos_user_id: workos.clone(),
                    project_id: project_id.clone(),
                    now: Some("2026-07-10T12:04:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(
                destroy_error,
                CoreError::RuntimeControlUnsupported
            ));
        })
        .await;
    }

    /// Row-scoped reconcile + claim against Postgres: reconcile mints an import
    /// candidate resolved by its natural key (source_import_key) with a surrogate
    /// id, a re-reconcile updates the same row, and claim materializes a project +
    /// runtime (fresh surrogate ids) that the owner can then see. Re-claim is
    /// idempotent and a missing candidate id is reported, not fabricated.
    #[tokio::test]
    async fn postgres_reconcile_and_claim_import_row_scoped() {
        with_isolated_postgres(|store| async move {
            let run = "import-flow";
            let email = format!("{run}@finite.vip");
            let workos = format!("workos_{run}");

            let record = ExistingHostProjectImport {
                source_host_id: "imphost".to_string(),
                source_machine_id: "imp-agent-001".to_string(),
                owner_email: Some(email.clone()),
                display_name: "Imported Agent".to_string(),
                hostname: None,
                runtime_host: Some("imphost".to_string()),
                runtime_status: RuntimeSummaryStatus::Unknown,
                active_inference_profile: None,
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                known_external_channel_participants: Vec::new(),
                admin_visible_to_emails: Vec::new(),
            };
            let report = store
                .reconcile_existing_host_imports(
                    vec![record.clone()],
                    ReconcileExistingHostImportsOptions {
                        allowlisted_owner_emails: vec![email.clone()],
                        now: None,
                    },
                )
                .await
                .unwrap();
            assert_eq!(report.created_candidates.len(), 1);
            let candidate_id = report.created_candidates[0].clone();
            assert!(
                candidate_id.starts_with("import_"),
                "candidate id must be a surrogate, got {candidate_id}"
            );

            let claimable = store
                .claimable_candidates_for_email(Some(&email))
                .await
                .unwrap();
            assert_eq!(claimable.len(), 1);
            assert_eq!(claimable[0].id, candidate_id);

            // Re-reconcile updates the same row (natural-key resolution).
            let report2 = store
                .reconcile_existing_host_imports(
                    vec![record],
                    ReconcileExistingHostImportsOptions {
                        allowlisted_owner_emails: vec![email.clone()],
                        now: None,
                    },
                )
                .await
                .unwrap();
            assert!(report2.created_candidates.is_empty());
            assert_eq!(report2.updated_candidates, vec![candidate_id.clone()]);

            let claim = store
                .claim_project_imports(ClaimProjectImportsInput {
                    verified_email: email.clone(),
                    workos_user_id: workos.clone(),
                    selected_candidate_ids: vec![
                        candidate_id.clone(),
                        "does-not-exist".to_string(),
                    ],
                    now: None,
                })
                .await
                .unwrap();
            assert_eq!(claim.claimed_project_ids.len(), 1);
            assert_eq!(
                claim.missing_candidate_ids,
                vec!["does-not-exist".to_string()]
            );
            let project_id = claim.claimed_project_ids[0].clone();
            assert!(project_id.starts_with("project_"), "surrogate project id");

            let visible = store
                .visible_projects_for_workos_user(&workos)
                .await
                .unwrap();
            assert_eq!(visible.len(), 1);
            assert_eq!(visible[0].project.id, project_id);
            assert_eq!(
                visible[0].runtime.as_ref().unwrap().source_machine_id,
                "imp-agent-001"
            );

            // Re-claim is idempotent: already-claimed, nothing newly claimed.
            let reclaim = store
                .claim_project_imports(ClaimProjectImportsInput {
                    verified_email: email.clone(),
                    workos_user_id: workos.clone(),
                    selected_candidate_ids: vec![candidate_id],
                    now: None,
                })
                .await
                .unwrap();
            assert!(reclaim.claimed_project_ids.is_empty());
            assert_eq!(reclaim.already_claimed_project_ids, vec![project_id]);
        })
        .await;
    }

    #[tokio::test]
    async fn postgres_imported_runtime_does_not_consume_self_serve_launch_entitlement() {
        with_isolated_postgres(|store| async move {
            let launch_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
            let email = "postgres-import-with-launch@finite.vip".to_string();
            let workos_user_id = "workos_postgres_import_with_launch".to_string();
            let record = ExistingHostProjectImport {
                source_host_id: "legacy-host".to_string(),
                source_machine_id: "legacy-agent-001".to_string(),
                owner_email: Some(email.clone()),
                display_name: "Imported Agent".to_string(),
                hostname: None,
                runtime_host: Some("legacy-host".to_string()),
                runtime_status: RuntimeSummaryStatus::Online,
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                known_external_channel_participants: Vec::new(),
                admin_visible_to_emails: Vec::new(),
            };
            let reconciled = store
                .reconcile_existing_host_imports(
                    vec![record],
                    ReconcileExistingHostImportsOptions {
                        allowlisted_owner_emails: vec![email.clone()],
                        now: None,
                    },
                )
                .await
                .unwrap();
            let candidate_id = reconciled.created_candidates[0].clone();
            let claimed = store
                .claim_project_imports(ClaimProjectImportsInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    selected_candidate_ids: vec![candidate_id.clone()],
                    now: None,
                })
                .await
                .unwrap();
            let imported_project_id = claimed.claimed_project_ids[0].clone();
            let imported_before = store
                .visible_projects_for_workos_user(&workos_user_id)
                .await
                .unwrap()
                .into_iter()
                .find(|visible| visible.project.id == imported_project_id)
                .expect("claimed import must remain visible");
            let imported_runtime_id = imported_before
                .runtime
                .as_ref()
                .expect("claimed import must expose its runtime")
                .id
                .clone();

            let created = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    display_name: "New Hosted Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "first-self-serve-submit".to_string(),
                    now: None,
                })
                .await
                .expect("an imported runtime must not consume the hosted launch");
            assert!(created.project.import_candidate_id.is_none());

            let exhausted = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    display_name: "Another Hosted Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "second-self-serve-submit".to_string(),
                    now: None,
                })
                .await
                .unwrap_err();
            assert!(matches!(exhausted, CoreError::InvalidLaunchCode));

            let requests = store
                .agent_creation_requests_for_workos_user(&workos_user_id)
                .await
                .unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].id, created.request.id);

            let imported_after = store
                .visible_projects_for_workos_user(&workos_user_id)
                .await
                .unwrap()
                .into_iter()
                .find(|visible| visible.project.id == imported_project_id)
                .expect("launch attempts must preserve the imported project");
            assert_eq!(imported_after, imported_before);

            let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });
            let candidate = raw
                .query_one(
                    "SELECT status, project_id, agent_runtime_id
                     FROM project_import_candidates WHERE id = $1",
                    &[&candidate_id],
                )
                .await
                .unwrap();
            assert_eq!(candidate.get::<_, String>("status"), "claimed");
            assert_eq!(
                candidate.get::<_, Option<String>>("project_id").as_deref(),
                Some(imported_project_id.as_str())
            );
            assert_eq!(
                candidate
                    .get::<_, Option<String>>("agent_runtime_id")
                    .as_deref(),
                Some(imported_runtime_id.as_str())
            );
            let active_link_count: i64 = raw
                .query_one(
                    "SELECT COUNT(*) FROM project_runtime_links
                     WHERE project_id = $1 AND agent_runtime_id = $2 AND active = TRUE",
                    &[&imported_project_id, &imported_runtime_id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(active_link_count, 1);
            drop(raw);
            connection.abort();
        })
        .await;
    }

    #[tokio::test]
    async fn postgres_owner_can_archive_imported_project() {
        with_isolated_postgres(|store| async move {
            let email = "postgres-archive-import@finite.vip".to_string();
            let workos_user_id = "workos_postgres_archive_import".to_string();
            let reconciled = store
                .reconcile_existing_host_imports(
                    vec![ExistingHostProjectImport {
                        source_host_id: "legacy-host".to_string(),
                        source_machine_id: "legacy-agent-archive".to_string(),
                        owner_email: Some(email.clone()),
                        display_name: "Imported Agent".to_string(),
                        hostname: None,
                        runtime_host: Some("legacy-host".to_string()),
                        runtime_status: RuntimeSummaryStatus::Online,
                        active_inference_profile: Some("finite-private".to_string()),
                        hermes_available: Some(true),
                        published_app_urls: Vec::new(),
                        known_external_channel_participants: Vec::new(),
                        admin_visible_to_emails: Vec::new(),
                    }],
                    ReconcileExistingHostImportsOptions {
                        allowlisted_owner_emails: vec![email.clone()],
                        now: Some("2026-05-25T12:00:00Z".to_string()),
                    },
                )
                .await
                .unwrap();
            let claimed = store
                .claim_project_imports(ClaimProjectImportsInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    selected_candidate_ids: reconciled.created_candidates,
                    now: Some("2026-05-25T12:01:00Z".to_string()),
                })
                .await
                .unwrap();

            store
                .archive_imported_project(ArchiveImportedProjectInput {
                    verified_email: email,
                    workos_user_id: workos_user_id.clone(),
                    project_id: claimed.claimed_project_ids[0].clone(),
                    now: Some("2026-05-25T12:02:00Z".to_string()),
                })
                .await
                .expect("timestamp text must serialize for Postgres archive");

            assert!(
                store
                    .visible_projects_for_workos_user(&workos_user_id)
                    .await
                    .unwrap()
                    .is_empty()
            );
        })
        .await;
    }

    /// The agent-creation lease queue is partitioned by source host: two requests
    /// routed to different hosts, and a runner declaring host A leases only A's
    /// request — never B's. Proves the global claim across all rows is gone.
    #[tokio::test]
    async fn postgres_agent_creation_lease_partition_by_source_host() {
        with_isolated_postgres(|store| async move {
            let launch_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
            let second_launch_code =
                issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
            let req_a = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "part-a@finite.vip".to_string(),
                    workos_user_id: "workos_part_a".to_string(),
                    display_name: "Partition Agent A".to_string(),
                    launch_code: second_launch_code,
                    idempotency_key: "part-a".to_string(),
                    now: None,
                })
                .await
                .unwrap()
                .request
                .id;
            let req_b = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "part-b@finite.vip".to_string(),
                    workos_user_id: "workos_part_b".to_string(),
                    display_name: "Partition Agent B".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "part-b".to_string(),
                    now: None,
                })
                .await
                .unwrap()
                .request
                .id;

            // Route each request to a specific host (no product path sets this yet,
            // so tag directly — the lease's partition filter is what's under test).
            let (raw, conn) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let conn = tokio::spawn(async move {
                let _ = conn.await;
            });
            raw.execute(
                "UPDATE agent_creation_requests SET target_source_host_id = 'parthosta' WHERE id = $1",
                &[&req_a],
            )
            .await
            .unwrap();
            raw.execute(
                "UPDATE agent_creation_requests SET target_source_host_id = 'parthostb' WHERE id = $1",
                &[&req_b],
            )
            .await
            .unwrap();

            // Host A's runner claims only A's request.
            let leased_a = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-a".to_string(),
                    lease_token: "lease-a".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    source_host_id: Some("parthosta".to_string()),
                    now: None,
                })
                .await
                .unwrap()
                .expect("host A runner should lease A's request");
            assert_eq!(leased_a.request.id, req_a);

            // A's runner has nothing else routable to it (B is host B).
            let leased_a_again = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-a".to_string(),
                    lease_token: "lease-a2".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    source_host_id: Some("parthosta".to_string()),
                    now: None,
                })
                .await
                .unwrap();
            assert!(leased_a_again.is_none(), "must not claim host B's request");

            // Host B's runner claims B's request.
            let leased_b = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-b".to_string(),
                    lease_token: "lease-b".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    source_host_id: Some("parthostb".to_string()),
                    now: None,
                })
                .await
                .unwrap()
                .expect("host B runner should lease B's request");
            assert_eq!(leased_b.request.id, req_b);

            raw.execute("SELECT 1", &[]).await.unwrap();
            drop(raw);
            conn.abort();
        })
        .await;
    }

    /// Centerpiece regression test: the STANDARD-billing (real paying) agent
    /// creation path, end to end, against Postgres. This is the path that
    /// shipped broken — `ensure_standard_agent_creation_entitlement_row` does
    /// `INSERT ... ON CONFLICT (customer_org_id)`, which fails deterministically
    /// unless the table carries a UNIQUE(customer_org_id) constraint. There was
    /// no test on this path, which is the whole reason the bug reached prod.
    ///
    /// It FAILS without the migration's UNIQUE(customer_org_id) constraint (the
    /// create call errors with a 23P01/42P10-class DB error) and PASSES with it.
    #[tokio::test]
    async fn postgres_standard_billing_agent_creation_succeeds() {
        with_isolated_postgres(|store| async move {
            // The database is isolated per test, so fixed identifiers are safe.
            let run = "standard-billing";
            let email = format!("standard-billing-{run}@finite.vip");
            let workos_user_id = format!("workos_standard_billing_{run}");

            // A paid user: link the Stripe customer, then sync an ACTIVE standard
            // subscription. No launch code -> the standard-billing entitlement path.
            // Surrogate ids are minted at insert, so read the org id back from
            // the create call rather than deriving it from the email.
            let org_id = store
                .link_stripe_customer(LinkStripeCustomerInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    stripe_customer_id: format!("cus_standard_{run}"),
                    now: None,
                })
                .await
                .unwrap()
                .customer_org_id;
            store
                .sync_stripe_subscription(SyncStripeSubscriptionInput {
                    customer_org_id: Some(org_id.clone()),
                    stripe_customer_id: format!("cus_standard_{run}"),
                    stripe_subscription_id: format!("sub_standard_{run}"),
                    stripe_price_id: Some("price_standard".to_string()),
                    expected_stripe_price_id: Some("price_standard".to_string()),
                    subscription_status: BillingSubscriptionStatus::Active,
                    current_period_end: Some("2026-08-01T12:00:00Z".to_string()),
                    cancel_at_period_end: false,
                    stripe_event_id: Some(format!("evt_standard_active_{run}")),
                    stripe_event_created: None,
                    now: None,
                })
                .await
                .unwrap();

            // Billing is recognized before any create attempt.
            let overview = store
                .billing_overview(LinkVerifiedUserInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    now: None,
                })
                .await
                .unwrap();
            assert!(overview.can_create_agent);
            assert!(!overview.requires_billing);

            // The create that was broken: no launch code -> standard entitlement
            // upsert via ON CONFLICT (customer_org_id). This is the line under test.
            let created = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    display_name: "Standard Billing Agent".to_string(),
                    launch_code: String::new(),
                    idempotency_key: format!("standard-submit-{run}"),
                    now: None,
                })
                .await
                .expect("standard-billing agent creation must succeed");
            assert!(!created.reused);
            assert_eq!(created.request.requested_launch_code, None);
            assert_eq!(created.request.customer_org_id, org_id);
            assert_eq!(
                created.request.status,
                AgentCreationRequestStatus::Requested
            );

            // Re-submitting the same idempotency key reuses the row (exercises the
            // ON CONFLICT upsert a second time, which is what originally exploded).
            let reused = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    display_name: "Standard Billing Agent".to_string(),
                    launch_code: String::new(),
                    idempotency_key: format!("standard-submit-{run}"),
                    now: None,
                })
                .await
                .expect("idempotent re-submit must succeed");
            assert!(reused.reused);
            assert_eq!(reused.request.id, created.request.id);

            // The entitlement carries no launch code (it is the paid, standard one).
            let overview_after = store
                .billing_overview(LinkVerifiedUserInput {
                    verified_email: email,
                    workos_user_id,
                    now: None,
                })
                .await
                .unwrap();
            assert_eq!(
                overview_after
                    .agent_creation_entitlement
                    .as_ref()
                    .and_then(|entitlement| entitlement.launch_code.as_deref()),
                None
            );
        })
        .await;
    }

    /// A forced constraint violation must surface as a typed, structured
    /// `CoreError::Database` carrying the SQLSTATE code / constraint / table /
    /// DETAIL for the logs, while the user-facing `Display` stays the generic
    /// "database error" — NOT the old bare "db error" that leaked to browsers.
    #[tokio::test]
    async fn postgres_constraint_violation_surfaces_structured_detail() {
        with_isolated_postgres(|store| async move {
            let launch_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
        let run = "constraint-detail";
        let email = format!("constraint-detail-{run}@finite.vip");

        // Materialize one org + one entitlement row via the launch-code path,
        // which needs no Stripe setup.
        let created = store
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: email.clone(),
                workos_user_id: format!("workos_constraint_detail_{run}"),
                display_name: "Constraint Detail Agent".to_string(),
                launch_code: launch_code.clone(),
                idempotency_key: format!("constraint-detail-{run}"),
                now: None,
            })
            .await
            .unwrap();
        let org_id = created.request.customer_org_id;

        // Raw client so we can force a duplicate entitlement for the same org,
        // violating the UNIQUE(customer_org_id) constraint this Phase 0 adds.
        let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let duplicate_id = format!("dup-entitlement-{run}");
        let db_error = raw
            .execute(
                "INSERT INTO agent_creation_entitlements
                   (id, customer_org_id, allowed_new_agent_runtimes, launch_code, created_at, updated_at)
                 VALUES ($1, $2, 1, NULL, now(), now())",
                &[&duplicate_id, &org_id],
            )
            .await
            .expect_err("duplicate customer_org_id must violate the UNIQUE constraint");

        let core_error = store_error(db_error);
        // User-facing surface is generic and safe to show verbatim.
        assert_eq!(core_error.to_string(), "database error");
        match &core_error {
            CoreError::Database(detail) => {
                assert_eq!(detail.code.as_deref(), Some("23505"), "unique_violation");
                assert_eq!(
                    detail.constraint.as_deref(),
                    Some("agent_creation_entitlements_customer_org_id_key"),
                    "the constraint this hotfix adds must be named in the detail"
                );
                assert_eq!(detail.table.as_deref(), Some("agent_creation_entitlements"));
                assert!(
                    detail.detail.is_some(),
                    "Postgres DETAIL line must be preserved for the logs"
                );
                // The whole point: the real message survives, not "db error".
                assert_ne!(detail.message, "db error");
                assert!(!detail.message.is_empty());
            }
            other => panic!("expected CoreError::Database, got {other:?}"),
        }
        })
        .await;
    }

    /// GOLDEN-PATH E2E (per-PR gate). Drives the real STANDARD-billing product
    /// path end to end against real Postgres with a FAKE runner (no Docker /
    /// Phala): link Stripe customer -> sync an ACTIVE standard subscription ->
    /// request_agent_creation (no launch code) -> lease the request (the
    /// runner's claim) -> provision the finite-private key -> register the
    /// runtime -> complete. Then assert the runtime is visible/online and the
    /// creation request is terminal (Running).
    ///
    /// This is the hop-by-hop test that would have caught the 2026-07-04
    /// incident: the standard-billing entitlement upsert, the lease queue, and
    /// the runtime registration all execute against real SQL and constraints.
    /// Phase 2 (surrogate IDs, ordering guard) extends this without rewriting:
    /// the shape is a linear sequence of store calls with assertions between.
    #[tokio::test]
    async fn postgres_golden_path_standard_billing_create_lifecycle() {
        with_isolated_postgres(|store| async move {
            let email = "golden@finite.vip".to_string();
            let workos_user_id = "workos_golden".to_string();
            let stripe_customer_id = "cus_golden".to_string();
            let runner_id = "runner-golden-1".to_string();
            let lease_token = "lease-golden-1".to_string();
            let source_host_id = "golden-host".to_string();
            let source_machine_id = "golden-agent-001".to_string();

            // The runtime image the fake runner will register.
            store
                .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                    id: "artifact-golden-v1".to_string(),
                    kind: RuntimeArtifactKind::OciImage,
                    reference: format!(
                        "ghcr.io/finitecomputer/finite-agent-runtime:golden-v1@sha256:{}",
                        "4".repeat(64)
                    ),
                    version_label: "golden-v1".to_string(),
                    source_git_sha: None,
                    finitec_version: None,
                    hermes_source_ref: None,
                    finite_platform_plugin_ref: None,
                    state_schema_version: "state-v1".to_string(),
                    base_image: Some("python:3.11-trixie".to_string()),
                    recover_known_good_chat: false,
                    promoted: true,
                    now: None,
                })
                .await
                .unwrap();

            // 1. Link the Stripe customer and sync an ACTIVE standard sub. The
            // org id is a surrogate minted at insert, so read it back from the
            // create call instead of deriving it from the email.
            let org_id = store
                .link_stripe_customer(LinkStripeCustomerInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    stripe_customer_id: stripe_customer_id.clone(),
                    now: None,
                })
                .await
                .unwrap()
                .customer_org_id;
            store
                .sync_stripe_subscription(SyncStripeSubscriptionInput {
                    customer_org_id: Some(org_id.clone()),
                    stripe_customer_id: stripe_customer_id.clone(),
                    stripe_subscription_id: "sub_golden".to_string(),
                    stripe_price_id: Some("price_standard".to_string()),
                    expected_stripe_price_id: Some("price_standard".to_string()),
                    subscription_status: BillingSubscriptionStatus::Active,
                    current_period_end: Some("2026-08-01T12:00:00Z".to_string()),
                    cancel_at_period_end: false,
                    stripe_event_id: Some("evt_golden_active".to_string()),
                    stripe_event_created: None,
                    now: None,
                })
                .await
                .unwrap();

            // Billing recognizes the paid user before any create attempt.
            let overview = store
                .billing_overview(LinkVerifiedUserInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    now: None,
                })
                .await
                .unwrap();
            assert!(overview.can_create_agent, "active standard sub can create");
            assert!(!overview.requires_billing);

            // 2. request_agent_creation with NO launch code (the paid path).
            let created = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    display_name: "Golden Agent".to_string(),
                    launch_code: String::new(),
                    idempotency_key: "golden-submit".to_string(),
                    now: None,
                })
                .await
                .expect("standard-billing create must succeed");
            assert_eq!(
                created.request.status,
                AgentCreationRequestStatus::Requested
            );
            assert_eq!(created.request.customer_org_id, org_id);

            // 3. The runner leases the pending creation request.
            let lease = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: runner_id.clone(),
                    source_host_id: None,
                    lease_token: lease_token.clone(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: None,
                })
                .await
                .unwrap()
                .expect("the pending request must be leasable");
            assert_eq!(lease.request.id, created.request.id);
            assert_eq!(lease.request.status, AgentCreationRequestStatus::Launching);

            // The project is visible but has no runtime yet.
            let visible_before = store
                .visible_projects_for_workos_user(&workos_user_id)
                .await
                .unwrap();
            assert_eq!(visible_before.len(), 1);
            assert!(visible_before[0].runtime.is_none());

            // 4. Provision the finite-private key + register the runtime.
            store
                .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                    request_id: lease.request.id.clone(),
                    runner_id: runner_id.clone(),
                    lease_token: lease_token.clone(),
                    source_host_id: Some(source_host_id.clone()),
                    source_machine_id: Some(source_machine_id.clone()),
                    now: None,
                })
                .await
                .unwrap();
            let runtime_token = "runtime-golden-token";
            store
                .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                    request_id: lease.request.id.clone(),
                    runner_id: runner_id.clone(),
                    lease_token: lease_token.clone(),
                    source_host_id: source_host_id.clone(),
                    source_machine_id: source_machine_id.clone(),
                    runtime_artifact_id: Some("artifact-golden-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: None,
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    runtime_relay_token_hash: runtime_relay_token_hash(runtime_token).unwrap(),
                    display_name: Some("Golden Agent".to_string()),
                    hostname: None,
                    runtime_host: Some(source_host_id.clone()),
                    runtime_status: Some(RuntimeSummaryStatus::Unknown),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: None,
                })
                .await
                .unwrap();

            // 5. Complete the creation.
            let completed = store
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: lease.request.id.clone(),
                    runner_id: runner_id.clone(),
                    lease_token: lease_token.clone(),
                    source_host_id: source_host_id.clone(),
                    source_machine_id: source_machine_id.clone(),
                    runtime_artifact_id: Some("artifact-golden-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: None,
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("Golden Agent".to_string()),
                    hostname: None,
                    runtime_host: Some(source_host_id.clone()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: None,
                })
                .await
                .unwrap();

            // The request is terminal (Running) ...
            assert_eq!(
                completed.request.status,
                AgentCreationRequestStatus::Running,
                "completed creation request must be terminal"
            );
            let requests = store
                .agent_creation_requests_for_workos_user(&workos_user_id)
                .await
                .unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].status, AgentCreationRequestStatus::Running);

            // ... and the runtime is visible and online.
            let visible_after = store
                .visible_projects_for_workos_user(&workos_user_id)
                .await
                .unwrap();
            assert_eq!(visible_after.len(), 1);
            let runtime = visible_after[0]
                .runtime
                .as_ref()
                .expect("completed project must expose a runtime");
            assert_eq!(runtime.source_machine_id, source_machine_id);

            // A second lease call finds nothing else pending: the queue drained.
            let empty = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: runner_id.clone(),
                    source_host_id: None,
                    lease_token: "lease-golden-2".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: None,
                })
                .await
                .unwrap();
            assert!(empty.is_none(), "no further pending requests to lease");
        })
        .await;
    }

    /// SURROGATE-ID REGRESSION (Phase 2a): wipe an account, then re-signup with
    /// the SAME email. Primary keys are now opaque surrogates minted at insert
    /// (`user_id`/`org_id`/`request_id` are random, resolved by natural key),
    /// so a clean full wipe followed by re-signup yields entirely FRESH ids that
    /// cannot collide with the previous account's orphaned rows. This is the
    /// flipped version of the old deterministic-id baseline: the point of the
    /// incident fix is that re-created identities do NOT reconstruct old keys.
    #[tokio::test]
    async fn postgres_wipe_then_recreate_same_email_gets_fresh_surrogate_ids() {
        with_isolated_postgres(|store| async move {
            let launch_code = issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;
            let email = "wipe-recreate@finite.vip".to_string();
            let workos_user_id = "workos_wipe_recreate".to_string();

            let first = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    display_name: "Wipe Recreate Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: "wipe-recreate-1".to_string(),
                    now: None,
                })
                .await
                .unwrap();
            // Read the minted surrogate ids back from the store — they are not
            // derivable from the email any more.
            let first_user_id = first.request.owner_user_id.clone();
            let first_org_id = first.request.customer_org_id.clone();
            let first_request_id = first.request.id.clone();

            // Full wipe via raw SQL. `TRUNCATE ... CASCADE` on the account root
            // tables removes every FK-dependent row (projects, requests,
            // entitlements, chat identities, memberships, ...) in one clean
            // sweep — this is the "clean" wipe; the incident was the *partial*
            // version that left orphans behind the same deterministic ids.
            let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });
            raw.batch_execute("TRUNCATE TABLE users CASCADE")
                .await
                .expect("clean full wipe should not violate FKs");
            drop(raw);
            connection.abort();

            let replacement_launch_code =
                issue_test_launch_code(&store, "2026-05-25T12:00:00Z").await;

            // Re-signup with the same email. A clean wipe means this succeeds,
            // and — because ids are now surrogate — mints a genuinely fresh
            // user/org/request that share NOTHING with the wiped account.
            let second = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    display_name: "Wipe Recreate Agent".to_string(),
                    launch_code: replacement_launch_code,
                    idempotency_key: "wipe-recreate-2".to_string(),
                    now: None,
                })
                .await
                .expect("re-signup after a clean wipe must succeed");
            assert_ne!(
                second.request.owner_user_id, first_user_id,
                "surrogate ids: the re-created user must get a fresh id"
            );
            assert_ne!(
                second.request.customer_org_id, first_org_id,
                "surrogate ids: the re-created org must get a fresh id"
            );
            assert_ne!(
                second.request.id, first_request_id,
                "surrogate ids: the re-created request must get a fresh id"
            );
        })
        .await;
    }

    /// Phase 2b event-ordering guard (audit finding #5): out-of-order Stripe
    /// webhooks for the SAME subscription. `sync_stripe_subscription` now compares
    /// the incoming `event.created` against the last applied one and IGNORES a
    /// stale event, so an `active` delivered AFTER a `canceled` can no longer
    /// resurrect billing. This is the flipped former baseline.
    #[tokio::test]
    async fn postgres_out_of_order_webhook_is_ignored() {
        with_isolated_postgres(|store| async move {
            let email = "webhook-order@finite.vip".to_string();
            let workos_user_id = "workos_webhook_order".to_string();
            let stripe_customer_id = "cus_webhook_order".to_string();
            let stripe_subscription_id = "sub_webhook_order".to_string();

            let org_id = store
                .link_stripe_customer(LinkStripeCustomerInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    stripe_customer_id: stripe_customer_id.clone(),
                    now: None,
                })
                .await
                .unwrap()
                .customer_org_id;

            let sync = |status: BillingSubscriptionStatus, event: &str, created: i64| {
                store.sync_stripe_subscription(SyncStripeSubscriptionInput {
                    customer_org_id: Some(org_id.clone()),
                    stripe_customer_id: stripe_customer_id.clone(),
                    stripe_subscription_id: stripe_subscription_id.clone(),
                    stripe_price_id: Some("price_standard".to_string()),
                    expected_stripe_price_id: Some("price_standard".to_string()),
                    subscription_status: status,
                    current_period_end: Some("2026-08-01T12:00:00Z".to_string()),
                    cancel_at_period_end: false,
                    stripe_event_id: Some(event.to_string()),
                    stripe_event_created: Some(created),
                    now: None,
                })
            };

            // Real order: active (created t0), then canceled (created t1 > t0).
            sync(BillingSubscriptionStatus::Active, "evt_active", 1_000)
                .await
                .unwrap();
            let canceled = sync(BillingSubscriptionStatus::Canceled, "evt_canceled", 2_000)
                .await
                .unwrap();
            assert_eq!(
                canceled.subscription_status,
                Some(BillingSubscriptionStatus::Canceled)
            );

            // A STALE `active` event (created BEFORE the canceled event) arrives LAST.
            let stale = sync(BillingSubscriptionStatus::Active, "evt_active_stale", 1_500)
                .await
                .unwrap();

            // The guard drops the stale event; billing stays canceled.
            assert_eq!(
                stale.subscription_status,
                Some(BillingSubscriptionStatus::Canceled),
                "stale out-of-order webhook must be ignored; billing stays canceled"
            );
            assert_eq!(stale.last_stripe_event_id.as_deref(), Some("evt_canceled"));
            let overview = store
                .billing_overview(LinkVerifiedUserInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    now: None,
                })
                .await
                .unwrap();
            assert!(
                !overview.can_create_agent,
                "canceled subscription must not re-grant create after a stale webhook"
            );
        })
        .await;
    }

    /// `billing_overview` is a READ: it must perform NO writes. We run it inside a
    /// genuinely read-only transaction and additionally assert the billing row's
    /// `updated_at` is byte-for-byte unchanged across the call.
    #[tokio::test]
    async fn postgres_billing_overview_performs_no_writes() {
        with_isolated_postgres(|store| async move {
            let email = "read-only@finite.vip".to_string();
            let workos_user_id = "workos_read_only".to_string();
            let stripe_customer_id = "cus_read_only".to_string();

            let org_id = store
                .link_stripe_customer(LinkStripeCustomerInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    stripe_customer_id: stripe_customer_id.clone(),
                    now: None,
                })
                .await
                .unwrap()
                .customer_org_id;
            store
                .sync_stripe_subscription(SyncStripeSubscriptionInput {
                    customer_org_id: Some(org_id.clone()),
                    stripe_customer_id: stripe_customer_id.clone(),
                    stripe_subscription_id: "sub_read_only".to_string(),
                    stripe_price_id: Some("price_standard".to_string()),
                    expected_stripe_price_id: Some("price_standard".to_string()),
                    subscription_status: BillingSubscriptionStatus::Active,
                    current_period_end: Some("2026-08-01T12:00:00Z".to_string()),
                    cancel_at_period_end: false,
                    stripe_event_id: Some("evt_read_only_active".to_string()),
                    stripe_event_created: Some(1_000),
                    now: None,
                })
                .await
                .unwrap();

            // Snapshot every row's updated_at (as text) across all billing-related
            // tables the overview touches.
            let (raw, raw_conn) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let raw_conn = tokio::spawn(async move {
                let _ = raw_conn.await;
            });
            async fn snapshot(raw: &tokio_postgres::Client) -> Vec<(String, String)> {
                let mut out: Vec<(String, String)> = Vec::new();
                for table in [
                    "customer_orgs",
                    "customer_billing_accounts",
                    "agent_creation_entitlements",
                    "users",
                ] {
                    let key = if table == "customer_billing_accounts" {
                        "customer_org_id"
                    } else {
                        "id"
                    };
                    for row in raw
                        .query(
                            &format!(
                                "SELECT {key}::text, updated_at::text FROM {table} ORDER BY 1"
                            ),
                            &[],
                        )
                        .await
                        .unwrap()
                    {
                        out.push((format!("{table}:{}", row.get::<_, String>(0)), row.get(1)));
                    }
                }
                out
            }

            let before = snapshot(&raw).await;

            // Read-only op: if it tried to write, the READ ONLY transaction would
            // error; assert it succeeds AND leaves every updated_at unchanged.
            let overview = store
                .billing_overview(LinkVerifiedUserInput {
                    verified_email: email.clone(),
                    workos_user_id: workos_user_id.clone(),
                    now: None,
                })
                .await
                .unwrap();
            assert!(overview.can_create_agent);
            assert!(!overview.requires_billing);

            let after = snapshot(&raw).await;
            assert_eq!(
                before, after,
                "billing_overview must not mutate any row (a read that writes is banned)"
            );

            drop(raw);
            raw_conn.abort();
        })
        .await;
    }
}
