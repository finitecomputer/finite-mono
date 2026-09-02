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
    RevokeFinitePrivateGrantInput, RotateFinitePrivateApiKeyInput, RunnerClass, RuntimeArtifact,
    RuntimeBootIntent, RuntimeCapabilitiesEnvelope, RuntimeControlCompletion,
    RuntimeControlExpectedBinding, RuntimeControlKind, RuntimeControlLease, RuntimeControlRequest,
    RuntimeControlRequestStatus, RuntimeHealthReportAck, RuntimeLifecycleStage, RuntimePlacement,
    RuntimeRelocationEnvelope, RuntimeRelocationV1, RuntimeRetirementSnapshot,
    RuntimeRetirementSnapshotReceipt, RuntimeSpecEnvelope, RuntimeSpecIdentity,
    RuntimeSummaryStatus, SettleFinitePrivateReservationInput,
    SettleFinitePrivateReservationResult, StoreErrorDetail, StoredRuntimeHealth,
    SyncStripeSubscriptionInput, UnrecoverableRuntimeArchiveReceipt, UpsertRuntimeArtifactInput,
    agent_creation_entitlement_id_for, append_provider_operation_transition,
    bound_runtime_capabilities_to_artifact, build_runtime_spec_v1, canonical_agent_email,
    chat_identity_id_for_user, current_time_iso, finite_private_api_key_id_for,
    finite_private_grant_id_for_user, generate_finite_private_api_key, hash_finite_private_api_key,
    merge_provider_runtime_handle, merge_runtime_capabilities, new_agent_creation_request_id,
    new_agent_runtime_id, new_customer_org_id, new_self_service_project_id, new_user_id,
    normalize_id_part, normalize_idempotency_key, normalize_owner_chat_account_id,
    normalize_owner_email, normalize_profile_picture_url, normalize_runtime_contact_endpoint,
    normalize_source_host_id, parse_agent_creation_request_status, parse_billing_class,
    parse_finite_private_api_key_status, parse_finite_private_grant_status,
    parse_finite_private_reservation_status, parse_hosting_tier, parse_offboarding_phase,
    parse_runner_class, parse_runtime_artifact_kind, parse_runtime_control_kind,
    parse_runtime_control_request_status, parse_runtime_lifecycle_stage,
    parse_runtime_resource_class, parse_time, parse_user_link_status,
    project_room_membership_id_for, project_runtime_health, project_runtime_link_id_for,
    provider_operation_allows_generic_failure, provider_operation_at_runtime_boundary,
    runtime_artifact_material_matches, runtime_artifact_reference_is_immutable_oci,
    runtime_lifecycle, runtime_operation_spec_v1, runtime_spec_secret_references, runtime_spec_v1,
    runtime_upgrade_contact_endpoint, runtime_upgrade_prelease_rejection_is_terminal,
    source_import_key, trim_to_option, valid_agent_npub, valid_sha256_hex,
    validate_runtime_capabilities_artifact_policy, validate_runtime_capabilities_policy,
    validate_runtime_relocation_registration, validate_runtime_retirement_snapshot_receipt,
    validate_runtime_spec_binding, validate_runtime_spec_environment,
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
    pub active_runtime_control: Option<RuntimeControlRequest>,
}

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

    pub async fn issue_launch_code_batch(
        &self,
        input: IssueLaunchCodeBatchInput,
    ) -> CoreResult<IssuedLaunchCodeBatch> {
        let prepared = prepare_launch_code_batch(input)?;
        let response = IssuedLaunchCodeBatch {
            batch: prepared.batch.clone(),
            codes: prepared.issued_codes,
        };
        let mut client = self.connection().await?;
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
        self.finish(tx).await?;
        Ok(response)
    }

    pub async fn list_launch_code_batches(&self) -> CoreResult<Vec<LaunchCodeBatchDetails>> {
        let client = self.connection().await?;
        postgres_list_launch_code_batches(&**client).await
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
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let row = tx
            .query_opt(
                "UPDATE launch_code_batches
                    SET revoked_at = COALESCE(revoked_at, $2::text::timestamptz),
                        revoked_by_workos_user_id = COALESCE(revoked_by_workos_user_id, $3)
                  WHERE id = $1
                  RETURNING id, name, hosting_tier, code_count, core_rfc3339(expires_at) AS expires_at,
                            core_rfc3339(revoked_at) AS revoked_at, revoked_by_workos_user_id,
                            created_by_workos_user_id, core_rfc3339(created_at) AS created_at",
                &[&input.batch_id.trim(), &now, &actor],
            )
            .await
            .map_err(store_error)?
            .ok_or(CoreError::LaunchCodeBatchNotFound)?;
        let batch = launch_code_batch_from_row(&row)?;
        let details = postgres_launch_code_batch_details(&*tx, batch).await?;
        self.finish(tx).await?;
        Ok(details)
    }

    pub async fn admin_request_runtime_upgrade(
        &self,
        input: AdminRuntimeUpgradeInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_request_runtime_upgrade(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_request_runtime_upgrade_exact(
        &self,
        input: AdminRuntimeUpgradeExactInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_request_runtime_upgrade_exact(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_request_runtime_retire_exact(
        &self,
        input: AdminRuntimeRetireExactInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_request_runtime_retire_exact(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_request_runtime_relocate_exact(
        &self,
        input: AdminRuntimeRelocateExactInput,
    ) -> CoreResult<AgentCreationRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_request_runtime_relocate_exact(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn runtime_control_request(
        &self,
        request_id: &str,
    ) -> CoreResult<RuntimeControlRequest> {
        let client = self.connection().await?;
        postgres_runtime_control_request(&**client, request_id).await
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
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_request_agent_creation(&*tx, input, configuration).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn request_runtime_restart(
        &self,
        input: RequestRuntimeRestartInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_request_runtime_control(&*tx, input, RuntimeControlKind::Restart).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn request_runtime_recover_known_good_chat(
        &self,
        input: RequestRuntimeRecoverKnownGoodChatInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_request_runtime_control(
            &*tx,
            input,
            RuntimeControlKind::RecoverKnownGoodChatRuntime,
        )
        .await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn request_runtime_stop(
        &self,
        input: RequestRuntimeStopInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_request_runtime_control(&*tx, input, RuntimeControlKind::Stop).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn request_runtime_destroy(
        &self,
        input: RequestRuntimeDestroyInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_request_runtime_control(&*tx, input, RuntimeControlKind::Destroy).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn link_verified_user(&self, input: LinkVerifiedUserInput) -> CoreResult<CoreUser> {
        let now = input.now.unwrap_or(current_time_iso()?);
        let verified_email = normalize_owner_email(Some(&input.verified_email))
            .ok_or(CoreError::MissingVerifiedEmail)?;
        let workos_user_id = input.workos_user_id.trim().to_string();
        if workos_user_id.is_empty() {
            return Err(CoreError::MissingWorkosUserId);
        }
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let user = ensure_linked_user_row(
            &*tx,
            &verified_email,
            &workos_user_id,
            BillingClass::Standard,
            &now,
        )
        .await?;
        self.finish(tx).await?;
        Ok(user)
    }

    pub async fn billing_overview(
        &self,
        input: LinkVerifiedUserInput,
    ) -> CoreResult<BillingOverview> {
        // Read-only: no global lock, no full-state rewrite, no writes at all.
        // A read that wrote the whole DB was anti-pattern #3; this is targeted
        // SELECTs inside a READ ONLY transaction.
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        tx.execute("SET TRANSACTION READ ONLY", &[])
            .await
            .map_err(store_error)?;
        let overview = postgres_billing_overview(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(overview)
    }

    pub async fn link_stripe_customer(
        &self,
        input: LinkStripeCustomerInput,
    ) -> CoreResult<CustomerBillingAccount> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let account = billing::link_stripe_customer(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(account)
    }

    pub async fn sync_stripe_subscription(
        &self,
        input: SyncStripeSubscriptionInput,
    ) -> CoreResult<CustomerBillingAccount> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let account = billing::sync_stripe_subscription(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(account)
    }

    pub async fn lease_agent_creation_request(
        &self,
        input: LeaseAgentCreationRequestInput,
    ) -> CoreResult<Option<AgentCreationLease>> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_lease_agent_creation_request(
            &*tx,
            input,
            self.runtime_environment.as_ref(),
            self.runtime_secret_references.as_ref(),
        )
        .await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn record_provider_operation_transition(
        &self,
        input: RecordProviderOperationTransitionInput,
    ) -> CoreResult<ProviderOperationEnvelope> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_record_provider_operation_transition(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn lease_runtime_control_request(
        &self,
        input: LeaseRuntimeControlRequestInput,
    ) -> CoreResult<Option<RuntimeControlLease>> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_lease_runtime_control_request(
            &*tx,
            input,
            self.runtime_environment.as_ref(),
            self.runtime_secret_references.as_ref(),
        )
        .await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn complete_runtime_control_request(
        &self,
        input: CompleteRuntimeControlRequestInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_complete_runtime_control_request(
            &*tx,
            input,
            self.runtime_environment.as_ref(),
            self.runtime_secret_references.as_ref(),
        )
        .await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn fail_runtime_control_request(
        &self,
        input: FailRuntimeControlRequestInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_fail_runtime_control_request(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn renew_runtime_control_request(
        &self,
        input: RenewRuntimeControlRequestInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_renew_runtime_control_request(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn retry_runtime_control_request(
        &self,
        input: RetryRuntimeControlRequestInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_retry_runtime_control_request(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn complete_agent_creation_request(
        &self,
        input: CompleteAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationLease> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_complete_agent_creation_request(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn register_agent_creation_runtime(
        &self,
        input: RegisterAgentCreationRuntimeInput,
    ) -> CoreResult<AgentCreationLease> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_register_agent_creation_runtime(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn fail_agent_creation_request(
        &self,
        input: FailAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_fail_agent_creation_request(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn cancel_agent_creation_request(
        &self,
        input: CancelAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_cancel_agent_creation_request(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn visible_projects_for_workos_user(
        &self,
        workos_user_id: &str,
    ) -> CoreResult<Vec<VisibleProject>> {
        let client = self.connection().await?;
        postgres_visible_projects_for_workos_user(&**client, workos_user_id).await
    }

    pub async fn agent_creation_requests_for_workos_user(
        &self,
        workos_user_id: &str,
    ) -> CoreResult<Vec<AgentCreationRequest>> {
        let client = self.connection().await?;
        postgres_agent_creation_requests_for_workos_user(&**client, workos_user_id).await
    }

    pub async fn runtime_artifact(&self, id: &str) -> CoreResult<Option<RuntimeArtifact>> {
        let id = trim_to_option(Some(id)).ok_or(CoreError::MissingRuntimeArtifactId)?;
        let client = self.connection().await?;
        select_runtime_artifact(&**client, &id).await
    }

    pub async fn upsert_runtime_artifact(
        &self,
        input: UpsertRuntimeArtifactInput,
    ) -> CoreResult<RuntimeArtifact> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let artifact = postgres_upsert_runtime_artifact(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(artifact)
    }

    pub async fn approve_finite_private_grant(
        &self,
        input: ApproveFinitePrivateGrantInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let grant = postgres_approve_finite_private_grant(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(grant)
    }

    pub async fn issue_finite_private_api_key(
        &self,
        input: IssueFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let key = postgres_issue_finite_private_api_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(key)
    }

    pub async fn provision_finite_private_runtime_key(
        &self,
        input: ProvisionFinitePrivateRuntimeKeyInput,
    ) -> CoreResult<ProvisionFinitePrivateRuntimeKeyResult> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_provision_finite_private_runtime_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn revoke_finite_private_grant(
        &self,
        input: RevokeFinitePrivateGrantInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let grant = postgres_revoke_finite_private_grant(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(grant)
    }

    pub async fn revoke_finite_private_api_key(
        &self,
        input: RevokeFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let key = postgres_revoke_finite_private_api_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(key)
    }

    pub async fn rotate_finite_private_api_key(
        &self,
        input: RotateFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let key = postgres_rotate_finite_private_api_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(key)
    }

    pub async fn reset_finite_private_usage_window(
        &self,
        input: ResetFinitePrivateUsageWindowInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let grant = postgres_reset_finite_private_usage_window(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(grant)
    }

    pub async fn admin_runtime_overviews(&self) -> CoreResult<Vec<AdminRuntimeOverview>> {
        let client = self.connection().await?;
        postgres_admin_runtime_overviews(&**client).await
    }

    pub async fn record_runtime_health_report(
        &self,
        input: RecordRuntimeHealthReportInput,
    ) -> CoreResult<RuntimeHealthReportAck> {
        let client = self.connection().await?;
        postgres_record_runtime_health_report(&**client, input).await
    }

    pub async fn admin_archive_unrecoverable_runtime(
        &self,
        input: AdminArchiveUnrecoverableRuntimeInput,
    ) -> CoreResult<UnrecoverableRuntimeArchiveReceipt> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let receipt = postgres_admin_archive_unrecoverable_runtime(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(receipt)
    }

    pub async fn admin_offboard_retired_runtime(
        &self,
        input: AdminOffboardRetiredRuntimeInput,
    ) -> CoreResult<RetiredRuntimeOffboardReceipt> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let receipt = postgres_admin_offboard_retired_runtime(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(receipt)
    }

    pub async fn admin_request_runtime_restart(
        &self,
        input: AdminRuntimeControlInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_admin_request_runtime_control(&*tx, input, RuntimeControlKind::Restart, None)
                .await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_request_runtime_recover_known_good_chat(
        &self,
        input: AdminRuntimeControlInput,
    ) -> CoreResult<RuntimeControlRequest> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_request_runtime_control(
            &*tx,
            input,
            RuntimeControlKind::RecoverKnownGoodChatRuntime,
            None,
        )
        .await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_issue_finite_private_friend_key(
        &self,
        input: AdminIssueFinitePrivateFriendKeyInput,
    ) -> CoreResult<AdminIssuedFinitePrivateKey> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_issue_finite_private_friend_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn issue_finite_private_friend_key(
        &self,
        input: IssueFinitePrivateFriendKeyInput,
    ) -> CoreResult<IssuedFinitePrivateFriendKey> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_issue_finite_private_friend_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_rotate_finite_private_api_key(
        &self,
        input: AdminRotateFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_rotate_finite_private_api_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_revoke_finite_private_api_key(
        &self,
        input: AdminRevokeFinitePrivateApiKeyInput,
    ) -> CoreResult<FinitePrivateApiKey> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_revoke_finite_private_api_key(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_reset_finite_private_usage_window(
        &self,
        input: AdminResetFinitePrivateUsageWindowInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_reset_finite_private_usage_window(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn admin_assign_finite_private_limit_profile(
        &self,
        input: AdminAssignFinitePrivateLimitProfileInput,
    ) -> CoreResult<FinitePrivateGrant> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_admin_assign_finite_private_limit_profile(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn finite_private_admin_audit_events(
        &self,
    ) -> CoreResult<Vec<FinitePrivateAdminAuditEvent>> {
        let client = self.connection().await?;
        postgres_finite_private_admin_audit_events(&**client).await
    }

    pub async fn finite_private_admin_state(&self) -> CoreResult<FinitePrivateAdminState> {
        let client = self.connection().await?;
        postgres_finite_private_admin_state(&**client).await
    }

    pub async fn reserve_finite_private_usage(
        &self,
        input: ReserveFinitePrivateUsageInput,
    ) -> CoreResult<FinitePrivateUsageDecision> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let decision = postgres_reserve_finite_private_usage(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(decision)
    }

    pub async fn settle_finite_private_reservation(
        &self,
        input: SettleFinitePrivateReservationInput,
    ) -> CoreResult<SettleFinitePrivateReservationResult> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_settle_finite_private_reservation(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn finite_private_usage_status_for_api_key(
        &self,
        presented_api_key: &str,
        claim_notice: bool,
        now: Option<String>,
    ) -> CoreResult<Option<FinitePrivateUsageStatus>> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_finite_private_usage_status_for_api_key(
            &*tx,
            presented_api_key,
            claim_notice,
            now,
        )
        .await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn finite_private_usage_status_for_workos_user(
        &self,
        workos_user_id: &str,
        now: Option<String>,
    ) -> CoreResult<Option<FinitePrivateUsageStatus>> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_finite_private_usage_status_for_workos_user(&*tx, workos_user_id, now).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn claim_finite_private_daily_reset_for_api_key(
        &self,
        presented_api_key: &str,
        now: Option<String>,
    ) -> CoreResult<FinitePrivateDailyResetResult> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_claim_finite_private_daily_reset_for_api_key(&*tx, presented_api_key, now)
                .await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn claim_finite_private_daily_reset_for_workos_user(
        &self,
        workos_user_id: &str,
        now: Option<String>,
    ) -> CoreResult<Option<FinitePrivateDailyResetResult>> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result =
            postgres_claim_finite_private_daily_reset_for_workos_user(&*tx, workos_user_id, now)
                .await?;
        self.finish(tx).await?;
        Ok(result)
    }
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
    let owner_chat_account_id =
        normalize_owner_chat_account_id(configuration.owner_chat_account_id.as_deref())?;
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
        Some(org_id) => billing::customer_org_has_active_billing(client, org_id).await?,
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
        billing::select_customer_billing_account(client, org_id, false)
            .await?
            .and_then(|account| account.hosting_tier)
            .ok_or(CoreError::MissingHostingTier)?
    };
    if configuration
        .requested_hosting_tier
        .is_some_and(|requested| requested != hosting_tier)
    {
        return Err(CoreError::HostingTierNotAuthorized);
    }
    // Fail-closed placement for the confidential lane: no deployed runner can
    // advertise `phala`, so no new AgentCreationRequest may carry a Phala
    // placement — not from the tier on the Launch Code/billing account, and
    // not from an operator-configured `FC_CORE_AGENT_CREATION_PLACEMENT_JSON`.
    // This rejects before any row is written, so a pre-existing unredeemed
    // confidential code is left untouched for the lane's eventual return.
    // Legacy confidential rows keep parsing; they just cannot mint new work.
    let placement = configuration
        .placement
        .or_else(|| RuntimePlacement::for_hosting_tier(hosting_tier))
        .filter(|placement| placement.runner_class != RunnerClass::Phala)
        .ok_or(CoreError::HostingTierUnavailable)?;
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
        agent_email: Some(canonical_agent_email(&display_name, &project_id)),
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
        target_source_host_id: None,
        relocation: None,
        profile_picture_url,
        owner_chat_account_id,
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
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
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

    let billing_account = billing::select_customer_billing_account(client, &org.id, false).await?;
    let agent_creation_entitlement =
        select_agent_creation_entitlement_by_org(client, &org.id).await?;
    let has_active_billing = billing::customer_org_has_active_billing(client, &org.id).await?;
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

pub(crate) async fn customer_org_exists<C>(client: &C, org_id: &str) -> CoreResult<bool>
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
    runtime_secret_references: &[String],
) -> CoreResult<Option<AgentCreationLease>>
where
    C: GenericClient + Sync,
{
    validate_runtime_spec_environment(runtime_environment)?;
    let runtime_secret_references = runtime_spec_secret_references(runtime_secret_references)?;
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
                        (status = 'requested' AND TRUE)
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
                        relocation_spec IS NULL
                        OR ($5::text IS NOT NULL AND target_source_host_id = $5)
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
                       request.desired_runtime_artifact_id, request.runtime_spec, request.target_source_host_id, request.relocation_spec,
                       request.profile_picture_url,
                       request.owner_chat_account_id,
                       request.status, request.requested_launch_code, request.agent_runtime_id,
                       request.runner_id, request.lease_token, core_rfc3339(request.lease_expires_at) AS lease_expires_at,
                       request.failure_message, core_rfc3339(request.created_at) AS created_at, core_rfc3339(request.updated_at) AS updated_at",
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
        // The owner chat identity is per-request state, so it joins the
        // Core-global environment only here, at spec-build time. A request
        // without it keeps the exact legacy environment (allow-all chat
        // admission owned by the runtime image).
        let mut environment = runtime_environment.clone();
        if let Some(owner_chat_account_id) = request.owner_chat_account_id.as_deref() {
            environment.insert(
                OWNER_CHAT_NPUBS_ENV.to_string(),
                owner_chat_account_id.to_string(),
            );
        }
        let runtime_spec = build_runtime_spec_v1(
            RuntimeSpecIdentity {
                operation_id: &request.id,
                project_id: &request.project_id,
                agent_runtime_id: &runtime_id,
                placement,
            },
            &artifact,
            &runtime_id,
            environment,
            runtime_secret_references,
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
    let runtime_by_id = select_agent_runtime(client, &runtime_id).await?;
    validate_runtime_relocation_registration(
        &request,
        runtime_by_id.as_ref(),
        &source_host_id,
        &source_machine_id,
    )?;
    if request.relocation.is_some() {
        // Relocation registration is deliberately non-mutating. Completion
        // below is the single transaction that replaces the source binding.
        return Ok(AgentCreationLease {
            project,
            request,
            provider_operation,
        });
    }
    let existing_runtime = match runtime_by_source {
        Some(runtime) => Some(runtime),
        None => runtime_by_id.filter(|runtime| runtime.source_import_key == source_import_key),
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
    validate_runtime_relocation_registration(
        &request,
        existing_runtime.as_ref(),
        &source_host_id,
        &source_machine_id,
    )?;
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
    if let Some(relocation) = request
        .relocation
        .as_ref()
        .map(RuntimeRelocationEnvelope::v1)
    {
        let updated = client
            .execute(
                "UPDATE agent_runtimes
                 SET source_host_id = $2,
                     source_machine_id = $3,
                     source_import_key = $4
                 WHERE id = $1
                   AND source_machine_id = $3
                   AND source_host_id IN ($5, $2)",
                &[
                    &runtime.id,
                    &runtime.source_host_id,
                    &runtime.source_machine_id,
                    &runtime.source_import_key,
                    &relocation.source_host_id,
                ],
            )
            .await
            .map_err(store_error)?;
        if updated != 1 {
            return Err(CoreError::RuntimeSpecMismatch);
        }
    }
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
    let is_relocation = request.relocation.is_some();
    if let Some(key_id) = input.provisioned_finite_private_api_key_id.as_deref() {
        let key_id = trim_to_option(Some(key_id)).ok_or(CoreError::InvalidFinitePrivateApiKey)?;
        let key_row = client
            .query_opt(
                "SELECT id, grant_id, project_id, agent_runtime_id, key_hash, status,
                        core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
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
    if !is_relocation && let Some(runtime_id) = request.agent_runtime_id.as_deref() {
        delete_runtime_rows(client, runtime_id).await?;
    }
    let agent_runtime_id = if is_relocation {
        request.agent_runtime_id.clone()
    } else {
        None
    };
    let row = client
        .query_one(
            "UPDATE agent_creation_requests
             SET status = 'failed',
                 agent_runtime_id = $4,
                 lease_token = NULL,
                 lease_expires_at = NULL,
                 failure_message = $2,
                 updated_at = $3::text::timestamptz
             WHERE id = $1
             RETURNING id, customer_org_id, owner_user_id, project_id, idempotency_key,
                       display_name, runner_class, hosting_tier, placement_runner_class,
                       runtime_resource_class, desired_runtime_artifact_id, runtime_spec, target_source_host_id, relocation_spec,
                       profile_picture_url,
                       owner_chat_account_id,
                       status, requested_launch_code, agent_runtime_id,
                       runner_id, lease_token, core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
            &[&input.request_id, &failure_message, &now, &agent_runtime_id],
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
    let is_relocation = request.relocation.is_some();
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
    if !is_relocation {
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
    }
    if !is_relocation && let Some(runtime_id) = request.agent_runtime_id.as_deref() {
        delete_runtime_rows(client, runtime_id).await?;
    }
    let agent_runtime_id = if is_relocation {
        request.agent_runtime_id.clone()
    } else {
        None
    };
    let row = client
        .query_one(
            "UPDATE agent_creation_requests
             SET status = 'cancelled',
                 agent_runtime_id = $2,
                 runner_id = NULL,
                 lease_token = NULL,
                 lease_expires_at = NULL,
                 failure_message = NULL,
                 updated_at = $3::text::timestamptz
             WHERE id = $1
             RETURNING id, customer_org_id, owner_user_id, project_id, idempotency_key,
                       display_name, runner_class, hosting_tier, placement_runner_class,
                       runtime_resource_class, desired_runtime_artifact_id, runtime_spec, target_source_host_id, relocation_spec,
                       profile_picture_url,
                       owner_chat_account_id,
                       status, requested_launch_code, agent_runtime_id,
                       runner_id, lease_token, core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
            &[&input.request_id, &agent_runtime_id, &now],
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
            let by_source = select_agent_runtime_by_source_import_key(client, &key)
                .await?
                .map(|runtime| runtime.id);
            if by_source.is_some() {
                by_source
            } else if request.relocation.is_some() {
                request.agent_runtime_id.clone()
            } else {
                None
            }
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
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
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
    postgres_visible_projects_for_user(client, &user_id).await
}

/// Visible projects for an already-resolved internal user id.
///
/// Split out so the WorkOS-keyed entry point and callers that already hold the
/// internal id share one query.
async fn postgres_visible_projects_for_user<C>(
    client: &C,
    user_id: &str,
) -> CoreResult<Vec<VisibleProject>>
where
    C: GenericClient + Sync,
{
    let rows = client
        .query(
            "SELECT project.id AS project_id, project.customer_org_id, project.owner_user_id,
                    project.display_name, project.agent_email, project.import_candidate_id,
                    project.hosting_tier,
                    project.placement_runner_class, project.runtime_resource_class,
                    core_rfc3339(project.created_at) AS created_at,
                    core_rfc3339(project.updated_at) AS updated_at,
                    runtime.id AS runtime_id, runtime.project_id AS runtime_project_id,
                    runtime.source_host_id, runtime.source_machine_id, runtime.source_import_key,
                    runtime.runtime_artifact_id, runtime.state_schema_version,
                    runtime.placement_runner_class AS runtime_placement_runner_class,
                    runtime.runtime_resource_class AS runtime_runtime_resource_class,
                    runtime.provider_runtime_handle, runtime.provider_runtime_handle_history,
                    runtime.contact_endpoint, runtime.runtime_capabilities,
                    runtime.host_facts, core_rfc3339(runtime.created_at) AS runtime_created_at,
                    core_rfc3339(runtime.updated_at) AS runtime_updated_at,
                    control.id AS control_id, control.project_id AS control_project_id,
                    control.agent_runtime_id AS control_agent_runtime_id,
                    control.source_host_id AS control_source_host_id,
                    control.source_machine_id AS control_source_machine_id,
                    control.requested_by_user_id AS control_requested_by_user_id,
                    control.kind AS control_kind,
                    control.target_runtime_artifact_id AS control_target_runtime_artifact_id,
                    control.status AS control_status,
                    control.failure_stage AS control_failure_stage,
                    control.runner_id AS control_runner_id,
                    control.lease_token AS control_lease_token,
                    core_rfc3339(control.lease_expires_at) AS control_lease_expires_at,
                    control.failure_message AS control_failure_message,
                    core_rfc3339(control.created_at) AS control_created_at,
                    core_rfc3339(control.updated_at) AS control_updated_at,
                    core_rfc3339(control.completed_at) AS control_completed_at
             FROM project_room_memberships AS membership
             JOIN chat_identities AS identity ON identity.id = membership.chat_identity_id
             JOIN projects AS project ON project.id = membership.project_id
             LEFT JOIN project_runtime_links AS link
               ON link.project_id = project.id AND link.active
             LEFT JOIN agent_runtimes AS runtime ON runtime.id = link.agent_runtime_id
             LEFT JOIN LATERAL (
               SELECT request.*
               FROM runtime_control_requests AS request
               WHERE request.agent_runtime_id = runtime.id
                 AND request.status IN ('requested', 'launching', 'compute_up', 'ready')
               ORDER BY request.created_at, request.id
               LIMIT 1
             ) AS control ON TRUE
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
                agent_email: row.get("agent_email"),
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
            let active_runtime_control = row
                .get::<_, Option<String>>("control_id")
                .map(|id| {
                    let status = parse_runtime_control_request_status(
                        &row.get::<_, String>("control_status"),
                    )
                    .ok_or_else(|| CoreError::Store("invalid runtime control status".into()))?;
                    let failure_stage = if status == RuntimeControlRequestStatus::Failed {
                        Some(
                            parse_runtime_lifecycle_stage(
                                &row.get::<_, String>("control_failure_stage"),
                            )
                            .ok_or_else(|| {
                                CoreError::Store("invalid runtime control failure stage".into())
                            })?,
                        )
                    } else {
                        None
                    };
                    Ok::<RuntimeControlRequest, CoreError>(RuntimeControlRequest {
                        id,
                        project_id: row.get("control_project_id"),
                        agent_runtime_id: row.get("control_agent_runtime_id"),
                        source_host_id: row.get("control_source_host_id"),
                        source_machine_id: row.get("control_source_machine_id"),
                        requested_by_user_id: row.get("control_requested_by_user_id"),
                        kind: parse_runtime_control_kind(&row.get::<_, String>("control_kind"))
                            .ok_or_else(|| {
                                CoreError::Store("invalid runtime control kind".into())
                            })?,
                        target_runtime_artifact_id: row.get("control_target_runtime_artifact_id"),
                        status,
                        failure_stage,
                        runner_id: row.get("control_runner_id"),
                        lease_token: row.get("control_lease_token"),
                        lease_expires_at: row.get("control_lease_expires_at"),
                        failure_message: row.get("control_failure_message"),
                        created_at: row.get("control_created_at"),
                        updated_at: row.get("control_updated_at"),
                        completed_at: row.get("control_completed_at"),
                    })
                })
                .transpose()?;
            Ok(VisibleProject {
                project,
                runtime,
                active_runtime_control,
            })
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
                    request.desired_runtime_artifact_id, request.runtime_spec, request.target_source_host_id, request.relocation_spec,
                    request.profile_picture_url,
                    request.owner_chat_account_id,
                    request.status, request.requested_launch_code, request.agent_runtime_id,
                    request.runner_id, request.lease_token, core_rfc3339(request.lease_expires_at) AS lease_expires_at,
                    request.failure_message, core_rfc3339(request.created_at) AS created_at, core_rfc3339(request.updated_at) AS updated_at
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

pub(crate) fn optional_hosting_tier_column(
    row: &Row,
    name: &str,
) -> CoreResult<Option<HostingTier>> {
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
        agent_email: row.get("agent_email"),
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
    let relocation = optional_json_column(row, "relocation_spec")?;
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
        target_source_host_id: row.get("target_source_host_id"),
        relocation: relocation
            .map(serde_json::from_value)
            .transpose()
            .map_err(json_error)?,
        profile_picture_url: row.get("profile_picture_url"),
        owner_chat_account_id: row.get("owner_chat_account_id"),
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
        burst_window_epoch: row.get("burst_window_epoch"),
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
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
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
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
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
            "SELECT id, owner_user_id, name, billing_class, core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
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
            "SELECT id, customer_org_id, owner_user_id, display_name, agent_email,
                    import_candidate_id,
                    hosting_tier, placement_runner_class, runtime_resource_class,
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
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
                    runtime_resource_class, desired_runtime_artifact_id, runtime_spec, target_source_host_id, relocation_spec,
                    profile_picture_url,
                    owner_chat_account_id,
                    status, requested_launch_code, agent_runtime_id,
                    runner_id, lease_token, core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message,
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
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
                    runtime_resource_class, desired_runtime_artifact_id, runtime_spec, target_source_host_id, relocation_spec,
                    profile_picture_url,
                    owner_chat_account_id,
                    status, requested_launch_code, agent_runtime_id,
                    runner_id, lease_token, core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message,
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
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
                    core_rfc3339(created_at) AS created_at, core_rfc3339(promoted_at) AS promoted_at, core_rfc3339(retired_at) AS retired_at
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
                    core_rfc3339(created_at) AS created_at, core_rfc3339(promoted_at) AS promoted_at, core_rfc3339(retired_at) AS retired_at
             FROM runtime_artifacts
             WHERE promoted_at IS NOT NULL AND retired_at IS NULL AND kind = 'oci_image'
             -- Qualified: a bare name here would bind the rendered-text output
             -- column and sort lexicographically, which is not chronological
             -- once fractional seconds vary.
             ORDER BY runtime_artifacts.promoted_at DESC,
                      runtime_artifacts.created_at DESC, id DESC
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
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
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
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
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
            "SELECT id, name, hosting_tier, code_count, core_rfc3339(expires_at) AS expires_at, core_rfc3339(revoked_at) AS revoked_at,
                    revoked_by_workos_user_id, created_by_workos_user_id,
                    core_rfc3339(created_at) AS created_at
               FROM launch_code_batches
              ORDER BY launch_code_batches.created_at DESC, id DESC",
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
            "SELECT id, redeemed_customer_org_id, core_rfc3339(redeemed_at) AS redeemed_at
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
                    code.redemption_idempotency_key, core_rfc3339(code.redeemed_at) AS redeemed_at,
                    core_rfc3339(code.created_at) AS created_at,
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
pub(crate) async fn upsert_linked_user<C>(
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
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
            &[&user_id, &email, &workos_user_id, &now],
        )
        .await
        .map_err(store_error)?;
    core_user_from_row(&row)
}

pub(crate) async fn ensure_personal_org_row<C>(
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
             RETURNING id, owner_user_id, name, billing_class, core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
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
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
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

pub(crate) async fn ensure_standard_agent_creation_entitlement_row<C>(
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
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
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
               (id, customer_org_id, owner_user_id, display_name, agent_email,
                import_candidate_id, hosting_tier, placement_runner_class,
                runtime_resource_class, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                     $10::text::timestamptz, $11::text::timestamptz)
             ON CONFLICT (id) DO UPDATE SET
               display_name = EXCLUDED.display_name,
               agent_email = COALESCE(projects.agent_email, EXCLUDED.agent_email),
               hosting_tier = EXCLUDED.hosting_tier,
               placement_runner_class = EXCLUDED.placement_runner_class,
               runtime_resource_class = EXCLUDED.runtime_resource_class,
               updated_at = EXCLUDED.updated_at",
            &[
                &project.id,
                &project.customer_org_id,
                &project.owner_user_id,
                &project.display_name,
                &project.agent_email,
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
    let relocation = request
        .relocation
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(json_error)?;
    client
        .execute(
            "INSERT INTO agent_creation_requests (
               id, customer_org_id, owner_user_id, project_id, idempotency_key, display_name,
               runner_class, hosting_tier, placement_runner_class, runtime_resource_class,
               desired_runtime_artifact_id, runtime_spec, target_source_host_id,
               relocation_spec,
               profile_picture_url, owner_chat_account_id, status, requested_launch_code,
               agent_runtime_id, runner_id, lease_token,
               lease_expires_at, failure_message, created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::jsonb,
                     $13, $14::jsonb, $15, $16, $17, $18, $19, $20, $21,
                     $22::text::timestamptz, $23, $24::text::timestamptz,
                     $25::text::timestamptz)
             ON CONFLICT (id) DO UPDATE SET
               status = EXCLUDED.status,
               display_name = EXCLUDED.display_name,
               runner_class = EXCLUDED.runner_class,
               hosting_tier = EXCLUDED.hosting_tier,
               placement_runner_class = EXCLUDED.placement_runner_class,
               runtime_resource_class = EXCLUDED.runtime_resource_class,
               desired_runtime_artifact_id = EXCLUDED.desired_runtime_artifact_id,
               runtime_spec = EXCLUDED.runtime_spec,
               target_source_host_id = EXCLUDED.target_source_host_id,
               relocation_spec = EXCLUDED.relocation_spec,
               profile_picture_url = EXCLUDED.profile_picture_url,
               owner_chat_account_id = EXCLUDED.owner_chat_account_id,
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
                &request.target_source_host_id,
                &relocation,
                &request.profile_picture_url,
                &request.owner_chat_account_id,
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

/// Return an existing profile, create one of Core's built-in profiles on
/// demand, and reject unknown profile identifiers.
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
    let burst_limit_units = match id {
        crate::DEFAULT_FINITE_PRIVATE_LIMIT_PROFILE => {
            crate::DEFAULT_FINITE_PRIVATE_BURST_LIMIT_UNITS
        }
        crate::FINITE_PRIVATE_5X_LIMIT_PROFILE => crate::FINITE_PRIVATE_5X_BURST_LIMIT_UNITS,
        _ => return Err(CoreError::FinitePrivateLimitProfileNotFound),
    };
    client
        .execute(
            "INSERT INTO finite_private_limit_profiles (
               id, burst_window_seconds, burst_limit_units, weekly_limit_units, created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5::text::timestamptz, $5::text::timestamptz)
             ON CONFLICT (id) DO NOTHING",
            &[
                &id,
                &crate::DEFAULT_FINITE_PRIVATE_BURST_WINDOW_SECONDS,
                &burst_limit_units,
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
               current_window_used_units, burst_window_epoch, created_at, updated_at
             )
             VALUES ($1, $2, $3, 'active', NULL, 0, 0, $4::text::timestamptz, $4::text::timestamptz)
             ON CONFLICT (user_id) DO UPDATE SET
               limit_profile_id = EXCLUDED.limit_profile_id,
               status = 'active',
               current_window_started_at = NULL,
               current_window_used_units = 0,
               burst_window_epoch = finite_private_grants.burst_window_epoch + 1,
               updated_at = EXCLUDED.updated_at
             RETURNING id, user_id, limit_profile_id, status,
                       core_rfc3339(current_window_started_at) AS current_window_started_at,
                       current_window_used_units, burst_window_epoch,
                       core_rfc3339(created_at) AS created_at,
                       core_rfc3339(updated_at) AS updated_at",
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
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
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
            "SELECT sequence, transition, core_rfc3339(recorded_at) AS recorded_at
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
    // A verified retirement receipt is terminal: re-registration of a retired
    // Runtime must never flip its retired link back to active. Live runtimes
    // have no snapshot rows, so normal launch and registration are unaffected.
    if client
        .query_opt(
            "SELECT 1
             FROM runtime_retirement_snapshots
             WHERE agent_runtime_id = $1 AND verified_at IS NOT NULL
             LIMIT 1",
            &[&runtime_id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::RuntimeRetirementSnapshotConflict);
    }
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
                       runtime_resource_class, desired_runtime_artifact_id, runtime_spec, target_source_host_id, relocation_spec,
                       profile_picture_url,
                       owner_chat_account_id,
                       status, requested_launch_code, agent_runtime_id,
                       runner_id, lease_token, core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
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
                       runtime_resource_class, desired_runtime_artifact_id, runtime_spec, target_source_host_id, relocation_spec,
                       profile_picture_url,
                       owner_chat_account_id,
                       status, requested_launch_code, agent_runtime_id,
                       runner_id, lease_token, core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
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
    // runtime_status_snapshots has no writer anymore, but production still
    // holds rows written before the writer was removed and the table's FK to
    // agent_runtimes has no ON DELETE CASCADE. This DELETE must stay until the
    // table itself is dropped, or deleting such a runtime fails the FK check.
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
    let failure_stage: String = row.get("failure_stage");
    let status = parse_runtime_control_request_status(&status).ok_or_else(|| {
        CoreError::Store(format!("invalid runtime control request status {status}"))
    })?;
    // The lifecycle invariant: a failed request always names its stage, and
    // only a failed request carries one.
    let failure_stage = if status == RuntimeControlRequestStatus::Failed {
        Some(
            parse_runtime_lifecycle_stage(&failure_stage).ok_or_else(|| {
                CoreError::Store(format!(
                    "invalid runtime control failure stage {failure_stage}"
                ))
            })?,
        )
    } else {
        None
    };
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
        status,
        failure_stage,
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
    status, failure_stage, runner_id, lease_token,
    core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message, core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at, core_rfc3339(completed_at) AS completed_at";

async fn postgres_runtime_control_request<C>(
    client: &C,
    request_id: &str,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT {RUNTIME_CONTROL_REQUEST_COLUMNS} FROM runtime_control_requests WHERE id = $1"
    );
    let row = client
        .query_opt(&sql, &[&request_id])
        .await
        .map_err(store_error)?
        .ok_or(CoreError::RuntimeControlRequestNotFound)?;
    runtime_control_request_from_row(&row)
}

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
                    runtime.host_facts, core_rfc3339(runtime.created_at) AS created_at, core_rfc3339(runtime.updated_at) AS updated_at
             FROM project_runtime_links AS link
             JOIN agent_runtimes AS runtime ON runtime.id = link.agent_runtime_id
             WHERE link.project_id = $1 AND link.active
             LIMIT 1
             FOR UPDATE OF runtime, link",
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
    postgres_enqueue_runtime_control_request_bound(
        client,
        project,
        requested_by_user_id,
        kind,
        target_runtime_artifact_id,
        now,
        None,
    )
    .await
}

async fn postgres_enqueue_runtime_control_request_bound<C>(
    client: &C,
    project: &Project,
    requested_by_user_id: &str,
    kind: RuntimeControlKind,
    target_runtime_artifact_id: Option<String>,
    now: &str,
    expected: Option<&RuntimeControlExpectedBinding>,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    let runtime = postgres_active_runtime_for_project(client, &project.id)
        .await?
        .ok_or(CoreError::ProjectRuntimeNotFound)?;
    if expected.is_some_and(|expected| {
        runtime.id != expected.agent_runtime_id
            || runtime.source_host_id != expected.source_host_id
            || runtime.source_machine_id != expected.source_machine_id
    }) {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    if kind == RuntimeControlKind::Destroy
        && let Some(phase) = postgres_offboarding_phase(client, &runtime.id).await?
        && phase.reached(OffboardingPhase::ReceiptVerified)
    {
        // A verified retirement receipt is already stored, so the destroy
        // boundary is behind this Runtime. Enqueueing a fresh destroy mints a
        // new request id whose retirement archive can never exist — the
        // uncapped retry wedge. The recorded phase is the resume point
        // instead: finish offboarding through runtime-offboard-retired-exact.
        return Err(CoreError::RuntimeOffboardingResumeRequired { phase });
    }
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
         WHERE agent_runtime_id = $1
           AND status IN ('requested', 'launching', 'compute_up', 'ready')
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
        if kind == RuntimeControlKind::Destroy {
            set_offboarding_phase(
                client,
                &existing.agent_runtime_id,
                OffboardingPhase::RetirementRequested,
                now,
            )
            .await?;
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
        failure_stage: None,
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
                       failure_stage, runner_id, lease_token,
                       core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message, core_rfc3339(created_at) AS created_at,
                       core_rfc3339(updated_at) AS updated_at, core_rfc3339(completed_at) AS completed_at",
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
    let request = runtime_control_request_from_row(&row)?;
    if kind == RuntimeControlKind::Destroy {
        // The destroy request is durably enqueued; record the first
        // offboarding phase in the same transaction.
        set_offboarding_phase(
            client,
            &request.agent_runtime_id,
            OffboardingPhase::RetirementRequested,
            now,
        )
        .await?;
    }
    Ok(request)
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
    postgres_admin_request_runtime_control_bound(
        client,
        input,
        kind,
        target_runtime_artifact_id,
        None,
    )
    .await
}

async fn postgres_admin_archive_unrecoverable_runtime<C>(
    client: &C,
    input: AdminArchiveUnrecoverableRuntimeInput,
) -> CoreResult<UnrecoverableRuntimeArchiveReceipt>
where
    C: GenericClient + Sync,
{
    if !input.operator_observed_compute_absent
        || !input.operator_observed_durable_state_absent
        || !input.owner_acknowledged_unrecoverable
    {
        return Err(CoreError::UnrecoverableRuntimeArchiveAcknowledgementRequired);
    }
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let admin_workos_user_id = input.admin_workos_user_id.trim().to_string();
    if admin_workos_user_id.is_empty() {
        return Err(CoreError::MissingWorkosUserId);
    }
    let expected_owner_email = normalize_owner_email(Some(&input.expected_owner_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let row = client
        .query_opt(
            "SELECT runtime.id AS agent_runtime_id, runtime.source_host_id,
                    runtime.source_machine_id, owner.normalized_email AS owner_email,
                    (
                      runtime.provider_runtime_handle IS NOT NULL
                      OR COALESCE(jsonb_array_length(runtime.provider_runtime_handle_history), 0) > 0
                      OR runtime.contact_endpoint IS NOT NULL
                    ) AS has_provider_metadata
             FROM projects AS project
             JOIN users AS owner ON owner.id = project.owner_user_id
             JOIN project_runtime_links AS link
               ON link.project_id = project.id AND link.active = TRUE
             JOIN agent_runtimes AS runtime ON runtime.id = link.agent_runtime_id
             WHERE project.id = $1
             FOR UPDATE OF project, link, runtime",
            &[&input.project_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::ProjectRuntimeNotFound)?;
    let agent_runtime_id: String = row.get("agent_runtime_id");
    let source_host_id: String = row.get("source_host_id");
    let source_machine_id: String = row.get("source_machine_id");
    let owner_email: String = row.get("owner_email");
    if owner_email != expected_owner_email {
        return Err(CoreError::UnrecoverableRuntimeArchiveOwnerMismatch);
    }
    if agent_runtime_id != input.expected_agent_runtime_id
        || source_host_id != input.expected_source_host_id
        || source_machine_id != input.expected_source_machine_id
    {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    if row.get::<_, bool>("has_provider_metadata") {
        return Err(CoreError::UnrecoverableRuntimeArchiveProviderMetadataPresent);
    }
    if client
        .query_opt(
            "SELECT 1
             FROM runtime_control_requests
             WHERE agent_runtime_id = $1
               AND status IN ('requested', 'launching', 'compute_up', 'ready')
             LIMIT 1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::RuntimeControlOperationConflict);
    }
    if client
        .query_opt(
            "SELECT 1 FROM runtime_retirement_snapshots WHERE agent_runtime_id = $1 LIMIT 1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::RuntimeRetirementSnapshotConflict);
    }

    ensure_grandfathered_linked_user(client, &admin_email, &admin_workos_user_id, &now).await?;
    let revoked_api_key_ids = postgres_offboard_runtime(
        client,
        &input.project_id,
        &agent_runtime_id,
        &now,
        "finite_private.runtime.archive_unrecoverable_revoke_keys",
        Some(&admin_email),
    )
    .await?;
    set_offboarding_phase(client, &agent_runtime_id, OffboardingPhase::Archived, &now).await?;
    let revoked_finite_private_key_count = revoked_api_key_ids.len();
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "runtime.admin_archive_unrecoverable",
            target_type: "agent_runtime",
            target_id: &agent_runtime_id,
            grant_id: None,
            api_key_id: None,
            actor: Some(&admin_email),
            metadata: json!({
                "projectId": input.project_id,
                "ownerEmail": owner_email,
                "sourceHostId": source_host_id,
                "sourceMachineId": source_machine_id,
                "operatorObservedComputeAbsent": true,
                "operatorObservedDurableStateAbsent": true,
                "ownerAcknowledgedUnrecoverable": true,
                "revokedApiKeyIds": revoked_api_key_ids,
            }),
            now: &now,
        },
    )
    .await?;
    Ok(UnrecoverableRuntimeArchiveReceipt {
        project_id: input.project_id,
        agent_runtime_id,
        source_host_id,
        source_machine_id,
        owner_email,
        archived_at: now,
        revoked_finite_private_key_count,
    })
}

/// Repair boundary for a Runtime whose destroy control stored a VERIFIED
/// retirement receipt but whose offboarding transaction never ran (the
/// `project_runtime_links.active` link, room membership, relay credential, and
/// Finite Private keys survive with no compute behind them). This path never
/// creates, modifies, or deletes the retirement snapshot, and never touches
/// provider metadata columns.
///
/// The command is safe to run twice: a second run finds no active link and
/// fails closed with `ProjectRuntimeNotFound`, leaving every committed effect
/// of the first run untouched.
async fn postgres_admin_offboard_retired_runtime<C>(
    client: &C,
    input: AdminOffboardRetiredRuntimeInput,
) -> CoreResult<RetiredRuntimeOffboardReceipt>
where
    C: GenericClient + Sync,
{
    if !input.operator_observed_compute_absent {
        return Err(CoreError::RetiredRuntimeOffboardAcknowledgementRequired);
    }
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let admin_workos_user_id = input.admin_workos_user_id.trim().to_string();
    if admin_workos_user_id.is_empty() {
        return Err(CoreError::MissingWorkosUserId);
    }
    let expected_owner_email = normalize_owner_email(Some(&input.expected_owner_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let row = client
        .query_opt(
            "SELECT runtime.id AS agent_runtime_id, runtime.source_host_id,
                    runtime.source_machine_id, owner.normalized_email AS owner_email
             FROM projects AS project
             JOIN users AS owner ON owner.id = project.owner_user_id
             JOIN project_runtime_links AS link
               ON link.project_id = project.id AND link.active = TRUE
             JOIN agent_runtimes AS runtime ON runtime.id = link.agent_runtime_id
             WHERE project.id = $1
             FOR UPDATE OF project, link, runtime",
            &[&input.project_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::ProjectRuntimeNotFound)?;
    let agent_runtime_id: String = row.get("agent_runtime_id");
    let source_host_id: String = row.get("source_host_id");
    let source_machine_id: String = row.get("source_machine_id");
    let owner_email: String = row.get("owner_email");
    if owner_email != expected_owner_email {
        return Err(CoreError::RetiredRuntimeOffboardOwnerMismatch);
    }
    if agent_runtime_id != input.expected_agent_runtime_id
        || source_host_id != input.expected_source_host_id
        || source_machine_id != input.expected_source_machine_id
    {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    if client
        .query_opt(
            "SELECT 1
             FROM runtime_control_requests
             WHERE agent_runtime_id = $1
               AND status IN ('requested', 'launching', 'compute_up', 'ready')
             LIMIT 1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::RuntimeControlOperationConflict);
    }
    let snapshot_row = client
        .query_opt(
            "SELECT request_id
             FROM runtime_retirement_snapshots
             WHERE agent_runtime_id = $1 AND verified_at IS NOT NULL
             LIMIT 1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::RetiredRuntimeOffboardReceiptMissing)?;
    let retirement_request_id: String = snapshot_row.get("request_id");
    let snapshot = postgres_runtime_retirement_snapshot(client, &retirement_request_id)
        .await?
        .ok_or(CoreError::RetiredRuntimeOffboardReceiptMissing)?;
    let request = locked_runtime_control_request(client, &retirement_request_id).await?;
    let runtime = select_agent_runtime(client, &agent_runtime_id)
        .await?
        .ok_or(CoreError::ProjectRuntimeNotFound)?;
    let spec_row = client
        .query_opt(
            "SELECT runtime_spec
             FROM agent_creation_requests
             WHERE agent_runtime_id = $1 AND runtime_spec IS NOT NULL
             ORDER BY created_at DESC, id DESC
             LIMIT 1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::RuntimeRetirementSnapshotMismatch)?;
    let value: Value = spec_row.get("runtime_spec");
    let runtime_spec: RuntimeSpecEnvelope = serde_json::from_value(value).map_err(json_error)?;
    // The stored receipt must re-verify against its own destroy request,
    // Runtime binding, and RuntimeSpec exactly as at destroy completion.
    validate_runtime_retirement_snapshot_receipt(
        &snapshot.receipt,
        &request,
        &runtime,
        &runtime_spec,
        &now,
    )?;

    ensure_grandfathered_linked_user(client, &admin_email, &admin_workos_user_id, &now).await?;
    // The operator's compute-absent attestation (required above) plus the
    // re-verified receipt resume the recorded phase forward.
    set_offboarding_phase(
        client,
        &agent_runtime_id,
        OffboardingPhase::ComputeRemoved,
        &now,
    )
    .await?;
    let revoked_api_key_ids = postgres_offboard_runtime(
        client,
        &input.project_id,
        &agent_runtime_id,
        &now,
        "finite_private.runtime.offboard_retired_revoke_keys",
        Some(&admin_email),
    )
    .await?;
    set_offboarding_phase(client, &agent_runtime_id, OffboardingPhase::Archived, &now).await?;
    let revoked_finite_private_key_count = revoked_api_key_ids.len();
    let retirement_locator = snapshot.receipt.locator.clone();
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "runtime.admin_offboard_retired",
            target_type: "agent_runtime",
            target_id: &agent_runtime_id,
            grant_id: None,
            api_key_id: None,
            actor: Some(&admin_email),
            metadata: json!({
                "projectId": input.project_id,
                "ownerEmail": owner_email,
                "sourceHostId": source_host_id,
                "sourceMachineId": source_machine_id,
                "operatorObservedComputeAbsent": true,
                "retirementRequestId": retirement_request_id,
                "retirementLocator": retirement_locator,
                "revokedApiKeyIds": revoked_api_key_ids,
            }),
            now: &now,
        },
    )
    .await?;
    Ok(RetiredRuntimeOffboardReceipt {
        project_id: input.project_id,
        agent_runtime_id,
        retirement_request_id,
        retirement_locator,
        offboarded_at: now,
        revoked_finite_private_key_count,
    })
}

async fn postgres_admin_request_runtime_control_bound<C>(
    client: &C,
    input: AdminRuntimeControlInput,
    kind: RuntimeControlKind,
    target_runtime_artifact_id: Option<String>,
    expected: Option<&RuntimeControlExpectedBinding>,
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
    let request = postgres_enqueue_runtime_control_request_bound(
        client,
        &project,
        &admin_user.id,
        kind,
        target_runtime_artifact_id,
        &now,
        expected,
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

async fn postgres_admin_request_runtime_upgrade_exact<C>(
    client: &C,
    input: AdminRuntimeUpgradeExactInput,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    let expected = RuntimeControlExpectedBinding {
        agent_runtime_id: input.expected_agent_runtime_id,
        source_host_id: input.expected_source_host_id,
        source_machine_id: input.expected_source_machine_id,
    };
    postgres_admin_request_runtime_control_bound(
        client,
        AdminRuntimeControlInput {
            admin_verified_email: input.admin_verified_email,
            admin_workos_user_id: input.admin_workos_user_id,
            project_id: input.project_id,
            now: input.now,
        },
        RuntimeControlKind::Upgrade,
        Some(input.target_runtime_artifact_id),
        Some(&expected),
    )
    .await
}

async fn postgres_admin_request_runtime_retire_exact<C>(
    client: &C,
    input: AdminRuntimeRetireExactInput,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    let expected = RuntimeControlExpectedBinding {
        agent_runtime_id: input.expected_agent_runtime_id,
        source_host_id: input.expected_source_host_id,
        source_machine_id: input.expected_source_machine_id,
    };
    postgres_admin_request_runtime_control_bound(
        client,
        AdminRuntimeControlInput {
            admin_verified_email: input.admin_verified_email,
            admin_workos_user_id: input.admin_workos_user_id,
            project_id: input.project_id,
            now: input.now,
        },
        RuntimeControlKind::Destroy,
        None,
        Some(&expected),
    )
    .await
}

async fn postgres_admin_request_runtime_relocate_exact<C>(
    client: &C,
    input: AdminRuntimeRelocateExactInput,
) -> CoreResult<AgentCreationRequest>
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
    let _admin_user =
        ensure_grandfathered_linked_user(client, &admin_email, &admin_workos_user_id, &now).await?;
    let project = select_project(client, &input.project_id)
        .await?
        .ok_or(CoreError::ProjectNotFound)?;
    let runtime = postgres_active_runtime_for_project(client, &project.id)
        .await?
        .ok_or(CoreError::ProjectRuntimeNotFound)?;
    if runtime.id != input.expected_agent_runtime_id
        || runtime.source_host_id != input.expected_source_host_id
        || runtime.source_machine_id != input.expected_source_machine_id
    {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    let placement = runtime.placement.ok_or(CoreError::RuntimeSpecMismatch)?;
    // `offline` is the cleanly-stopped precondition. Under the operator's
    // compute-absent attestation, `stale` AND `online` are also frozen: a
    // failed control marks a runtime stale, and absent compute can never
    // reach `offline` because the stop that would record it fails by
    // definition. `online` is the last runner report before the source
    // host died — under the attestation nothing could have updated it
    // since (the dead host's runner is gone, so no control can lease and
    // no report can arrive), making it exactly as frozen as `stale`.
    // Without the attestation `online` stays movable-only-by-nothing: an
    // operator must not relocate a runtime that may still be running.
    let source_status_frozen = match runtime.host_facts.runtime_status {
        RuntimeSummaryStatus::Offline => true,
        RuntimeSummaryStatus::Online => input.operator_observed_compute_absent,
        RuntimeSummaryStatus::Stale => input.operator_observed_compute_absent,
        _ => false,
    };
    if placement.runner_class != crate::RunnerClass::Kata || !source_status_frozen {
        return Err(CoreError::RuntimeControlUnsupported);
    }
    let target_source_host_id = normalize_source_host_id(&input.target_source_host_id)?;
    if target_source_host_id == runtime.source_host_id {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    let expected_agent_npub = input.expected_agent_npub.trim().to_string();
    let manifest = input
        .durable_state_manifest_sha256
        .trim()
        .to_ascii_lowercase();
    if !valid_agent_npub(&expected_agent_npub) || !valid_sha256_hex(&manifest) {
        return Err(CoreError::RuntimeSpecMismatch);
    }
    if client
        .query_opt(
            "SELECT 1
             FROM runtime_control_requests
             WHERE agent_runtime_id = $1
               AND status IN ('requested', 'launching', 'compute_up', 'ready')
             LIMIT 1",
            &[&runtime.id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::RuntimeControlOperationConflict);
    }
    // The stopped stop receipt proves no writer survives on the source.
    // Under the compute-absent attestation there is nothing to stop and the
    // receipt is unobtainable; absence itself (verified by the operator's
    // bounded probe per the relocation runbook) is the stronger guarantee.
    if !input.operator_observed_compute_absent
        && client
            .query_opt(
                "SELECT 1
             FROM runtime_control_requests
             WHERE agent_runtime_id = $1
               AND source_host_id = $2
               AND source_machine_id = $3
               AND kind = 'stop'
               AND status = 'stopped'
             LIMIT 1",
                &[
                    &runtime.id,
                    &runtime.source_host_id,
                    &runtime.source_machine_id,
                ],
            )
            .await
            .map_err(store_error)?
            .is_none()
    {
        return Err(CoreError::RuntimeControlOperationConflict);
    }
    if client
        .query_opt(
            "SELECT 1
             FROM runtime_retirement_snapshots AS snapshot
             JOIN runtime_control_requests AS control ON control.id = snapshot.request_id
             WHERE control.agent_runtime_id = $1
             LIMIT 1",
            &[&runtime.id],
        )
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(CoreError::RuntimeRetirementSnapshotConflict);
    }
    let relocation = RuntimeRelocationEnvelope::V1(RuntimeRelocationV1 {
        source_host_id: runtime.source_host_id.clone(),
        source_machine_id: runtime.source_machine_id.clone(),
        target_source_host_id: target_source_host_id.clone(),
        expected_agent_npub,
        durable_state_manifest_sha256: manifest,
        source_compute_absent: input.operator_observed_compute_absent,
    });
    let active_sql = "SELECT id, customer_org_id, owner_user_id, project_id, idempotency_key,
               display_name, runner_class, hosting_tier, placement_runner_class,
               runtime_resource_class, desired_runtime_artifact_id, runtime_spec,
               target_source_host_id, relocation_spec, profile_picture_url,
               owner_chat_account_id,
               status, requested_launch_code, agent_runtime_id,
               runner_id, lease_token, lease_expires_at::text, failure_message,
               created_at::text, updated_at::text
         FROM agent_creation_requests
         WHERE agent_runtime_id = $1
           AND relocation_spec IS NOT NULL
           AND status IN ('requested', 'launching')
         ORDER BY created_at, id
         LIMIT 1
         FOR UPDATE";
    if let Some(row) = client
        .query_opt(active_sql, &[&runtime.id])
        .await
        .map_err(store_error)?
    {
        let existing = agent_creation_request_from_row(&row)?;
        if existing.relocation.as_ref() == Some(&relocation)
            && existing.target_source_host_id.as_deref() == Some(target_source_host_id.as_str())
        {
            return Ok(existing);
        }
        return Err(CoreError::RuntimeControlOperationConflict);
    }
    let current_row = client
        .query_opt(
            "SELECT id, customer_org_id, owner_user_id, project_id, idempotency_key,
                    display_name, runner_class, hosting_tier, placement_runner_class,
                    runtime_resource_class, desired_runtime_artifact_id, runtime_spec,
                    target_source_host_id, relocation_spec, profile_picture_url,
                    owner_chat_account_id,
                    status, requested_launch_code, agent_runtime_id,
                    runner_id, lease_token, lease_expires_at::text, failure_message,
                    created_at::text, updated_at::text
             FROM agent_creation_requests
             WHERE agent_runtime_id = $1
               AND status = 'running'
               AND runtime_spec IS NOT NULL
             ORDER BY created_at DESC, id DESC
             LIMIT 1",
            &[&runtime.id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::RuntimeSpecMismatch)?;
    let current_creation = agent_creation_request_from_row(&current_row)?;
    let artifact_id = runtime
        .runtime_artifact_id
        .as_deref()
        .ok_or(CoreError::MissingRuntimeArtifactId)?;
    let artifact = select_runtime_artifact(client, artifact_id)
        .await?
        .ok_or(CoreError::RuntimeArtifactNotFound)?;
    let request_id = new_agent_creation_request_id()?;
    let runtime_spec = runtime_operation_spec_v1(
        current_creation
            .runtime_spec
            .as_ref()
            .ok_or(CoreError::RuntimeSpecMismatch)?,
        RuntimeSpecIdentity {
            operation_id: &request_id,
            project_id: &project.id,
            agent_runtime_id: &runtime.id,
            placement,
        },
        &artifact,
        &artifact,
        RuntimeBootIntent::Normal,
        None,
        None,
    )?;
    let idempotency_key = format!(
        "cold-relocate:{}:{}:{}",
        runtime.id, target_source_host_id, request_id
    );
    let request = AgentCreationRequest {
        id: request_id,
        customer_org_id: project.customer_org_id.clone(),
        owner_user_id: project.owner_user_id.clone(),
        project_id: project.id.clone(),
        idempotency_key,
        display_name: project.display_name.clone(),
        runner_class: placement.runner_class,
        hosting_tier: project.hosting_tier,
        placement: Some(placement),
        desired_runtime_artifact_id: Some(artifact.id),
        runtime_spec: Some(runtime_spec),
        target_source_host_id: Some(target_source_host_id),
        relocation: Some(relocation),
        profile_picture_url: current_creation.profile_picture_url,
        owner_chat_account_id: current_creation.owner_chat_account_id,
        status: AgentCreationRequestStatus::Requested,
        requested_launch_code: None,
        agent_runtime_id: Some(runtime.id.clone()),
        runner_id: None,
        lease_token: None,
        lease_expires_at: None,
        failure_message: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    upsert_agent_creation_request_row(client, &request).await?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "runtime.admin_cold_relocate",
            target_type: "agent_runtime",
            target_id: &runtime.id,
            grant_id: None,
            api_key_id: None,
            actor: Some(&admin_email),
            metadata: json!({
                "projectId": project.id,
                "agentCreationRequestId": request.id,
                "sourceHostId": runtime.source_host_id,
                "sourceMachineId": runtime.source_machine_id,
                "targetSourceHostId": request.target_source_host_id,
            }),
            now: &now,
        },
    )
    .await?;
    Ok(request)
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
    runtime_secret_references: &[String],
) -> CoreResult<Option<RuntimeControlLease>>
where
    C: GenericClient + Sync,
{
    validate_runtime_spec_environment(runtime_environment)?;
    runtime_spec_secret_references(runtime_secret_references)?;
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
                          request.status = 'launching'
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
             SET status = 'launching',
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
                       request.failure_stage,
                       request.runner_id, request.lease_token, core_rfc3339(request.lease_expires_at) AS lease_expires_at,
                       request.failure_message, core_rfc3339(request.created_at) AS created_at,
                       core_rfc3339(request.updated_at) AS updated_at, core_rfc3339(request.completed_at) AS completed_at",
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
                         SET status = 'failed', failure_stage = 'launch',
                             runner_id = NULL, lease_token = NULL,
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
                (request.kind == RuntimeControlKind::Upgrade).then_some(runtime_environment),
                (request.kind == RuntimeControlKind::Upgrade).then_some(runtime_secret_references),
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
    if request.status != RuntimeControlRequestStatus::Launching {
        return Err(CoreError::RuntimeControlRequestNotLaunching);
    }
    if request.runner_id.as_deref() != Some(runner_id.as_str())
        || request.lease_token.as_deref() != Some(lease_token.as_str())
    {
        return Err(CoreError::RuntimeControlRequestLeaseConflict);
    }
    Ok(())
}

async fn verify_postgres_runtime_control_lease_at<C>(
    client: &C,
    request: &RuntimeControlRequest,
    runner_id: &str,
    lease_token: &str,
    now: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    verify_runtime_control_lease(request, runner_id, lease_token)?;
    let active: bool = client
        .query_one(
            "SELECT COALESCE(lease_expires_at >= $2::text::timestamptz, FALSE)
             FROM runtime_control_requests WHERE id = $1",
            &[&request.id, &now],
        )
        .await
        .map_err(store_error)?
        .get(0);
    if !active {
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
    contact_endpoint: String,
    runtime_spec: Option<RuntimeSpecEnvelope>,
    runtime_capabilities: Option<RuntimeCapabilitiesEnvelope>,
}

async fn postgres_runtime_retirement_snapshot<C>(
    client: &C,
    request_id: &str,
) -> CoreResult<Option<RuntimeRetirementSnapshot>>
where
    C: GenericClient + Sync,
{
    let sql = format!(
        "SELECT request_id, project_id, agent_runtime_id, durable_state_id,
                runtime_artifact_id, schema_version, backend, locator,
                zip_bytes, zip_sha256, manifest_sha256,
                created_at, verified_at,
                recovery_authority_id, retention_policy, {stored_at} AS stored_at
         FROM runtime_retirement_snapshots
         WHERE request_id = $1",
        stored_at = rfc3339_col("stored_at"),
    );
    let Some(row) = client
        .query_opt(&sql, &[&request_id])
        .await
        .map_err(store_error)?
    else {
        return Ok(None);
    };
    let zip_bytes: i64 = row.get("zip_bytes");
    let zip_bytes =
        u64::try_from(zip_bytes).map_err(|_| CoreError::RuntimeRetirementSnapshotMismatch)?;
    Ok(Some(RuntimeRetirementSnapshot {
        receipt: RuntimeRetirementSnapshotReceipt {
            schema: row.get("schema_version"),
            request_id: row.get("request_id"),
            project_id: row.get("project_id"),
            agent_runtime_id: row.get("agent_runtime_id"),
            durable_state_id: row.get("durable_state_id"),
            runtime_artifact_id: row.get("runtime_artifact_id"),
            backend: row.get("backend"),
            locator: row.get("locator"),
            zip_bytes,
            zip_sha256: row.get("zip_sha256"),
            manifest_sha256: row.get("manifest_sha256"),
            created_at: row.get("created_at"),
            verified_at: row.get("verified_at"),
            recovery_authority_id: row.get("recovery_authority_id"),
            retention_policy: row.get("retention_policy"),
        },
        stored_at: row.get("stored_at"),
    }))
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
            runtime.contact_endpoint = Some(upgrade.contact_endpoint.clone());
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
    Ok(())
}

async fn postgres_complete_runtime_control_request<C>(
    client: &C,
    input: CompleteRuntimeControlRequestInput,
    runtime_environment: &BTreeMap<String, String>,
    runtime_secret_references: &[String],
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    validate_runtime_spec_environment(runtime_environment)?;
    runtime_spec_secret_references(runtime_secret_references)?;
    let now = input.now.clone().unwrap_or(current_time_iso()?);
    let locked = locked_runtime_control_request(client, &input.request_id).await?;
    // Terminal requests accept no completion. The single exception is the
    // idempotent Destroy replay: the same receipt re-presented against the
    // stopped request returns the stored row unchanged.
    if locked.status.is_terminal() {
        let stored = postgres_runtime_retirement_snapshot(client, &input.request_id).await?;
        let idempotent_destroy_replay = locked.status == RuntimeControlRequestStatus::Stopped
            && locked.kind == RuntimeControlKind::Destroy
            && matches!(
                RuntimeControlCompletion::parse(locked.kind, &input),
                Ok(RuntimeControlCompletion::Destroy(ref receipt))
                    if stored.as_ref().map(|snapshot| &snapshot.receipt) == Some(&**receipt)
            );
        if idempotent_destroy_replay {
            return Ok(locked);
        }
        return Err(CoreError::RuntimeRetirementSnapshotConflict);
    }
    verify_postgres_runtime_control_lease_at(
        client,
        &locked,
        &input.runner_id,
        &input.lease_token,
        &now,
    )
    .await?;
    // The completion shape is parsed once and keyed on the request kind, so
    // the upgrade-with-facts / destroy / plain shapes cannot be confused
    // anywhere below this line.
    let completion = RuntimeControlCompletion::parse(locked.kind, &input)?;
    let retirement_snapshot = match &completion {
        RuntimeControlCompletion::Destroy(receipt) => {
            let runtime = select_agent_runtime(client, &locked.agent_runtime_id)
                .await?
                .ok_or(CoreError::ProjectRuntimeNotFound)?;
            let row = client
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
                .ok_or(CoreError::RuntimeRetirementSnapshotMismatch)?;
            let value: Value = row.get("runtime_spec");
            let runtime_spec: RuntimeSpecEnvelope =
                serde_json::from_value(value).map_err(json_error)?;
            validate_runtime_retirement_snapshot_receipt(
                receipt,
                &locked,
                &runtime,
                &runtime_spec,
                &now,
            )?;
            Some(RuntimeRetirementSnapshot {
                receipt: (**receipt).clone(),
                stored_at: now.clone(),
            })
        }
        _ => None,
    };
    let upgrade = match &completion {
        RuntimeControlCompletion::Upgrade(facts) => {
            let target_id = locked
                .target_runtime_artifact_id
                .as_deref()
                .ok_or(CoreError::RuntimeUpgradeCompletionMismatch)?;
            let reported_id = facts.runtime_artifact_id.clone();
            let target = select_runtime_artifact(client, target_id)
                .await?
                .ok_or(CoreError::RuntimeArtifactNotFound)?;
            let runtime = select_agent_runtime(client, &locked.agent_runtime_id)
                .await?
                .ok_or(CoreError::ProjectRuntimeNotFound)?;
            validate_runtime_capabilities_artifact_policy(
                facts.runtime_capabilities.as_ref(),
                runtime.placement,
                &target,
            )?;
            // A target may be retired after the runner leased and swapped it.
            // Immutable material remains authoritative for committing the actual
            // compute state; lifecycle policy is enforced at request and lease.
            ensure_runtime_upgrade_target_material(&runtime, &target)?;
            let state_schema_version = facts.state_schema_version.clone();
            let runtime_host = facts.runtime_host.clone();
            let published_app_urls = facts.published_app_urls.clone();
            let contact_endpoint = runtime_upgrade_contact_endpoint(&published_app_urls)?;
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
                    Some(runtime_environment),
                    Some(runtime_secret_references),
                )?)
            } else {
                None
            };
            Some(RuntimeUpgradeCompletion {
                runtime_artifact_id: reported_id,
                state_schema_version,
                runtime_host,
                published_app_urls,
                contact_endpoint,
                runtime_spec,
                runtime_capabilities: facts.runtime_capabilities.clone(),
            })
        }
        _ => None,
    };
    if let Some(snapshot) = retirement_snapshot.as_ref() {
        let receipt = &snapshot.receipt;
        let zip_bytes = receipt.zip_bytes as i64;
        let inserted = client
            .execute(
                "INSERT INTO runtime_retirement_snapshots (
                   request_id, project_id, agent_runtime_id, durable_state_id,
                   runtime_artifact_id, schema_version, backend, locator,
                   zip_bytes, zip_sha256, manifest_sha256, created_at,
                   verified_at, recovery_authority_id, retention_policy, stored_at
                 ) VALUES (
                   $1, $2, $3, $4, $5, $6, $7, $8, $9,
                   $10, $11, $12, $13,
                   $14, $15, $16::text::timestamptz
                 ) ON CONFLICT (request_id) DO NOTHING",
                &[
                    &receipt.request_id,
                    &receipt.project_id,
                    &receipt.agent_runtime_id,
                    &receipt.durable_state_id,
                    &receipt.runtime_artifact_id,
                    &receipt.schema,
                    &receipt.backend,
                    &receipt.locator,
                    &zip_bytes,
                    &receipt.zip_sha256,
                    &receipt.manifest_sha256,
                    &receipt.created_at,
                    &receipt.verified_at,
                    &receipt.recovery_authority_id,
                    &receipt.retention_policy,
                    &snapshot.stored_at,
                ],
            )
            .await
            .map_err(store_error)?;
        if inserted != 1 {
            return Err(CoreError::RuntimeRetirementSnapshotConflict);
        }
        // The verified receipt is now durably stored; record the phase in the
        // same transaction as the insert.
        set_offboarding_phase(
            client,
            &locked.agent_runtime_id,
            OffboardingPhase::ReceiptVerified,
            &now,
        )
        .await?;
    }
    // Drive the canonical lifecycle machine to its terminal. Up-bound
    // operations pass through ComputeUp and Ready before Succeeded: the
    // Runner only calls complete after its bounded readiness wait returned
    // ready, so the chain is recorded atomically here. (Persisting ComputeUp
    // and Ready as separately observable writes lands with the readiness
    // transport follow-up; the ordering invariant is already enforced by the
    // machine.) Down-bound operations confirm straight into Stopped.
    let launching =
        runtime_lifecycle::RuntimeLifecycle::<runtime_lifecycle::phase::Launching>::from_status(
            locked.status,
        )
        .ok_or(CoreError::RuntimeControlRequestNotLaunching)?;
    let terminal_status = match locked.kind {
        RuntimeControlKind::Restart
        | RuntimeControlKind::RecoverKnownGoodChatRuntime
        | RuntimeControlKind::Upgrade => {
            launching.compute_up(&completion).ready().succeed().status()
        }
        RuntimeControlKind::Stop | RuntimeControlKind::Destroy => {
            launching.confirm_stopped(&completion).status()
        }
    };
    let row = client
        .query_one(
            "UPDATE runtime_control_requests
             SET status = $3,
                 lease_token = NULL,
                 lease_expires_at = NULL,
                 failure_message = NULL,
                 updated_at = $2::text::timestamptz,
                 completed_at = $2::text::timestamptz
             WHERE id = $1
             RETURNING id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                       requested_by_user_id, kind, target_runtime_artifact_id, status,
                       failure_stage, runner_id, lease_token,
                       core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message, core_rfc3339(created_at) AS created_at,
                       core_rfc3339(updated_at) AS updated_at, core_rfc3339(completed_at) AS completed_at",
            &[&input.request_id, &now, &terminal_status.as_str()],
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
        // A runner only completes a destroy after its verified readback,
        // canonical container removal, and staging cleanup, so the committed
        // completion is the compute-removed record.
        set_offboarding_phase(
            client,
            &request.agent_runtime_id,
            OffboardingPhase::ComputeRemoved,
            &now,
        )
        .await?;
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
    // N-1 Runners do not name a stage; their failures record `unknown`
    // rather than blocking the failure write.
    let failure_stage = input
        .failure_stage
        .unwrap_or(RuntimeLifecycleStage::Unknown);
    let locked = locked_runtime_control_request(client, &input.request_id).await?;
    verify_runtime_control_lease(&locked, &input.runner_id, &input.lease_token)?;
    let row = client
        .query_one(
            "UPDATE runtime_control_requests
             SET status = 'failed',
                 failure_stage = $4,
                 lease_token = NULL,
                 lease_expires_at = NULL,
                 failure_message = $2,
                 updated_at = $3::text::timestamptz,
                 completed_at = $3::text::timestamptz
             WHERE id = $1
             RETURNING id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                       requested_by_user_id, kind, target_runtime_artifact_id, status,
                       failure_stage, runner_id, lease_token,
                       core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message, core_rfc3339(created_at) AS created_at,
                       core_rfc3339(updated_at) AS updated_at, core_rfc3339(completed_at) AS completed_at",
            &[&input.request_id, &failure_message, &now, &failure_stage.as_str()],
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
    Ok(request)
}

async fn postgres_renew_runtime_control_request<C>(
    client: &C,
    input: RenewRuntimeControlRequestInput,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let now_time = parse_time(&now)?;
    let lease_seconds = input
        .lease_seconds
        .unwrap_or(crate::DEFAULT_AGENT_CREATION_LEASE_SECONDS);
    if !(1..=crate::MAX_AGENT_CREATION_LEASE_SECONDS).contains(&lease_seconds) {
        return Err(CoreError::InvalidAgentCreationLeaseDuration);
    }
    let locked = locked_runtime_control_request(client, &input.request_id).await?;
    verify_postgres_runtime_control_lease_at(
        client,
        &locked,
        &input.runner_id,
        &input.lease_token,
        &now,
    )
    .await?;
    let lease_expires_at = (now_time + Duration::seconds(lease_seconds)).format(&Rfc3339)?;
    let row = client
        .query_one(
            "UPDATE runtime_control_requests
             SET lease_expires_at = $2::text::timestamptz,
                 updated_at = $3::text::timestamptz
             WHERE id = $1
             RETURNING id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                       requested_by_user_id, kind, target_runtime_artifact_id, status,
                       failure_stage, runner_id, lease_token, core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at, core_rfc3339(completed_at) AS completed_at",
            &[&input.request_id, &lease_expires_at, &now],
        )
        .await
        .map_err(store_error)?;
    runtime_control_request_from_row(&row)
}

async fn postgres_retry_runtime_control_request<C>(
    client: &C,
    input: RetryRuntimeControlRequestInput,
) -> CoreResult<RuntimeControlRequest>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let failure_message = trim_to_option(Some(&input.failure_message))
        .ok_or(CoreError::MissingRuntimeControlFailureMessage)?;
    let locked = locked_runtime_control_request(client, &input.request_id).await?;
    verify_postgres_runtime_control_lease_at(
        client,
        &locked,
        &input.runner_id,
        &input.lease_token,
        &now,
    )
    .await?;
    if locked.kind != RuntimeControlKind::Destroy {
        return Err(CoreError::RuntimeControlOperationConflict);
    }
    let row = client
        .query_one(
            "UPDATE runtime_control_requests
             SET status = 'requested', failure_stage = 'unknown',
                 runner_id = NULL, lease_token = NULL,
                 lease_expires_at = NULL, failure_message = $2,
                 updated_at = $3::text::timestamptz, completed_at = NULL
             WHERE id = $1
             RETURNING id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                       requested_by_user_id, kind, target_runtime_artifact_id, status,
                       failure_stage, runner_id, lease_token, core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at, core_rfc3339(completed_at) AS completed_at",
            &[&input.request_id, &failure_message, &now],
        )
        .await
        .map_err(store_error)?;
    let request = runtime_control_request_from_row(&row)?;
    if let Some(mut runtime) = select_agent_runtime(client, &request.agent_runtime_id).await? {
        runtime.host_facts.runtime_status = RuntimeSummaryStatus::Stale;
        runtime.updated_at = now.clone();
        upsert_agent_runtime_row(client, &runtime).await?;
    }
    Ok(request)
}

/// Read the runtime's recorded offboarding phase. The callers that mutate
/// phases already hold the runtime row locked inside their transaction.
async fn postgres_offboarding_phase<C>(
    client: &C,
    agent_runtime_id: &str,
) -> CoreResult<Option<OffboardingPhase>>
where
    C: GenericClient + Sync,
{
    let phase: Option<String> = client
        .query_opt(
            "SELECT offboarding_phase FROM agent_runtimes WHERE id = $1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?
        .ok_or(CoreError::ProjectRuntimeNotFound)?
        .get("offboarding_phase");
    phase
        .as_deref()
        .map(|value| {
            parse_offboarding_phase(value)
                .ok_or_else(|| CoreError::Store(format!("invalid offboarding phase {value}")))
        })
        .transpose()
}

/// Advance the runtime's offboarding phase strictly forward, in the same
/// transaction as the side effect the phase records. Restating the current
/// phase is an idempotent no-op (replayed completions); any backward move
/// fails closed and names both phases.
async fn set_offboarding_phase<C>(
    client: &C,
    agent_runtime_id: &str,
    phase: OffboardingPhase,
    now: &str,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let updated = client
        .execute(
            "UPDATE agent_runtimes
             SET offboarding_phase = $2, updated_at = $3::text::timestamptz
             WHERE id = $1
               AND (
                 offboarding_phase IS NULL
                 OR offboarding_phase = $2
                 OR array_position(
                      ARRAY['retirement_requested', 'receipt_verified', 'compute_removed',
                            'link_deactivated', 'archived']::text[],
                      offboarding_phase
                    ) < array_position(
                      ARRAY['retirement_requested', 'receipt_verified', 'compute_removed',
                            'link_deactivated', 'archived']::text[],
                      $2
                    )
               )",
            &[&agent_runtime_id, &phase.as_str(), &now],
        )
        .await
        .map_err(store_error)?;
    if updated == 1 {
        return Ok(());
    }
    let current = postgres_offboarding_phase(client, agent_runtime_id)
        .await?
        .ok_or_else(|| {
            CoreError::Store(format!(
                "runtime {agent_runtime_id} rejected the forward-only offboarding phase update with no recorded phase"
            ))
        })?;
    Err(CoreError::OffboardingPhaseRegression {
        current,
        attempted: phase,
    })
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
    postgres_offboard_runtime(
        client,
        &request.project_id,
        &request.agent_runtime_id,
        now,
        "finite_private.runtime.destroy_revoke_keys",
        None,
    )
    .await?;
    set_offboarding_phase(
        client,
        &request.agent_runtime_id,
        OffboardingPhase::Archived,
        now,
    )
    .await?;
    Ok(())
}

async fn postgres_offboard_runtime<C>(
    client: &C,
    project_id: &str,
    agent_runtime_id: &str,
    now: &str,
    revocation_action: &'static str,
    actor: Option<&str>,
) -> CoreResult<Vec<String>>
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
            &[&project_id, &now],
        )
        .await
        .map_err(store_error)?;
    client
        .execute(
            "UPDATE project_runtime_links SET active = FALSE WHERE agent_runtime_id = $1",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?;
    // The link deactivation above is the offboarding boundary; record it in
    // the same transaction.
    set_offboarding_phase(
        client,
        agent_runtime_id,
        OffboardingPhase::LinkDeactivated,
        now,
    )
    .await?;
    client
        .execute(
            "DELETE FROM runtime_relay_credentials WHERE agent_runtime_id = $1",
            &[&agent_runtime_id],
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
            &[&agent_runtime_id, &project_id, &now],
        )
        .await
        .map_err(store_error)?;
    let revoked_api_key_ids: Vec<String> = revoked_rows.iter().map(|row| row.get("id")).collect();
    if !revoked_api_key_ids.is_empty() {
        insert_finite_private_admin_audit_event(
            client,
            FinitePrivateAdminAuditInsert {
                action: revocation_action,
                target_type: "agent_runtime",
                target_id: agent_runtime_id,
                grant_id: None,
                api_key_id: None,
                actor,
                metadata: json!({
                    "projectId": project_id,
                    "revokedApiKeyIds": revoked_api_key_ids,
                }),
                now,
            },
        )
        .await?;
    }
    Ok(revoked_api_key_ids)
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
                    core_rfc3339(created_at) AS created_at, core_rfc3339(promoted_at) AS promoted_at, core_rfc3339(retired_at) AS retired_at
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
                       core_rfc3339(created_at) AS created_at, core_rfc3339(promoted_at) AS promoted_at, core_rfc3339(retired_at) AS retired_at",
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

async fn postgres_admin_runtime_overviews<C>(client: &C) -> CoreResult<Vec<AdminRuntimeOverview>>
where
    C: GenericClient + Sync,
{
    let now = current_time_iso()?;
    let rows = client
        .query(
            "SELECT runtime.id AS agent_runtime_id, runtime.project_id, runtime.source_host_id,
                    runtime.source_machine_id, runtime.runtime_artifact_id, runtime.host_facts,
                    runtime.offboarding_phase,
                    core_rfc3339(runtime.updated_at) AS runtime_updated_at,
                    project.display_name AS project_display_name,
                    owner.normalized_email AS owner_email,
                    artifact.version_label AS runtime_artifact_version_label,
                    runtime.runtime_capabilities,
                    core_rfc3339(runtime.health_reported_at) AS health_reported_at,
                    core_rfc3339(runtime.health_observed_at) AS health_observed_at,
                    runtime.health_ready,
                    runtime.health_reason,
                    runtime.health_report_interval_seconds,
                    runtime.health_reporting_npub,
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
             LEFT JOIN runtime_artifacts AS artifact ON artifact.id = runtime.runtime_artifact_id
             ORDER BY runtime.source_host_id, runtime.source_machine_id, runtime.id",
            &[],
        )
        .await
        .map_err(store_error)?;
    rows.iter()
        .map(|row| {
            let host_facts: HostOwnedRuntimeFacts = json_column(row, "host_facts")?;
            let runtime_capabilities: Option<RuntimeCapabilitiesEnvelope> =
                optional_json_column(row, "runtime_capabilities")?
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(json_error)?;
            let offboarding_phase: Option<String> = row.get("offboarding_phase");
            let offboarding_phase = offboarding_phase
                .as_deref()
                .map(|value| {
                    parse_offboarding_phase(value).ok_or_else(|| {
                        CoreError::Store(format!("invalid offboarding phase {value}"))
                    })
                })
                .transpose()?;
            let project_display_name: Option<String> = row.get("project_display_name");
            let runtime_health = project_runtime_health(
                host_facts.runtime_status,
                &StoredRuntimeHealth {
                    reported_at: row.get("health_reported_at"),
                    observed_at: row.get("health_observed_at"),
                    ready: row.get("health_ready"),
                    reason: row.get("health_reason"),
                    report_interval_seconds: row
                        .get::<_, Option<i32>>("health_report_interval_seconds")
                        .map(i64::from),
                    reporting_npub: row.get("health_reporting_npub"),
                },
                &now,
            )?;
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
                runtime_status: host_facts.runtime_status,
                // runtime_status_snapshots has no writer; the wire fields stay
                // serialized as null for dashboard compatibility until the
                // gated table drop and wire-type change land together.
                last_heartbeat_at: None,
                status_updated_at: None,
                runtime_updated_at: row.get("runtime_updated_at"),
                hermes_available: host_facts.hermes_available,
                published_app_urls: host_facts.published_app_urls.clone(),
                active_finite_private_key_count: row.get("active_finite_private_key_count"),
                runtime_link_active: row.get("runtime_link_active"),
                runtime_capabilities: runtime_capabilities
                    .as_ref()
                    .map(|capabilities| *capabilities.v1()),
                offboarding_phase,
                runtime_health,
            })
        })
        .collect()
}

/// Record one runner-ferried standing-readiness report on the runtime row.
/// The source host comes from the runner credential and scopes the UPDATE, so
/// a body naming another host's runtime (or an unknown runtime) misses every
/// row and fails closed as not-found without leaking cross-host existence.
async fn postgres_record_runtime_health_report<C>(
    client: &C,
    input: RecordRuntimeHealthReportInput,
) -> CoreResult<RuntimeHealthReportAck>
where
    C: GenericClient + Sync,
{
    let now = input.now.clone().unwrap_or(current_time_iso()?);
    let agent_runtime_id =
        trim_to_option(Some(&input.agent_runtime_id)).ok_or(CoreError::MissingAgentRuntimeId)?;
    let source_host_id =
        trim_to_option(Some(&input.source_host_id)).ok_or(CoreError::MissingSourceHostId)?;
    let reason = trim_to_option(input.reason.as_deref());
    if reason
        .as_ref()
        .is_some_and(|value| value.chars().count() > MAX_RUNTIME_HEALTH_REPORT_REASON_CHARS)
    {
        return Err(CoreError::InvalidRuntimeHealthReport);
    }
    // The observation time is runner-clock evidence; it must still parse.
    parse_time(&input.observed_at)?;
    let agent_npub = trim_to_option(input.agent_npub.as_deref());
    if agent_npub
        .as_ref()
        .is_some_and(|value| !valid_agent_npub(value))
    {
        return Err(CoreError::InvalidRuntimeHealthReport);
    }
    let interval_seconds = input.report_interval_seconds;
    if interval_seconds.is_some_and(|value| {
        !(RUNTIME_HEALTH_REPORT_MIN_INTERVAL_SECONDS..=RUNTIME_HEALTH_REPORT_MAX_INTERVAL_SECONDS)
            .contains(&value)
    }) {
        return Err(CoreError::InvalidRuntimeHealthReport);
    }
    let interval_seconds = interval_seconds
        .map(i32::try_from)
        .transpose()
        .map_err(|_| CoreError::InvalidRuntimeHealthReport)?;
    let row = client
        .query_opt(
            "UPDATE agent_runtimes
             SET health_reported_at = $3::text::timestamptz,
                 health_observed_at = $4::text::timestamptz,
                 health_ready = $5,
                 health_reason = $6,
                 health_report_interval_seconds = $7,
                 health_reporting_npub = $8
             WHERE id = $1 AND source_host_id = $2
             RETURNING id",
            &[
                &agent_runtime_id,
                &source_host_id,
                &now,
                &input.observed_at,
                &input.ready,
                &reason,
                &interval_seconds,
                &agent_npub,
            ],
        )
        .await
        .map_err(store_error)?;
    let Some(row) = row else {
        return Err(CoreError::ProjectRuntimeNotFound);
    };
    Ok(RuntimeHealthReportAck {
        agent_runtime_id: row.get("id"),
        recorded_at: now,
    })
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
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
            &[&user_id, &email, &now],
        )
        .await
        .map_err(store_error)?;
    core_user_from_row(&row)
}

/// RFC3339 rendering for a TIMESTAMPTZ column so stored strings round-trip
/// through `parse_time` (the Finite Private timestamps are parsed, not just
/// echoed).
fn rfc3339_col(expr: &str) -> String {
    format!("core_rfc3339({expr})")
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
        burst_window_epoch: row.get("burst_window_epoch"),
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
                current_window_used_units, burst_window_epoch,
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
                upstream_error_class, burst_window_epoch,
                {created} AS created_at, {updated} AS updated_at
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
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
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
             RETURNING id, user_id, limit_profile_id, status,
                       core_rfc3339(current_window_started_at) AS current_window_started_at,
                       current_window_used_units, burst_window_epoch,
                       core_rfc3339(created_at) AS created_at,
                       core_rfc3339(updated_at) AS updated_at",
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
             SET current_window_started_at = $2::text::timestamptz,
                 current_window_used_units = 0,
                 burst_window_epoch = burst_window_epoch + 1,
                 updated_at = $2::text::timestamptz
             WHERE id = $1
             RETURNING id, user_id, limit_profile_id, status,
                       core_rfc3339(current_window_started_at) AS current_window_started_at,
                       current_window_used_units, burst_window_epoch,
                       core_rfc3339(created_at) AS created_at,
                       core_rfc3339(updated_at) AS updated_at",
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
                    core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
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

/// Approve a grant and issue its first key against one client.
///
/// Callers pass a transaction, so the pair is atomic: no orphaned grant on a
/// failed key issue, and a dry run can preview both steps.
async fn postgres_issue_finite_private_friend_key<C>(
    client: &C,
    input: IssueFinitePrivateFriendKeyInput,
) -> CoreResult<IssuedFinitePrivateFriendKey>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let grant = postgres_approve_finite_private_grant(
        client,
        ApproveFinitePrivateGrantInput {
            verified_email: input.verified_email,
            workos_user_id: input.workos_user_id,
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
            project_id: input.project_id,
            agent_runtime_id: input.agent_runtime_id,
            now: Some(now),
        },
    )
    .await?;
    Ok(IssuedFinitePrivateFriendKey { grant, api_key })
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

async fn postgres_admin_assign_finite_private_limit_profile<C>(
    client: &C,
    input: AdminAssignFinitePrivateLimitProfileInput,
) -> CoreResult<FinitePrivateGrant>
where
    C: GenericClient + Sync,
{
    let now = input.now.unwrap_or(current_time_iso()?);
    let admin_email = normalize_owner_email(Some(&input.admin_verified_email))
        .ok_or(CoreError::MissingVerifiedEmail)?;
    let grant_id =
        trim_to_option(Some(&input.grant_id)).ok_or(CoreError::FinitePrivateGrantNotFound)?;
    let limit_profile_id = trim_to_option(Some(&input.limit_profile_id))
        .ok_or(CoreError::FinitePrivateLimitProfileNotFound)?;
    ensure_finite_private_limit_profile_row(client, &limit_profile_id, &now).await?;
    let previous = select_finite_private_grant(client, &grant_id, true)
        .await?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    client
        .execute(
            "UPDATE finite_private_grants
             SET limit_profile_id = $2, updated_at = $3::text::timestamptz
             WHERE id = $1",
            &[&grant_id, &limit_profile_id, &now],
        )
        .await
        .map_err(store_error)?;
    let grant = select_finite_private_grant(client, &grant_id, false)
        .await?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    insert_finite_private_admin_audit_event(
        client,
        FinitePrivateAdminAuditInsert {
            action: "finite_private.grant.admin_assign_limit_profile",
            target_type: "grant",
            target_id: &grant.id,
            grant_id: Some(&grant.id),
            api_key_id: None,
            actor: Some(&admin_email),
            metadata: json!({
                "previousLimitProfileId": previous.limit_profile_id,
                "limitProfileId": grant.limit_profile_id.clone(),
            }),
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
    let (weekly_used_units, weekly_reset_at) = if profile.weekly_limit_units.is_some() {
        let window_start = (now_time
            - Duration::seconds(crate::FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS))
        .format(&Rfc3339)?;
        postgres_finite_private_weekly_usage(client, &grant.id, &window_start, &now).await?
    } else {
        // The shipped profiles have no rolling weekly limit. Avoid scanning
        // the reservation ledger when its result cannot affect admission or
        // the public response.
        (0, None)
    };

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
    let begins_new_epoch = crate::finite_private_begins_new_epoch(&grant, &window_started_at)?;
    let reservation_epoch = grant.burst_window_epoch + i64::from(begins_new_epoch);
    let remaining_before = profile.burst_limit_units - current_used_units;
    if input.estimated_usage_units > remaining_before {
        let retry_after = (parse_time(&reset_at)? - now_time).whole_seconds().max(0);
        let message =
            crate::finite_private_limit_reached_message("burst window", &reset_at, retry_after);
        return Ok(crate::finite_private_denial(
            request_id,
            dashboard_url,
            &message,
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
            let message =
                crate::finite_private_limit_reached_message("weekly", &reset_at, retry_after);
            return Ok(crate::finite_private_denial(
                request_id,
                dashboard_url,
                &message,
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
                 burst_window_epoch = $4,
                 updated_at = $5::text::timestamptz
             WHERE id = $1",
            &[
                &grant.id,
                &window_started_at,
                &new_used_units,
                &reservation_epoch,
                &now,
            ],
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
               upstream_error_class, burst_window_epoch, created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7, NULL, NULL, 'reserved', $8, NULL, NULL,
                     $9, $10::text::timestamptz, $10::text::timestamptz)",
            &[
                &reservation_id,
                &request_id,
                &api_key.id,
                &grant.id,
                &endpoint,
                &model,
                &input.estimated_usage_units,
                &usage_formula_version,
                &reservation_epoch,
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
    let settled_units = input
        .usage_units
        .unwrap_or(existing.reserved_usage_units)
        .max(0);
    if existing.status == FinitePrivateReservationStatus::Settled {
        let formula = crate::trim_or_fallback(
            &input.usage_formula_version,
            &existing.usage_formula_version,
        );
        if existing.settled_usage_units == Some(settled_units)
            && existing.settlement_kind == Some(input.settlement)
            && existing.usage_formula_version == formula
            && existing.upstream_status == input.upstream_status
            && existing.upstream_error_class
                == trim_to_option(input.upstream_error_class.as_deref())
        {
            return Ok(SettleFinitePrivateReservationResult {
                settled: true,
                reservation_id,
            });
        }
        return Err(CoreError::FinitePrivateReservationAlreadySettled);
    }
    let delta = settled_units - existing.reserved_usage_units;
    // Adjust the grant's burst usage by the settle delta (clamped at 0).
    client
        .execute(
            "UPDATE finite_private_grants
             SET current_window_used_units = GREATEST(current_window_used_units + $2, 0),
                 updated_at = $3::text::timestamptz
             WHERE id = $1 AND burst_window_epoch = $4",
            &[
                &existing.grant_id,
                &delta,
                &now,
                &existing.burst_window_epoch,
            ],
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

async fn postgres_finite_private_grant_id_for_workos_user<C>(
    client: &C,
    workos_user_id: &str,
) -> CoreResult<Option<String>>
where
    C: GenericClient + Sync,
{
    Ok(client
        .query_opt(
            "SELECT fpg.id
             FROM finite_private_grants fpg
             JOIN users usr ON usr.id = fpg.user_id
             WHERE usr.workos_user_id = $1 AND fpg.status = 'active'",
            &[&workos_user_id],
        )
        .await
        .map_err(store_error)?
        .map(|row| row.get("id")))
}

async fn postgres_finite_private_usage_status_for_api_key<C>(
    client: &C,
    presented_api_key: &str,
    claim_notice: bool,
    now: Option<String>,
) -> CoreResult<Option<FinitePrivateUsageStatus>>
where
    C: GenericClient + Sync,
{
    let Some((_, grant)) = postgres_finite_private_key_and_grant(client, presented_api_key).await?
    else {
        return Ok(None);
    };
    postgres_finite_private_usage_status_for_grant(client, &grant.id, claim_notice, now)
        .await
        .map(Some)
}

async fn postgres_finite_private_usage_status_for_workos_user<C>(
    client: &C,
    workos_user_id: &str,
    now: Option<String>,
) -> CoreResult<Option<FinitePrivateUsageStatus>>
where
    C: GenericClient + Sync,
{
    let Some(grant_id) =
        postgres_finite_private_grant_id_for_workos_user(client, workos_user_id).await?
    else {
        return Ok(None);
    };
    postgres_finite_private_usage_status_for_grant(client, &grant_id, false, now)
        .await
        .map(Some)
}

async fn postgres_finite_private_usage_status_for_grant<C>(
    client: &C,
    grant_id: &str,
    claim_notice: bool,
    now: Option<String>,
) -> CoreResult<FinitePrivateUsageStatus>
where
    C: GenericClient + Sync,
{
    let now = now.unwrap_or(current_time_iso()?);
    let now_time = parse_time(&now)?;
    let grant = select_finite_private_grant(client, grant_id, false)
        .await?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    if grant.status != FinitePrivateGrantStatus::Active {
        return Err(CoreError::FinitePrivateGrantNotActive);
    }
    let profile = select_finite_private_limit_profile(client, &grant.limit_profile_id)
        .await?
        .ok_or(CoreError::FinitePrivateLimitProfileNotFound)?;
    let (window_started_at, current_used_units, reset_at) =
        crate::finite_private_active_window(&grant, &profile, now_time)?;
    let begins_new_epoch = crate::finite_private_begins_new_epoch(&grant, &window_started_at)?;
    let epoch = grant.burst_window_epoch + i64::from(begins_new_epoch);
    let settled_used_units: i64 = client
        .query_one(
            "SELECT COALESCE(SUM(settled_usage_units), 0)::bigint AS used_units
             FROM finite_private_reservations
             WHERE grant_id = $1
               AND burst_window_epoch = $2
               AND status = 'settled'
               AND created_at >= $3::text::timestamptz",
            &[&grant.id, &epoch, &window_started_at],
        )
        .await
        .map_err(store_error)?
        .get("used_units");
    let notice = if claim_notice {
        postgres_claim_finite_private_usage_notice(
            client,
            &grant.id,
            epoch,
            settled_used_units,
            &profile,
            &reset_at,
            &now,
        )
        .await?
    } else {
        None
    };
    let daily_reset_used = client
        .query_opt(
            "SELECT 1
             FROM finite_private_daily_resets
             WHERE grant_id = $1
               AND reset_day = ($2::text::timestamptz AT TIME ZONE 'UTC')::date",
            &[&grant.id, &now],
        )
        .await
        .map_err(store_error)?
        .is_some();
    Ok(FinitePrivateUsageStatus {
        burst_limit_units: profile.burst_limit_units,
        burst_used_units: current_used_units.max(0),
        burst_remaining_units: (profile.burst_limit_units - current_used_units).max(0),
        burst_reset_at: reset_at,
        free_daily_reset_available: !daily_reset_used,
        free_daily_reset_available_again_at: crate::finite_private_next_daily_reset_at(now_time)?,
        notice,
    })
}

async fn postgres_claim_finite_private_usage_notice<C>(
    client: &C,
    grant_id: &str,
    epoch: i64,
    settled_used_units: i64,
    profile: &FinitePrivateLimitProfile,
    reset_at: &str,
    now: &str,
) -> CoreResult<Option<FinitePrivateUsageNotice>>
where
    C: GenericClient + Sync,
{
    let remaining = (profile.burst_limit_units - settled_used_units).max(0);
    let threshold: Option<i16> =
        if i128::from(remaining) * 100 <= i128::from(profile.burst_limit_units) * 10 {
            Some(10)
        } else if i128::from(remaining) * 100 <= i128::from(profile.burst_limit_units) * 25 {
            Some(25)
        } else {
            None
        };
    let Some(threshold) = threshold else {
        return Ok(None);
    };
    if threshold == 10 {
        client
            .execute(
                "INSERT INTO finite_private_notice_claims (
                   grant_id, burst_window_epoch, threshold_remaining_percent, claimed_at
                 ) VALUES ($1, $2, 25, $3::text::timestamptz)
                 ON CONFLICT DO NOTHING",
                &[&grant_id, &epoch, &now],
            )
            .await
            .map_err(store_error)?;
    }
    let claimed = client
        .query_opt(
            "INSERT INTO finite_private_notice_claims (
               grant_id, burst_window_epoch, threshold_remaining_percent, claimed_at
             ) VALUES ($1, $2, $3, $4::text::timestamptz)
             ON CONFLICT DO NOTHING
             RETURNING threshold_remaining_percent",
            &[&grant_id, &epoch, &threshold, &now],
        )
        .await
        .map_err(store_error)?
        .is_some();
    if !claimed {
        return Ok(None);
    }
    let retry_after = (parse_time(reset_at)? - parse_time(now)?)
        .whole_seconds()
        .max(0);
    Ok(Some(FinitePrivateUsageNotice {
        threshold_remaining_percent: i64::from(threshold),
        message: format!(
            "You have {threshold}% of your Finite Private burst limit remaining. Your usage resets at {reset_at} ({}).",
            crate::finite_private_retry_after_label(retry_after)
        ),
    }))
}

async fn postgres_claim_finite_private_daily_reset_for_api_key<C>(
    client: &C,
    presented_api_key: &str,
    now: Option<String>,
) -> CoreResult<FinitePrivateDailyResetResult>
where
    C: GenericClient + Sync,
{
    let Some((_, grant)) = postgres_finite_private_key_and_grant(client, presented_api_key).await?
    else {
        return Err(CoreError::InvalidFinitePrivateApiKey);
    };
    postgres_claim_finite_private_daily_reset_for_grant(client, &grant.id, now).await
}

async fn postgres_claim_finite_private_daily_reset_for_workos_user<C>(
    client: &C,
    workos_user_id: &str,
    now: Option<String>,
) -> CoreResult<Option<FinitePrivateDailyResetResult>>
where
    C: GenericClient + Sync,
{
    let Some(grant_id) =
        postgres_finite_private_grant_id_for_workos_user(client, workos_user_id).await?
    else {
        return Ok(None);
    };
    postgres_claim_finite_private_daily_reset_for_grant(client, &grant_id, now)
        .await
        .map(Some)
}

async fn postgres_claim_finite_private_daily_reset_for_grant<C>(
    client: &C,
    grant_id: &str,
    now: Option<String>,
) -> CoreResult<FinitePrivateDailyResetResult>
where
    C: GenericClient + Sync,
{
    let now = now.unwrap_or(current_time_iso()?);
    let grant = select_finite_private_grant(client, grant_id, true)
        .await?
        .ok_or(CoreError::FinitePrivateGrantNotFound)?;
    if grant.status != FinitePrivateGrantStatus::Active {
        return Err(CoreError::FinitePrivateGrantNotActive);
    }
    let performed = client
        .query_opt(
            "INSERT INTO finite_private_daily_resets (grant_id, reset_day, claimed_at)
             VALUES (
               $1,
               ($2::text::timestamptz AT TIME ZONE 'UTC')::date,
               $2::text::timestamptz
             )
             ON CONFLICT DO NOTHING
             RETURNING grant_id",
            &[&grant_id, &now],
        )
        .await
        .map_err(store_error)?
        .is_some();
    if performed {
        client
            .execute(
                "UPDATE finite_private_grants
                 SET current_window_started_at = $2::text::timestamptz,
                     current_window_used_units = 0,
                     burst_window_epoch = burst_window_epoch + 1,
                     updated_at = $2::text::timestamptz
                 WHERE id = $1",
                &[&grant_id, &now],
            )
            .await
            .map_err(store_error)?;
    }
    let status =
        postgres_finite_private_usage_status_for_grant(client, grant_id, false, Some(now)).await?;
    Ok(FinitePrivateDailyResetResult { performed, status })
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
        "SELECT fp_grant.id, fp_grant.user_id, fp_grant.limit_profile_id, fp_grant.status,
                CASE WHEN fp_grant.current_window_started_at IS NULL THEN NULL
                     ELSE {started} END AS current_window_started_at,
                fp_grant.current_window_used_units, fp_grant.burst_window_epoch,
                {created} AS created_at, {updated} AS updated_at,
                account.normalized_email
         FROM finite_private_grants AS fp_grant
         JOIN users AS account ON account.id = fp_grant.user_id
         ORDER BY fp_grant.created_at, fp_grant.id",
        started = rfc3339_col("fp_grant.current_window_started_at"),
        created = rfc3339_col("fp_grant.created_at"),
        updated = rfc3339_col("fp_grant.updated_at"),
    );
    let grant_rows = client.query(&grant_sql, &[]).await.map_err(store_error)?;
    let grants = grant_rows
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
    let profile_sql = format!(
        "SELECT id, burst_window_seconds, burst_limit_units, weekly_limit_units,
                {created} AS created_at, {updated} AS updated_at
         FROM finite_private_limit_profiles
         ORDER BY id",
        created = rfc3339_col("created_at"),
        updated = rfc3339_col("updated_at"),
    );
    let profiles = client
        .query(&profile_sql, &[])
        .await
        .map_err(store_error)?
        .iter()
        .map(finite_private_limit_profile_from_row)
        .collect::<Vec<_>>();
    let project_rows = client
        .query(
            "SELECT project.id, project.owner_user_id, project.display_name,
                    link.agent_runtime_id
             FROM projects AS project
             JOIN finite_private_grants AS fp_grant
               ON fp_grant.user_id = project.owner_user_id
             LEFT JOIN project_runtime_links AS link
               ON link.project_id = project.id AND link.active = TRUE
             ORDER BY project.display_name, project.id",
            &[],
        )
        .await
        .map_err(store_error)?;
    let mut projects_by_user = BTreeMap::<String, Vec<FinitePrivateAdminProject>>::new();
    for row in project_rows {
        projects_by_user
            .entry(row.get("owner_user_id"))
            .or_default()
            .push(FinitePrivateAdminProject {
                id: row.get("id"),
                display_name: row.get("display_name"),
                agent_runtime_id: row.get("agent_runtime_id"),
            });
    }
    let mut accounts = grant_rows
        .iter()
        .zip(grants.iter())
        .map(|(row, grant)| FinitePrivateAdminAccount {
            user_id: grant.user_id.clone(),
            email: row.get("normalized_email"),
            grant: grant.clone(),
            api_keys: api_keys
                .iter()
                .filter(|key| key.grant_id == grant.id)
                .cloned()
                .collect(),
            projects: projects_by_user.remove(&grant.user_id).unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    accounts.sort_by(|left, right| {
        left.email
            .cmp(&right.email)
            .then_with(|| left.user_id.cmp(&right.user_id))
    });
    let admin_audit_events = postgres_finite_private_admin_audit_events(client).await?;
    Ok(FinitePrivateAdminState {
        accounts,
        profiles,
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

/// Typed row reads used only by tests.
///
/// The production store API is task-shaped (request / lease / complete), so
/// tests that used to inspect `BridgeCoreState`'s public maps have no reader
/// for a bare row. These reuse the production `select_*` + `*_from_row` pair,
/// so a test decodes a row exactly the way production does — including the
/// column list and timestamp rendering.
///
/// List readers select ids and then reuse the per-id reader rather than
/// duplicating each entity's column list, which would drift.
#[cfg(test)]
impl CoreStore {
    async fn ids(&self, table: &str) -> Vec<String> {
        self.ids_by(table, "id").await
    }

    /// Primary keys of `table`, for tables whose key column is not `id`.
    async fn ids_by(&self, table: &str, key: &str) -> Vec<String> {
        let client = self.connection().await.unwrap();
        client
            .query(
                &format!("SELECT {key} AS id FROM {table} ORDER BY {key}"),
                &[],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.get::<_, String>("id"))
            .collect()
    }

    pub(crate) async fn agent_runtime(&self, id: &str) -> Option<AgentRuntime> {
        let client = self.connection().await.unwrap();
        select_agent_runtime(&**client, id).await.unwrap()
    }

    pub(crate) async fn all_agent_runtimes(&self) -> Vec<AgentRuntime> {
        let mut out = Vec::new();
        for id in self.ids("agent_runtimes").await {
            out.push(self.agent_runtime(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn user_by_email(&self, email: &str) -> Option<CoreUser> {
        let client = self.connection().await.unwrap();
        select_user_by_email(&**client, email).await.unwrap()
    }

    pub(crate) async fn personal_org_by_owner(
        &self,
        owner_user_id: &str,
    ) -> Option<CustomerOrganization> {
        let client = self.connection().await.unwrap();
        select_personal_org_by_owner(&**client, owner_user_id)
            .await
            .unwrap()
    }

    pub(crate) async fn project(&self, id: &str) -> Option<Project> {
        let client = self.connection().await.unwrap();
        select_project(&**client, id).await.unwrap()
    }

    pub(crate) async fn all_projects(&self) -> Vec<Project> {
        let mut out = Vec::new();
        for id in self.ids("projects").await {
            out.push(self.project(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn finite_private_grant(&self, id: &str) -> Option<FinitePrivateGrant> {
        let client = self.connection().await.unwrap();
        select_finite_private_grant(&**client, id, false)
            .await
            .unwrap()
    }

    pub(crate) async fn runtime_artifact_row(&self, id: &str) -> Option<RuntimeArtifact> {
        let client = self.connection().await.unwrap();
        select_runtime_artifact(&**client, id).await.unwrap()
    }

    /// Column list mirrors the production lease/read queries so a test decodes
    /// the row exactly as production does.
    pub(crate) async fn agent_creation_request(&self, id: &str) -> Option<AgentCreationRequest> {
        let client = self.connection().await.unwrap();
        client
            .query_opt(
                "SELECT id, customer_org_id, owner_user_id, project_id, idempotency_key,
                        display_name, runner_class, hosting_tier, placement_runner_class,
                        runtime_resource_class, desired_runtime_artifact_id, runtime_spec,
                        target_source_host_id, relocation_spec,
                        profile_picture_url, owner_chat_account_id, status, requested_launch_code, agent_runtime_id,
                        runner_id, lease_token, core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message,
                        core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
                 FROM agent_creation_requests WHERE id = $1",
                &[&id],
            )
            .await
            .unwrap()
            .map(|row| agent_creation_request_from_row(&row).unwrap())
    }

    pub(crate) async fn all_agent_creation_requests(&self) -> Vec<AgentCreationRequest> {
        let mut out = Vec::new();
        for id in self.ids("agent_creation_requests").await {
            out.push(self.agent_creation_request(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn finite_private_api_key(&self, id: &str) -> Option<FinitePrivateApiKey> {
        let client = self.connection().await.unwrap();
        client
            .query_opt(
                "SELECT id, grant_id, project_id, agent_runtime_id, key_hash, status,
                        core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
                 FROM finite_private_api_keys WHERE id = $1",
                &[&id],
            )
            .await
            .unwrap()
            .map(|row| finite_private_api_key_from_row(&row).unwrap())
    }

    pub(crate) async fn all_finite_private_api_keys(&self) -> Vec<FinitePrivateApiKey> {
        let mut out = Vec::new();
        for id in self.ids("finite_private_api_keys").await {
            out.push(self.finite_private_api_key(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn all_runtime_control_requests(&self) -> Vec<RuntimeControlRequest> {
        let mut out = Vec::new();
        for id in self.ids("runtime_control_requests").await {
            out.push(self.runtime_control_request(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn user(&self, id: &str) -> Option<CoreUser> {
        let client = self.connection().await.unwrap();
        select_user_by_id(&**client, id).await.unwrap()
    }

    pub(crate) async fn all_users(&self) -> Vec<CoreUser> {
        let mut out = Vec::new();
        for id in self.ids("users").await {
            out.push(self.user(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn provider_operation(&self, id: &str) -> Option<ProviderOperationEnvelope> {
        let client = self.connection().await.unwrap();
        select_provider_operation(&**client, id).await.unwrap()
    }

    pub(crate) async fn finite_private_reservation(
        &self,
        id: &str,
    ) -> Option<FinitePrivateReservation> {
        let client = self.connection().await.unwrap();
        select_finite_private_reservation(&**client, id, false)
            .await
            .unwrap()
    }

    pub(crate) async fn all_finite_private_reservations(&self) -> Vec<FinitePrivateReservation> {
        let mut out = Vec::new();
        for id in self.ids("finite_private_reservations").await {
            out.push(self.finite_private_reservation(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn visible_projects_for_user(&self, user_id: &str) -> Vec<VisibleProject> {
        let client = self.connection().await.unwrap();
        postgres_visible_projects_for_user(&**client, user_id)
            .await
            .unwrap()
    }

    /// The key with this raw material, plus its grant.
    ///
    /// Delegates to the production lookup so a test sees the same
    /// active-key/active-grant semantics the API enforces.
    pub(crate) async fn finite_private_key_and_grant(
        &self,
        raw_key: &str,
    ) -> Option<(FinitePrivateApiKey, FinitePrivateGrant)> {
        let client = self.connection().await.unwrap();
        postgres_finite_private_key_and_grant(&**client, raw_key)
            .await
            .unwrap()
    }

    pub(crate) async fn customer_org(&self, id: &str) -> Option<CustomerOrganization> {
        let client = self.connection().await.unwrap();
        client
            .query_opt(
                "SELECT id, owner_user_id, name, billing_class,
                        core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at
                 FROM customer_orgs WHERE id = $1",
                &[&id],
            )
            .await
            .unwrap()
            .map(|row| customer_org_from_row(&row).unwrap())
    }

    pub(crate) async fn all_customer_orgs(&self) -> Vec<CustomerOrganization> {
        let mut out = Vec::new();
        for id in self.ids("customer_orgs").await {
            out.push(self.customer_org(&id).await.unwrap());
        }
        out
    }

    pub(crate) async fn customer_billing_account(
        &self,
        org_id: &str,
    ) -> Option<CustomerBillingAccount> {
        let client = self.connection().await.unwrap();
        billing::select_customer_billing_account(&**client, org_id, false)
            .await
            .unwrap()
    }

    pub(crate) async fn agent_creation_entitlement(
        &self,
        org_id: &str,
    ) -> Option<AgentCreationEntitlement> {
        let client = self.connection().await.unwrap();
        select_agent_creation_entitlement_by_org(&**client, org_id)
            .await
            .unwrap()
    }

    pub(crate) async fn active_runtime_for_project(
        &self,
        project_id: &str,
    ) -> Option<AgentRuntime> {
        let client = self.connection().await.unwrap();
        postgres_active_runtime_for_project(&**client, project_id)
            .await
            .unwrap()
    }

    /// Weekly reserved/settled usage for a grant at `now`.
    ///
    /// Mirrors the production window: `now` minus the weekly window seconds.
    pub(crate) async fn finite_private_weekly_usage(
        &self,
        grant_id: &str,
        now: time::OffsetDateTime,
    ) -> CoreResult<(i64, Option<String>)> {
        let window_start = (now - Duration::seconds(crate::FINITE_PRIVATE_WEEKLY_WINDOW_SECONDS))
            .format(&Rfc3339)?;
        let now = now.format(&Rfc3339)?;
        let client = self.connection().await?;
        postgres_finite_private_weekly_usage(&**client, grant_id, &window_start, &now).await
    }

    /// Run a statement for tests that need to stage durable state the store
    /// API cannot reach (an expired lease, a legacy row).
    pub(crate) async fn exec(&self, sql: &str) {
        let client = self.connection().await.unwrap();
        client
            .batch_execute(sql)
            .await
            .unwrap_or_else(|error| panic!("test statement failed: {error}\n{sql}"));
    }

    pub(crate) async fn table_len(&self, table: &str) -> usize {
        let key = match table {
            "runtime_retirement_snapshots" => "request_id",
            "runtime_relay_credentials" => "agent_runtime_id",
            _ => "id",
        };
        self.ids_by(table, key).await.len()
    }
}

#[cfg(test)]
mod tests {
    use crate::BillingSubscriptionStatus;

    use super::*;
    use crate::test_support::{TestDb, with_isolated_postgres};
    use crate::{
        FinitePrivateApiKeyStatus, RUNTIME_RELOCATION_SCHEMA, RunnerClass, RunnerLeaseCapacity,
        RuntimeArtifactKind, RuntimeCapabilitiesEnvelope, RuntimeCapabilitiesV1,
    };
    use futures_util::FutureExt;
    use std::collections::{BTreeMap, BTreeSet};

    fn kata_runtime_capabilities() -> RuntimeCapabilitiesEnvelope {
        RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
            restart: true,
            recover_known_good_chat: false,
            runtime_upgrade: true,
            stop: true,
            runtime_retirement: false,
        })
    }

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
            .find("impl CoreStore {")
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
    async fn postgres_pool_does_not_head_of_line_block_independent_reads() {
        with_isolated_postgres(|database| async move {
            let store = database.store.clone();
            assert_eq!(store.pool.status().max_size, DEFAULT_POSTGRES_POOL_SIZE);

            let slow_store = store.clone();
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let slow_query = tokio::spawn(async move {
                let client = slow_store.connection().await.unwrap();
                started_tx.send(()).unwrap();
                client.query_one("SELECT pg_sleep(1)", &[]).await.unwrap();
            });
            started_rx.await.unwrap();
            // Give Postgres time to enter pg_sleep. The assertion's 500 ms
            // budget is still comfortably below the one-second slow query.
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;

            tokio::time::timeout(
                std::time::Duration::from_millis(500),
                store.list_launch_code_batches(),
            )
            .await
            .expect("an unrelated read must use another pooled connection")
            .unwrap();
            slow_query.await.unwrap();
        })
        .await;
    }

    #[tokio::test]
    async fn postgres_finite_private_usage_accepts_linked_user_without_grant() {
        with_isolated_postgres(|store| async move {
            let workos_user_id = "workos_dashboard_summary_no_grant";
            store
                .link_verified_user(LinkVerifiedUserInput {
                    verified_email: "dashboard-summary-no-grant@finite.vip".to_string(),
                    workos_user_id: workos_user_id.to_string(),
                    now: None,
                })
                .await
                .unwrap();

            assert_eq!(
                store
                    .finite_private_usage_status_for_workos_user(workos_user_id, None)
                    .await
                    .unwrap(),
                None
            );
        })
        .await;
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
    async fn postgres_rejects_selected_tier_mismatch_without_consuming_launch_code() {
        with_isolated_postgres(|store| async move {
            let issued = store
                .issue_launch_code_batch(IssueLaunchCodeBatchInput {
                    name: "Tier mismatch".to_string(),
                    code_count: 1,
                    expires_in_hours: Some(1),
                    hosting_tier: Some(HostingTier::Standard),
                    created_by_workos_user_id: "workos_operator".to_string(),
                    now: Some("2026-07-23T12:00:00Z".to_string()),
                })
                .await
                .unwrap();
            let input = RequestAgentCreationInput {
                verified_email: "tier-check@finite.vip".to_string(),
                workos_user_id: "workos_tier_check".to_string(),
                display_name: "Tier Check".to_string(),
                launch_code: issued.codes[0].code.clone(),
                idempotency_key: "tier-check-submit".to_string(),
                now: Some("2026-07-23T12:01:00Z".to_string()),
            };

            let denied = store
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

            let created = store
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
    async fn postgres_migration_replay_preserves_and_repairs_pending_explicit_placement() {
        with_isolated_postgres(|store| async move {
            let launch_code = issue_test_launch_code(&store, "2026-07-31T12:00:00Z").await;
            let created = store
                .request_agent_creation_configured(
                    RequestAgentCreationInput {
                        verified_email: "migration-replay@finite.vip".to_string(),
                        workos_user_id: "workos_migration_replay".to_string(),
                        display_name: "Migration Replay".to_string(),
                        launch_code,
                        idempotency_key: "migration-replay-submit".to_string(),
                        now: Some("2026-07-31T12:01:00Z".to_string()),
                    },
                    AgentCreationConfiguration {
                        placement: Some(RuntimePlacement {
                            runner_class: RunnerClass::AppleContainer,
                            runtime_resource_class: crate::RuntimeResourceClass::Vcpu4Memory8Gib,
                        }),
                        requested_hosting_tier: Some(HostingTier::Standard),
                        profile_picture_url: None,
                        owner_chat_account_id: None,
                    },
                )
                .await
                .unwrap();
            assert_eq!(created.request.runner_class, RunnerClass::AppleContainer);
            assert_eq!(
                created.request.placement.unwrap().runner_class,
                RunnerClass::AppleContainer
            );

            // Core replays the full concatenated schema on every startup. A
            // modern pending request must retain its exact placement.
            store.migrate().await.unwrap();
            let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });
            let runner_class: String = raw
                .query_one(
                    "SELECT runner_class FROM agent_creation_requests WHERE id = $1",
                    &[&created.request.id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(runner_class, "apple_container");

            // Reproduce the durable shape left by the bad replay, then prove
            // the guarded repair before exercising the real lease query.
            raw.execute(
                "UPDATE agent_creation_requests SET runner_class = 'kata' WHERE id = $1",
                &[&created.request.id],
            )
            .await
            .unwrap();
            store.migrate().await.unwrap();
            let repaired_runner_class: String = raw
                .query_one(
                    "SELECT runner_class FROM agent_creation_requests WHERE id = $1",
                    &[&created.request.id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(repaired_runner_class, "apple_container");

            let lease = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "devfinity-apple-runner".to_string(),
                    lease_token: "migration-replay-lease".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::AppleContainer],
                        max_sandbox_count: Some(1),
                        active_sandbox_count: Some(0),
                        ..RunnerLeaseCapacity::default()
                    }),
                    source_host_id: Some("devfinity-apple".to_string()),
                    now: Some("2026-07-31T12:02:00Z".to_string()),
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(lease.request.id, created.request.id);
            assert_eq!(lease.request.runner_class, RunnerClass::AppleContainer);
            assert_eq!(
                lease.request.placement.unwrap().runner_class,
                RunnerClass::AppleContainer
            );

            drop(raw);
            connection.abort();
        })
        .await;
    }

    #[tokio::test]
    async fn postgres_owner_chat_account_id_persists_and_lease_injects_spec_environment() {
        with_isolated_postgres(|store| async move {
            store
                .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                    id: "artifact-owner-npub-v1".to_string(),
                    kind: RuntimeArtifactKind::OciImage,
                    reference: format!(
                        "ghcr.io/finitecomputer/agent-runtime:owner-npub-v1@sha256:{}",
                        "5".repeat(64)
                    ),
                    version_label: "owner-npub-v1".to_string(),
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

            // Malformed owner chat identities are rejected before any durable
            // state is minted; an npub is not accepted because the Hermes
            // adapter allowlist and this column both speak 64-hex account ids.
            let malformed = store
                .request_agent_creation_configured(
                    RequestAgentCreationInput {
                        verified_email: "owner-npub-bad@finite.vip".to_string(),
                        workos_user_id: "workos_owner_npub_bad".to_string(),
                        display_name: "Owner Npub Bad".to_string(),
                        launch_code: issue_test_launch_code(&store, "2026-08-27T12:00:00Z").await,
                        idempotency_key: "owner-npub-bad-submit".to_string(),
                        now: None,
                    },
                    AgentCreationConfiguration {
                        owner_chat_account_id: Some(format!("npub1{}", "q".repeat(58))),
                        ..AgentCreationConfiguration::default()
                    },
                )
                .await
                .unwrap_err();
            assert!(matches!(malformed, CoreError::InvalidOwnerChatAccountId));

            let owner_account_id = "a".repeat(64);
            let created = store
                .request_agent_creation_configured(
                    RequestAgentCreationInput {
                        verified_email: "owner-npub@finite.vip".to_string(),
                        workos_user_id: "workos_owner_npub".to_string(),
                        display_name: "Owner Npub Agent".to_string(),
                        launch_code: issue_test_launch_code(&store, "2026-08-27T12:00:01Z").await,
                        idempotency_key: "owner-npub-submit".to_string(),
                        now: None,
                    },
                    AgentCreationConfiguration {
                        owner_chat_account_id: Some(owner_account_id.clone()),
                        ..AgentCreationConfiguration::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                created.request.owner_chat_account_id.as_deref(),
                Some(owner_account_id.as_str())
            );
            let persisted = store
                .agent_creation_request(&created.request.id)
                .await
                .unwrap();
            assert_eq!(
                persisted.owner_chat_account_id.as_deref(),
                Some(owner_account_id.as_str())
            );

            let lease = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "owner-npub-runner".to_string(),
                    lease_token: "owner-npub-lease".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        ..RunnerLeaseCapacity::default()
                    }),
                    source_host_id: None,
                    now: None,
                })
                .await
                .unwrap()
                .unwrap();
            let RuntimeSpecEnvelope::V1(spec) = lease.request.runtime_spec.as_ref().unwrap();
            assert_eq!(
                spec.environment.get("FINITECHAT_OWNER_NPUBS"),
                Some(&owner_account_id)
            );

            // A request without the owner identity leases with the exact
            // legacy environment: no FINITECHAT_OWNER_NPUBS key.
            let legacy = store
                .request_agent_creation_configured(
                    RequestAgentCreationInput {
                        verified_email: "owner-npub-absent@finite.vip".to_string(),
                        workos_user_id: "workos_owner_npub_absent".to_string(),
                        display_name: "Owner Npub Absent".to_string(),
                        launch_code: issue_test_launch_code(&store, "2026-08-27T12:00:02Z").await,
                        idempotency_key: "owner-npub-absent-submit".to_string(),
                        now: None,
                    },
                    AgentCreationConfiguration::default(),
                )
                .await
                .unwrap();
            assert_eq!(legacy.request.owner_chat_account_id, None);
            let legacy_lease = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "owner-npub-runner".to_string(),
                    lease_token: "owner-npub-legacy-lease".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        ..RunnerLeaseCapacity::default()
                    }),
                    source_host_id: None,
                    now: None,
                })
                .await
                .unwrap()
                .unwrap();
            let RuntimeSpecEnvelope::V1(legacy_spec) =
                legacy_lease.request.runtime_spec.as_ref().unwrap();
            assert!(
                !legacy_spec
                    .environment
                    .contains_key("FINITECHAT_OWNER_NPUBS")
            );
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

            let competing = CoreStore::connect(&store.url).await.unwrap();
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
            let first = CoreStore::connect(&store.url).await.unwrap();
            let second = CoreStore::connect(&store.url).await.unwrap();
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

    /// The schema as production knew it before the lifecycle state machine:
    /// every migration except 0021. The remap test below builds this shape,
    /// seeds the legacy vocabulary, and proves 0021 maps it exactly.
    const PRE_LIFECYCLE_SCHEMA_SQL: &str = concat!(
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
        include_str!("../migrations/0019_brain_agent_departure_facts.sql")
    );

    #[tokio::test]
    async fn postgres_lifecycle_migration_remaps_legacy_statuses_exactly() {
        // A scratch database at the pre-H1 schema, migrated forward by 0021
        // alone, mirrors the production upgrade path byte for byte.
        let admin_url = std::env::var("FC_CORE_POSTGRES_TEST_URL")
            .expect("FC_CORE_POSTGRES_TEST_URL is required for Core Postgres tests");
        let (admin, admin_connection) = tokio_postgres::connect(&admin_url, NoTls).await.unwrap();
        let admin_connection = tokio::spawn(async move {
            let _ = admin_connection.await;
        });
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let db_name = format!("fc_test_lifecycle_{unique}");
        admin
            .execute(&format!("CREATE DATABASE \"{db_name}\""), &[])
            .await
            .unwrap();
        let (base, query) = match admin_url.split_once('?') {
            Some((base, query)) => (base.to_string(), Some(query.to_string())),
            None => (admin_url.clone(), None),
        };
        let scheme_end = base.find("://").map(|idx| idx + 3).unwrap_or(0);
        let db_url = match base[scheme_end..].find('/') {
            Some(rel) => format!("{}/{db_name}", &base[..scheme_end + rel]),
            None => format!("{base}/{db_name}"),
        };
        let db_url = match query {
            Some(query) => format!("{db_url}?{query}"),
            None => db_url,
        };
        let (raw, connection) = tokio_postgres::connect(&db_url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });

        let outcome = std::panic::AssertUnwindSafe(async {
            raw.batch_execute(PRE_LIFECYCLE_SCHEMA_SQL).await.unwrap();
            raw.batch_execute(
                "INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at)
                 VALUES ('legacy-user', 'legacy@finite.vip', 'linked', 'workos-legacy', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO customer_orgs (id, owner_user_id, name, billing_class, created_at, updated_at)
                 VALUES ('legacy-org', 'legacy-user', 'Legacy', 'grandfathered', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, created_at, updated_at)
                 VALUES ('legacy-project', 'legacy-org', 'legacy-user', 'Legacy', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO runtime_artifacts (id, kind, reference, version_label, state_schema_version, created_at, promoted_at)
                 VALUES ('legacy-artifact', 'oci_image', 'image@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'v1', 'state-v1', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO agent_runtimes (
                   id, project_id, source_host_id, source_machine_id, source_import_key,
                   runtime_artifact_id, state_schema_version, host_facts, created_at, updated_at
                 ) VALUES
                   ('legacy-runtime', 'legacy-project', 'legacy-host', 'legacy-machine',
                    'legacy-host/legacy-machine', 'legacy-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('legacy-runtime-2', 'legacy-project', 'legacy-host', 'legacy-machine-2',
                    'legacy-host/legacy-machine-2', 'legacy-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO runtime_control_requests (
                   id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                   requested_by_user_id, kind, status, created_at, updated_at
                 ) VALUES
                   ('legacy-requested', 'legacy-project', 'legacy-runtime', 'legacy-host', 'legacy-machine', 'legacy-user', 'restart', 'requested', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('legacy-running', 'legacy-project', 'legacy-runtime-2', 'legacy-host', 'legacy-machine-2', 'legacy-user', 'restart', 'running', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('legacy-succeeded-restart', 'legacy-project', 'legacy-runtime', 'legacy-host', 'legacy-machine', 'legacy-user', 'restart', 'succeeded', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('legacy-succeeded-stop', 'legacy-project', 'legacy-runtime', 'legacy-host', 'legacy-machine', 'legacy-user', 'stop', 'succeeded', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('legacy-succeeded-destroy', 'legacy-project', 'legacy-runtime-2', 'legacy-host', 'legacy-machine-2', 'legacy-user', 'destroy', 'succeeded', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('legacy-failed', 'legacy-project', 'legacy-runtime', 'legacy-host', 'legacy-machine', 'legacy-user', 'upgrade', 'failed', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);",
            )
            .await
            .unwrap();

            raw.batch_execute(include_str!("../migrations/0021_runtime_lifecycle.sql"))
                .await
                .unwrap();

            let rows = raw
                .query(
                    "SELECT id, status, failure_stage FROM runtime_control_requests ORDER BY id",
                    &[],
                )
                .await
                .unwrap();
            let mapped: BTreeMap<String, (String, String)> = rows
                .iter()
                .map(|row| {
                    (
                        row.get::<_, String>("id"),
                        (row.get::<_, String>("status"), row.get::<_, String>("failure_stage")),
                    )
                })
                .collect();
            assert_eq!(
                mapped.get("legacy-requested"),
                Some(&("requested".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("legacy-running"),
                Some(&("launching".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("legacy-succeeded-restart"),
                Some(&("succeeded".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("legacy-succeeded-stop"),
                Some(&("stopped".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("legacy-succeeded-destroy"),
                Some(&("stopped".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("legacy-failed"),
                Some(&("failed".to_string(), "unknown".to_string()))
            );

            // The new CHECK rejects the legacy vocabulary.
            let legacy_write = raw
                .execute(
                    "UPDATE runtime_control_requests SET status = 'running' WHERE id = 'legacy-requested'",
                    &[],
                )
                .await;
            assert!(legacy_write.is_err());

            // The one-active index spans every non-terminal state.
            let index_definition: String = raw
                .query_one(
                    "SELECT pg_get_indexdef(indexrelid) FROM pg_index
                     WHERE indexrelid = 'runtime_control_requests_one_active_per_runtime'::regclass",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert!(index_definition.contains("'launching'"));
            assert!(index_definition.contains("'compute_up'"));
            assert!(!index_definition.contains("'running'"));

            // Reapplying is a no-op: the migration runs at every Core startup.
            raw.batch_execute(include_str!("../migrations/0021_runtime_lifecycle.sql"))
                .await
                .unwrap();
            let remapped_count: i64 = raw
                .query_one(
                    "SELECT count(*) FROM runtime_control_requests WHERE status IN ('running')",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(remapped_count, 0);
        })
        .catch_unwind()
        .await;

        drop(raw);
        connection.abort();
        let _ = admin
            .execute(
                &format!("DROP DATABASE IF EXISTS \"{db_name}\" WITH (FORCE)"),
                &[],
            )
            .await;
        drop(admin);
        admin_connection.abort();
        if let Err(panic) = outcome {
            std::panic::resume_unwind(panic);
        }
    }

    /// The symmetric counterpart to
    /// `postgres_lifecycle_migration_remaps_legacy_statuses_exactly`: 0021
    /// applies forward onto a pre-H1 database, post-H1 writers mix in the new
    /// vocabulary, and the operator-initiated reverse remap lands the table on
    /// a shape the PREVIOUS generation of Core accepts — legacy CHECK and
    /// index predicate restored, legacy-vocabulary writes admitted once more,
    /// and the N-1 lease scan finding every relaunched row.
    #[tokio::test]
    async fn postgres_lifecycle_reverse_remap_restores_previous_generation_shape() {
        // A scratch database at the pre-H1 schema migrated forward by 0021
        // alone mirrors the production state an H1 rollback would face.
        let admin_url = std::env::var("FC_CORE_POSTGRES_TEST_URL")
            .expect("FC_CORE_POSTGRES_TEST_URL is required for Core Postgres tests");
        let (admin, admin_connection) = tokio_postgres::connect(&admin_url, NoTls).await.unwrap();
        let admin_connection = tokio::spawn(async move {
            let _ = admin_connection.await;
        });
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let db_name = format!("fc_test_reverse_remap_{unique}");
        admin
            .execute(&format!("CREATE DATABASE \"{db_name}\""), &[])
            .await
            .unwrap();
        let (base, query) = match admin_url.split_once('?') {
            Some((base, query)) => (base.to_string(), Some(query.to_string())),
            None => (admin_url.clone(), None),
        };
        let scheme_end = base.find("://").map(|idx| idx + 3).unwrap_or(0);
        let db_url = match base[scheme_end..].find('/') {
            Some(rel) => format!("{}/{db_name}", &base[..scheme_end + rel]),
            None => format!("{base}/{db_name}"),
        };
        let db_url = match query {
            Some(query) => format!("{db_url}?{query}"),
            None => db_url,
        };
        let (raw, connection) = tokio_postgres::connect(&db_url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });

        let outcome = std::panic::AssertUnwindSafe(async {
            raw.batch_execute(PRE_LIFECYCLE_SCHEMA_SQL).await.unwrap();
            raw.batch_execute(
                "INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at)
                 VALUES ('remap-user', 'remap@finite.vip', 'linked', 'workos-remap', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO customer_orgs (id, owner_user_id, name, billing_class, created_at, updated_at)
                 VALUES ('remap-org', 'remap-user', 'Remap', 'grandfathered', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, created_at, updated_at)
                 VALUES ('remap-project', 'remap-org', 'remap-user', 'Remap', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO runtime_artifacts (id, kind, reference, version_label, state_schema_version, created_at, promoted_at)
                 VALUES ('remap-artifact', 'oci_image', 'image@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'v1', 'state-v1', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO agent_runtimes (
                   id, project_id, source_host_id, source_machine_id, source_import_key,
                   runtime_artifact_id, state_schema_version, host_facts, created_at, updated_at
                 ) VALUES
                   ('remap-runtime-r0', 'remap-project', 'remap-host', 'remap-runtime-r0',
                    'remap-host/remap-runtime-r0', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r1', 'remap-project', 'remap-host', 'remap-runtime-r1',
                    'remap-host/remap-runtime-r1', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r2', 'remap-project', 'remap-host', 'remap-runtime-r2',
                    'remap-host/remap-runtime-r2', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r3', 'remap-project', 'remap-host', 'remap-runtime-r3',
                    'remap-host/remap-runtime-r3', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r4', 'remap-project', 'remap-host', 'remap-runtime-r4',
                    'remap-host/remap-runtime-r4', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r5', 'remap-project', 'remap-host', 'remap-runtime-r5',
                    'remap-host/remap-runtime-r5', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO runtime_control_requests (
                   id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                   requested_by_user_id, kind, status, created_at, updated_at
                 ) VALUES
                   ('remap-legacy-requested', 'remap-project', 'remap-runtime-r0', 'remap-host', 'remap-runtime-r0', 'remap-user', 'restart', 'requested', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-legacy-running', 'remap-project', 'remap-runtime-r1', 'remap-host', 'remap-runtime-r1', 'remap-user', 'restart', 'running', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-legacy-succ-restart', 'remap-project', 'remap-runtime-r2', 'remap-host', 'remap-runtime-r2', 'remap-user', 'restart', 'succeeded', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-legacy-succ-stop', 'remap-project', 'remap-runtime-r3', 'remap-host', 'remap-runtime-r3', 'remap-user', 'stop', 'succeeded', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-legacy-succ-destroy', 'remap-project', 'remap-runtime-r4', 'remap-host', 'remap-runtime-r4', 'remap-user', 'destroy', 'succeeded', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-legacy-failed', 'remap-project', 'remap-runtime-r5', 'remap-host', 'remap-runtime-r5', 'remap-user', 'upgrade', 'failed', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);",
            )
            .await
            .unwrap();

            raw.batch_execute(include_str!("../migrations/0021_runtime_lifecycle.sql"))
                .await
                .unwrap();

            // Post-H1 writers continue against the new vocabulary: active rows
            // span launching/compute_up/ready, stop/destroy completions land on
            // 'stopped', and one upgrade request stays in flight so the refusal
            // guard has something to catch.
            raw.batch_execute(
                "INSERT INTO agent_runtimes (
                   id, project_id, source_host_id, source_machine_id, source_import_key,
                   runtime_artifact_id, state_schema_version, host_facts, created_at, updated_at
                 ) VALUES
                   ('remap-runtime-r6', 'remap-project', 'remap-host', 'remap-runtime-r6',
                    'remap-host/remap-runtime-r6', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r7', 'remap-project', 'remap-host', 'remap-runtime-r7',
                    'remap-host/remap-runtime-r7', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r8', 'remap-project', 'remap-host', 'remap-runtime-r8',
                    'remap-host/remap-runtime-r8', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r9', 'remap-project', 'remap-host', 'remap-runtime-r9',
                    'remap-host/remap-runtime-r9', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r10', 'remap-project', 'remap-host', 'remap-runtime-r10',
                    'remap-host/remap-runtime-r10', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-runtime-r11', 'remap-project', 'remap-host', 'remap-runtime-r11',
                    'remap-host/remap-runtime-r11', 'remap-artifact', 'state-v1', '{}'::jsonb,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);
                 INSERT INTO runtime_control_requests (
                   id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                   requested_by_user_id, kind, status, created_at, updated_at
                 ) VALUES
                   ('remap-launching', 'remap-project', 'remap-runtime-r6', 'remap-host', 'remap-runtime-r6', 'remap-user', 'restart', 'launching', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-compute-up', 'remap-project', 'remap-runtime-r7', 'remap-host', 'remap-runtime-r7', 'remap-user', 'recover_known_good_chat_runtime', 'compute_up', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-ready', 'remap-project', 'remap-runtime-r8', 'remap-host', 'remap-runtime-r8', 'remap-user', 'restart', 'ready', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-stopped-stop', 'remap-project', 'remap-runtime-r9', 'remap-host', 'remap-runtime-r9', 'remap-user', 'stop', 'stopped', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-stopped-destroy', 'remap-project', 'remap-runtime-r10', 'remap-host', 'remap-runtime-r10', 'remap-user', 'destroy', 'stopped', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
                   ('remap-upgrade-bait', 'remap-project', 'remap-runtime-r11', 'remap-host', 'remap-runtime-r11', 'remap-user', 'upgrade', 'requested', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);",
            )
            .await
            .unwrap();

            // The refusal guard aborts the whole rescue while any upgrade-kind
            // request is still active.
            let active_error = raw
                .batch_execute(crate::RUNTIME_LIFECYCLE_REVERSE_REMAP_SQL)
                .await
                .unwrap_err();
            let db_error = active_error
                .as_db_error()
                .expect("reverse remap refusal must be a PostgreSQL error");
            assert_eq!(
                db_error.code(),
                &tokio_postgres::error::SqlState::RAISE_EXCEPTION
            );
            assert_eq!(
                db_error.message(),
                "runtime lifecycle reverse remap refused: active upgrade requests still exist"
            );
            raw.batch_execute("ROLLBACK").await.unwrap();
            let untouched_status: String = raw
                .query_one(
                    "SELECT status FROM runtime_control_requests WHERE id = 'remap-launching'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(untouched_status, "launching");
            let refused_audit_count: i64 = raw
                .query_one(
                    "SELECT count(*) FROM finite_private_admin_audit_events
                     WHERE action = 'runtime.lifecycle.reverse_remap'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(refused_audit_count, 0);

            // The operator makes the blocking request terminal; now the rescue
            // applies — twice, since the rolled-back generation may start
            // against a schema this script has already reversed.
            raw.execute(
                "UPDATE runtime_control_requests
                 SET status = 'failed', completed_at = CURRENT_TIMESTAMP
                 WHERE id = 'remap-upgrade-bait'",
                &[],
            )
            .await
            .unwrap();
            raw.batch_execute(crate::RUNTIME_LIFECYCLE_REVERSE_REMAP_SQL)
                .await
                .unwrap();
            raw.batch_execute(crate::RUNTIME_LIFECYCLE_REVERSE_REMAP_SQL)
                .await
                .unwrap();

            // Every row is back inside the legacy vocabulary. The relaunched
            // rows carry 'running'; the stopped stop/destroy rows are terminal
            // successes again; nothing else moved.
            let rows = raw
                .query(
                    "SELECT id, status, failure_stage FROM runtime_control_requests ORDER BY id",
                    &[],
                )
                .await
                .unwrap();
            let mapped: BTreeMap<String, (String, String)> = rows
                .iter()
                .map(|row| {
                    (
                        row.get::<_, String>("id"),
                        (row.get::<_, String>("status"), row.get::<_, String>("failure_stage")),
                    )
                })
                .collect();
            assert_eq!(
                mapped.get("remap-legacy-requested"),
                Some(&("requested".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-legacy-running"),
                Some(&("running".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-legacy-succ-restart"),
                Some(&("succeeded".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-legacy-succ-stop"),
                Some(&("succeeded".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-legacy-succ-destroy"),
                Some(&("succeeded".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-legacy-failed"),
                Some(&("failed".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-launching"),
                Some(&("running".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-compute-up"),
                Some(&("running".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-ready"),
                Some(&("running".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-stopped-stop"),
                Some(&("succeeded".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-stopped-destroy"),
                Some(&("succeeded".to_string(), "unknown".to_string()))
            );
            assert_eq!(
                mapped.get("remap-upgrade-bait"),
                Some(&("failed".to_string(), "unknown".to_string()))
            );

            // The legacy CHECK admits the legacy vocabulary again: the same
            // write 0021's test proved rejected must now succeed.
            raw.execute(
                "UPDATE runtime_control_requests SET status = 'running' WHERE id = 'remap-legacy-requested'",
                &[],
            )
            .await
            .unwrap();

            // The previous generation of Core inserts with an explicit column
            // list that predates failure_stage; the NOT NULL DEFAULT 'unknown'
            // column left in place must fill silently behind it.
            raw.execute(
                "INSERT INTO runtime_control_requests (
                   id, project_id, agent_runtime_id, source_host_id, source_machine_id,
                   requested_by_user_id, kind, target_runtime_artifact_id, status,
                   runner_id, lease_token, lease_expires_at,
                   failure_message, created_at, updated_at, completed_at
                 )
                 VALUES ('remap-n1-insert', 'remap-project', 'remap-runtime-r2', 'remap-host',
                         'remap-runtime-r2', 'remap-user', 'restart', NULL, 'requested',
                         NULL, NULL, NULL, NULL, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, NULL)",
                &[],
            )
            .await
            .unwrap();

            // The N-1 lease scan keys active work off requested/running only;
            // every relaunched row must be visible to it. 4 relaunched rows +
            // remap-legacy-running round-tripped to running + the probe above
            // flipped remap-legacy-requested onto 'running' + the inserted
            // 'remap-n1-insert' at 'requested'.
            let n_minus_one_active_count: i64 = raw
                .query_one(
                    "SELECT count(*) FROM runtime_control_requests
                     WHERE status IN ('requested', 'running')",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(n_minus_one_active_count, 6);

            // Constraint and index speak the legacy shape again.
            let constraint_definition: String = raw
                .query_one(
                    "SELECT pg_get_constraintdef(constraint_row.oid)
                     FROM pg_constraint AS constraint_row
                     WHERE constraint_row.conrelid = 'runtime_control_requests'::regclass
                       AND constraint_row.conname = 'runtime_control_requests_status_check'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert!(!constraint_definition.contains("'launching'"));
            assert!(!constraint_definition.contains("'stopped'"));
            let index_definition: String = raw
                .query_one(
                    "SELECT pg_get_indexdef(indexrelid) FROM pg_index
                     WHERE indexrelid = 'runtime_control_requests_one_active_per_runtime'::regclass",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert!(index_definition.contains("'running'"));
            assert!(!index_definition.contains("'launching'"));
            assert!(!index_definition.contains("'compute_up'"));

            // One audit row per rewritten request, preserving prior values.
            let audit_count: i64 = raw
                .query_one(
                    "SELECT count(*) FROM finite_private_admin_audit_events
                     WHERE action = 'runtime.lifecycle.reverse_remap'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(audit_count, 8);
            let ready_original: String = raw
                .query_one(
                    "SELECT metadata->>'originalStatus'
                     FROM finite_private_admin_audit_events
                     WHERE action = 'runtime.lifecycle.reverse_remap'
                       AND target_id = 'remap-ready'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(ready_original, "ready");
            let stopped_stop_kind: Option<String> = raw
                .query_one(
                    "SELECT metadata->>'originalKind'
                     FROM finite_private_admin_audit_events
                     WHERE action = 'runtime.lifecycle.reverse_remap'
                       AND target_id = 'remap-stopped-stop'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(stopped_stop_kind.as_deref(), Some("stop"));
        })
        .catch_unwind()
        .await;

        drop(raw);
        connection.abort();
        let _ = admin
            .execute(
                &format!("DROP DATABASE IF EXISTS \"{db_name}\" WITH (FORCE)"),
                &[],
            )
            .await;
        drop(admin);
        admin_connection.abort();
        if let Err(panic) = outcome {
            std::panic::resume_unwind(panic);
        }
    }

    #[tokio::test]
    async fn postgres_cold_relocation_migration_reapplies_and_preserves_primary_creation_fence() {
        with_isolated_postgres(|store| async move {
            let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });

            raw.batch_execute(include_str!(
                "../migrations/0016_runtime_cold_relocation.sql"
            ))
            .await
            .unwrap();
            raw.batch_execute(include_str!(
                "../migrations/0016_runtime_cold_relocation.sql"
            ))
            .await
            .unwrap();

            let relocation_column_count: i64 = raw
                .query_one(
                    "SELECT count(*)
                     FROM information_schema.columns
                     WHERE table_schema = 'public'
                       AND table_name = 'agent_creation_requests'
                       AND column_name = 'relocation_spec'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(relocation_column_count, 1);

            let indexes = raw
                .query(
                    "SELECT indexname, indexdef
                     FROM pg_indexes
                     WHERE schemaname = 'public'
                       AND tablename = 'agent_creation_requests'
                       AND indexname IN (
                         'agent_creation_requests_one_primary_creation_per_project',
                         'agent_creation_requests_one_active_relocation_per_runtime'
                       )
                     ORDER BY indexname",
                    &[],
                )
                .await
                .unwrap();
            assert_eq!(indexes.len(), 2);
            let definitions = indexes
                .iter()
                .map(|row| row.get::<_, String>("indexdef"))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(definitions.contains("(project_id) WHERE (relocation_spec IS NULL)"));
            assert!(definitions.contains("(agent_runtime_id) WHERE"));
            assert!(definitions.contains("relocation_spec IS NOT NULL"));

            let old_project_unique_constraint: i64 = raw
                .query_one(
                    "SELECT count(*)
                     FROM pg_constraint
                     WHERE conrelid = 'agent_creation_requests'::regclass
                       AND conname = 'agent_creation_requests_project_id_key'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(old_project_unique_constraint, 0);

            drop(raw);
            connection.abort();
        })
        .await;
    }

    #[tokio::test]
    async fn postgres_cold_relocation_routes_exactly_and_register_failure_keeps_source() {
        with_isolated_postgres(|store| async move {
            let run = "cold-relocate";
            let source_host = "relocate-source";
            let target_host = "relocate-target";
            let machine = "finite-kata-relocate";
            let email = format!("{run}@finite.vip");
            let workos = format!("workos-{run}");
            let launch_code = issue_test_launch_code(&store, "2026-07-25T12:00:00Z").await;

            store
                .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                    id: "artifact-relocate-v1".to_string(),
                    kind: RuntimeArtifactKind::OciImage,
                    reference: format!(
                        "ghcr.io/finitecomputer/agent-runtime:relocate-v1@sha256:{}",
                        "4".repeat(64)
                    ),
                    version_label: "relocate-v1".to_string(),
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
                        display_name: "Relocation Canary".to_string(),
                        launch_code,
                        idempotency_key: format!("{run}-create"),
                        now: None,
                    },
                    AgentCreationConfiguration {
                        placement: RuntimePlacement::for_hosting_tier(HostingTier::Standard),
                        requested_hosting_tier: None,
                        profile_picture_url: None,
                        owner_chat_account_id: None,
                    },
                )
                .await
                .unwrap();
            let creation = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: format!("runner-{source_host}"),
                    source_host_id: Some(source_host.to_string()),
                    lease_token: "create-lease".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: None,
                })
                .await
                .unwrap()
                .unwrap();
            let completed = store
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: creation.request.id,
                    runner_id: format!("runner-{source_host}"),
                    lease_token: "create-lease".to_string(),
                    source_host_id: source_host.to_string(),
                    source_machine_id: machine.to_string(),
                    runtime_artifact_id: Some("artifact-relocate-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: Some("http://127.0.0.1:4201/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("Relocation Canary".to_string()),
                    hostname: None,
                    runtime_host: Some(source_host.to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: None,
                })
                .await
                .unwrap();
            let project_id = completed.project.id;
            let runtime_id = completed.request.agent_runtime_id.unwrap();

            let stop = store
                .request_runtime_stop(RequestRuntimeStopInput {
                    verified_email: email,
                    workos_user_id: workos,
                    project_id: project_id.clone(),
                    now: None,
                })
                .await
                .unwrap();
            let stop_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("runner-{source_host}"),
                    lease_token: "stop-lease".to_string(),
                    lease_seconds: Some(300),
                    source_host_id: Some(source_host.to_string()),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: None,
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stop_lease.request.id, stop.id);
            store
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: stop.id,
                    runner_id: format!("runner-{source_host}"),
                    lease_token: "stop-lease".to_string(),
                    runtime_artifact_id: None,
                    state_schema_version: None,
                    runtime_capabilities: None,
                    runtime_host: None,
                    published_app_urls: None,
                    retirement_snapshot: None,
                    now: None,
                })
                .await
                .unwrap();

            let relocation = store
                .admin_request_runtime_relocate_exact(AdminRuntimeRelocateExactInput {
                    admin_verified_email: "relocate-admin@finite.vip".to_string(),
                    admin_workos_user_id: "workos-relocate-admin".to_string(),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: source_host.to_string(),
                    expected_source_machine_id: machine.to_string(),
                    target_source_host_id: target_host.to_string(),
                    expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                    durable_state_manifest_sha256: "b".repeat(64),
                    operator_observed_compute_absent: false,
                    now: None,
                })
                .await
                .unwrap();
            assert!(
                store
                    .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                        runner_id: format!("runner-{source_host}"),
                        source_host_id: Some(source_host.to_string()),
                        lease_token: "wrong-host".to_string(),
                        lease_seconds: Some(300),
                        runner_capacity: Some(RunnerLeaseCapacity {
                            runner_classes: vec![RunnerClass::Kata],
                            ..RunnerLeaseCapacity::default()
                        }),
                        now: None,
                    })
                    .await
                    .unwrap()
                    .is_none()
            );
            let relocation_lease = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: format!("runner-{target_host}"),
                    source_host_id: Some(target_host.to_string()),
                    lease_token: "relocation-lease".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: None,
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(relocation_lease.request.id, relocation.id);

            store
                .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                    request_id: relocation.id.clone(),
                    runner_id: format!("runner-{target_host}"),
                    lease_token: "relocation-lease".to_string(),
                    source_host_id: target_host.to_string(),
                    source_machine_id: machine.to_string(),
                    runtime_artifact_id: Some("artifact-relocate-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: Some("http://127.0.0.1:4202/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("Relocation Canary".to_string()),
                    hostname: None,
                    runtime_host: Some(target_host.to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Unknown),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: None,
                })
                .await
                .unwrap();
            store
                .fail_agent_creation_request(FailAgentCreationRequestInput {
                    request_id: relocation.id.clone(),
                    runner_id: format!("runner-{target_host}"),
                    lease_token: "relocation-lease".to_string(),
                    failure_message: "synthetic post-register failure".to_string(),
                    provisioned_finite_private_api_key_id: None,
                    now: None,
                })
                .await
                .unwrap();
            let preserved = store
                .admin_runtime_overviews()
                .await
                .unwrap()
                .into_iter()
                .find(|overview| overview.agent_runtime_id == runtime_id)
                .unwrap();
            assert_eq!(preserved.source_host_id, source_host);
            assert_eq!(preserved.runtime_status, RuntimeSummaryStatus::Offline);

            // A failed attempt must not consume the Project's original
            // creation-row uniqueness or prevent an exact retry.
            let retry = store
                .admin_request_runtime_relocate_exact(AdminRuntimeRelocateExactInput {
                    admin_verified_email: "relocate-admin@finite.vip".to_string(),
                    admin_workos_user_id: "workos-relocate-admin".to_string(),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: source_host.to_string(),
                    expected_source_machine_id: machine.to_string(),
                    target_source_host_id: target_host.to_string(),
                    expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                    durable_state_manifest_sha256: "b".repeat(64),
                    operator_observed_compute_absent: false,
                    now: None,
                })
                .await
                .unwrap();
            assert_ne!(retry.id, relocation.id);
            store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: format!("runner-{target_host}"),
                    source_host_id: Some(target_host.to_string()),
                    lease_token: "relocation-retry-lease".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: None,
                })
                .await
                .unwrap()
                .unwrap();
            store
                .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                    request_id: retry.id.clone(),
                    runner_id: format!("runner-{target_host}"),
                    lease_token: "relocation-retry-lease".to_string(),
                    source_host_id: target_host.to_string(),
                    source_machine_id: machine.to_string(),
                    runtime_artifact_id: Some("artifact-relocate-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: Some("http://127.0.0.1:4202/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("Relocation Canary".to_string()),
                    hostname: None,
                    runtime_host: Some(target_host.to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Unknown),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: None,
                })
                .await
                .unwrap();
            let completed_retry = store
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: retry.id.clone(),
                    runner_id: format!("runner-{target_host}"),
                    lease_token: "relocation-retry-lease".to_string(),
                    source_host_id: target_host.to_string(),
                    source_machine_id: machine.to_string(),
                    runtime_artifact_id: Some("artifact-relocate-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: Some("http://127.0.0.1:4202/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("Relocation Canary".to_string()),
                    hostname: None,
                    runtime_host: Some(target_host.to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: None,
                })
                .await
                .unwrap();
            assert_eq!(
                completed_retry.request.status,
                AgentCreationRequestStatus::Running
            );

            let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });
            let runtime = raw
                .query_one(
                    "SELECT source_host_id, source_machine_id, host_facts->>'runtime_status' AS status
                     FROM agent_runtimes WHERE id = $1",
                    &[&runtime_id],
                )
                .await
                .unwrap();
            assert_eq!(runtime.get::<_, String>("source_host_id"), target_host);
            assert_eq!(runtime.get::<_, String>("source_machine_id"), machine);
            assert_eq!(runtime.get::<_, String>("status"), "online");
            let request = raw
                .query_one(
                    "SELECT status, agent_runtime_id, relocation_spec->>'schema' AS schema
                     FROM agent_creation_requests WHERE id = $1",
                    &[&relocation.id],
                )
                .await
                .unwrap();
            assert_eq!(request.get::<_, String>("status"), "failed");
            assert_eq!(
                request.get::<_, Option<String>>("agent_runtime_id").as_deref(),
                Some(runtime_id.as_str())
            );
            assert_eq!(
                request.get::<_, Option<String>>("schema").as_deref(),
                Some(RUNTIME_RELOCATION_SCHEMA)
            );
            let retry_status: String = raw
                .query_one(
                    "SELECT status FROM agent_creation_requests WHERE id = $1",
                    &[&retry.id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(retry_status, "running");

            drop(raw);
            connection.abort();
        })
        .await;
    }

    #[tokio::test]
    async fn postgres_cold_relocation_accepts_online_source_only_under_absence_attestation() {
        with_isolated_postgres(|store| async move {
            let run = "cold-relocate-online";
            let source_host = "relocate-online-source";
            let target_host = "relocate-online-target";
            let machine = "finite-kata-relocate-online";
            let email = format!("{run}@finite.vip");
            let workos = format!("workos-{run}");
            let launch_code = issue_test_launch_code(&store, "2026-07-25T12:00:00Z").await;

            store
                .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                    id: "artifact-relocate-v1".to_string(),
                    kind: RuntimeArtifactKind::OciImage,
                    reference: format!(
                        "ghcr.io/finitecomputer/agent-runtime:relocate-v1@sha256:{}",
                        "4".repeat(64)
                    ),
                    version_label: "relocate-v1".to_string(),
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
                        display_name: "Online Relocation Canary".to_string(),
                        launch_code,
                        idempotency_key: format!("{run}-create"),
                        now: None,
                    },
                    AgentCreationConfiguration {
                        placement: RuntimePlacement::for_hosting_tier(HostingTier::Standard),
                        requested_hosting_tier: None,
                        profile_picture_url: None,
                        owner_chat_account_id: None,
                    },
                )
                .await
                .unwrap();
            let created = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: format!("runner-{source_host}"),
                    source_host_id: Some(source_host.to_string()),
                    lease_token: "create-lease".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: Some(RunnerLeaseCapacity {
                        runner_classes: vec![RunnerClass::Kata],
                        runtime_capabilities: Some(kata_runtime_capabilities()),
                        ..RunnerLeaseCapacity::default()
                    }),
                    now: None,
                })
                .await
                .unwrap()
                .unwrap();
            let completed = store
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: created.request.id,
                    runner_id: format!("runner-{source_host}"),
                    lease_token: "create-lease".to_string(),
                    source_host_id: source_host.to_string(),
                    source_machine_id: machine.to_string(),
                    runtime_artifact_id: Some("artifact-relocate-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: Some("http://127.0.0.1:4203/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("Online Relocation Canary".to_string()),
                    hostname: None,
                    runtime_host: Some(source_host.to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: None,
                })
                .await
                .unwrap();
            let project_id = completed.project.id;
            let runtime_id = completed.request.agent_runtime_id.unwrap();

            // Without the operator's compute-absent attestation an online
            // source is NOT frozen: it may still be running, so the exact
            // relocation must refuse.
            let unattested = store
                .admin_request_runtime_relocate_exact(AdminRuntimeRelocateExactInput {
                    admin_verified_email: "relocate-admin@finite.vip".to_string(),
                    admin_workos_user_id: "workos-relocate-admin".to_string(),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: source_host.to_string(),
                    expected_source_machine_id: machine.to_string(),
                    target_source_host_id: target_host.to_string(),
                    expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                    durable_state_manifest_sha256: "b".repeat(64),
                    operator_observed_compute_absent: false,
                    now: None,
                })
                .await;
            assert!(matches!(
                unattested,
                Err(CoreError::RuntimeControlUnsupported)
            ));

            // Under the attestation the pre-death `online` report is exactly
            // as frozen as `stale`: the dead host's runner can neither lease
            // a control nor file a fresh report, so nothing could have moved
            // the runtime since the host died.
            let relocation = store
                .admin_request_runtime_relocate_exact(AdminRuntimeRelocateExactInput {
                    admin_verified_email: "relocate-admin@finite.vip".to_string(),
                    admin_workos_user_id: "workos-relocate-admin".to_string(),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: source_host.to_string(),
                    expected_source_machine_id: machine.to_string(),
                    target_source_host_id: target_host.to_string(),
                    expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                    durable_state_manifest_sha256: "b".repeat(64),
                    operator_observed_compute_absent: true,
                    now: None,
                })
                .await
                .unwrap();
            assert_eq!(relocation.status, AgentCreationRequestStatus::Requested);
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
            let placement = RuntimePlacement::for_hosting_tier(HostingTier::Standard)
                .expect("standard tier placement");
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

    /// Timestamp reads do not depend on the database server's timezone.
    ///
    /// A bare `col::text` renders Postgres's display format in the SERVER's
    /// zone, so the same row read on a UTC box and an Asia/Kolkata box produced
    /// different strings -- and neither was RFC3339. `core_rfc3339` pins the
    /// rendering to UTC. This drives the session timezone directly so the
    /// guarantee is checked rather than assumed from wherever CI happens to run.
    #[tokio::test]
    async fn postgres_timestamp_reads_are_independent_of_server_timezone() {
        with_isolated_postgres(|db| async move {
            let written = "2026-05-25T12:00:00Z";
            db.upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: "artifact-tz".to_string(),
                kind: crate::RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/agent-runtime:tz@sha256:{}",
                    "c".repeat(64)
                ),
                version_label: "tz".to_string(),
                source_git_sha: None,
                finitec_version: None,
                hermes_source_ref: None,
                finite_platform_plugin_ref: None,
                state_schema_version: "state-v1".to_string(),
                base_image: None,
                recover_known_good_chat: false,
                promoted: false,
                now: Some(written.to_string()),
            })
            .await
            .unwrap();

            for zone in ["UTC", "America/Chicago", "Asia/Kolkata"] {
                let client = db.store.connection().await.unwrap();
                client
                    .batch_execute(&format!("SET TIME ZONE '{zone}'"))
                    .await
                    .unwrap();
                let artifact = select_runtime_artifact(&**client, "artifact-tz")
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    artifact.created_at, written,
                    "read under TimeZone={zone} must match what was written"
                );
                // And Core must be able to read its own output back.
                crate::parse_time(&artifact.created_at)
                    .unwrap_or_else(|_| panic!("unparsable under TimeZone={zone}"));
            }
        })
        .await;
    }

    /// Artifact selection orders by the TIMESTAMPTZ column, not its rendered
    /// text.
    ///
    /// `SELECT core_rfc3339(promoted_at) AS promoted_at ... ORDER BY
    /// promoted_at` binds the output column in Postgres, so a bare name sorts
    /// lexicographically. RFC3339 only sorts correctly as text at a FIXED
    /// precision, and `current_time_iso` trims trailing zeros, so a whole
    /// second ("…:02Z") sorts after a fractional one ("…:02.5Z") -- 'Z' > '.'.
    /// That would launch new agents on the older artifact.
    #[tokio::test]
    async fn postgres_launchable_artifact_orders_by_instant_not_rendered_text() {
        with_isolated_postgres(|db| async move {
            // Same second, differing fractional precision. Lexicographically
            // "…:02Z" > "…:02.500000Z"; chronologically it is earlier.
            for (id, promoted) in [
                ("artifact-frac", "2030-01-01T00:00:02.5Z"),
                ("artifact-whole", "2030-01-01T00:00:02Z"),
            ] {
                db.upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                    id: id.to_string(),
                    kind: crate::RuntimeArtifactKind::OciImage,
                    reference: format!(
                        "ghcr.io/finitecomputer/agent-runtime:{id}@sha256:{}",
                        "a".repeat(64)
                    ),
                    version_label: id.to_string(),
                    source_git_sha: None,
                    finitec_version: None,
                    hermes_source_ref: None,
                    finite_platform_plugin_ref: None,
                    state_schema_version: "state-v1".to_string(),
                    base_image: None,
                    recover_known_good_chat: false,
                    promoted: true,
                    now: Some(promoted.to_string()),
                })
                .await
                .unwrap();
            }

            let client = db.store.connection().await.unwrap();
            let latest = select_latest_launchable_runtime_artifact(&**client)
                .await
                .unwrap();
            assert_eq!(
                latest.id, "artifact-frac",
                "the later instant must win, even though it sorts earlier as text"
            );
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
    async fn postgres_finite_private_default_survives_reapply_and_n_minus_one_schema() {
        with_isolated_postgres(|store| async move {
            // Reapplying the current concat is how this service migrates on
            // every startup. A rolled-back N-1 binary would then replay 0010,
            // which still names the old 50M policy. The compatibility trigger
            // must preserve the doubled limit in both cases.
            store.migrate().await.unwrap();
            let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });
            raw.batch_execute(include_str!(
                "../migrations/0010_align_finite_private_generous.sql"
            ))
            .await
            .unwrap();

            let old_limit: i64 = raw
                .query_one(
                    "SELECT burst_limit_units FROM finite_private_limit_profiles WHERE id = 'finite-private-generous'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            let new_limit: i64 = raw
                .query_one(
                    "SELECT burst_limit_units FROM finite_private_limit_profiles WHERE id = 'finite-private-generous-v2'",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            let five_x_limit: i64 = raw
                .query_one(
                    "SELECT burst_limit_units FROM finite_private_limit_profiles WHERE id = $1",
                    &[&crate::FINITE_PRIVATE_5X_LIMIT_PROFILE],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(old_limit, 100_000_000);
            assert_eq!(new_limit, 100_000_000);
            assert_eq!(five_x_limit, 500_000_000);
            let usage_index_exists: bool = raw
                .query_one(
                    "SELECT to_regclass('finite_private_reservations_grant_status_epoch_created_idx') IS NOT NULL",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert!(usage_index_exists);

            let run = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let raw_key = format!("fpk_live_schema_replay_{run}");
            let issued = store
                .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                    admin_verified_email: "schema-replay-admin@finite.vip".to_string(),
                    friend_email: format!("schema-replay-{run}@finite.vip"),
                    limit_profile_id: None,
                    raw_key,
                    now: None,
                })
                .await
                .unwrap();
            let profile_id: String = raw
                .query_one(
                    "SELECT limit_profile_id FROM finite_private_grants WHERE id = $1",
                    &[&issued.grant.id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(profile_id, "finite-private-generous-v2");

            drop(raw);
            connection.abort();
        })
        .await;
    }

    #[tokio::test]
    async fn postgres_finite_private_same_window_reservations_share_epoch_and_settle() {
        with_isolated_postgres(|store| async move {
            let issued = store
                .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
                    admin_verified_email: "epoch-admin@finite.vip".to_string(),
                    friend_email: "epoch-user@finite.vip".to_string(),
                    limit_profile_id: None,
                    raw_key: "fpk_live_postgres_epoch".to_string(),
                    now: Some("2026-07-21T12:00:00Z".to_string()),
                })
                .await
                .unwrap();

            let reserve =
                |request_id: &str, units: i64, now: &str| ReserveFinitePrivateUsageInput {
                    request_id: request_id.to_string(),
                    presented_api_key: "fpk_live_postgres_epoch".to_string(),
                    endpoint: "/v1/chat/completions".to_string(),
                    model: "glm-5.2".to_string(),
                    estimated_prompt_tokens: units,
                    estimated_completion_tokens: 0,
                    estimated_usage_units: units,
                    usage_formula_version: "v1".to_string(),
                    dashboard_url: "https://finite.computer/dashboard".to_string(),
                    now: Some(now.to_string()),
                };
            let first = store
                .reserve_finite_private_usage(reserve(
                    "req-postgres-epoch-1",
                    30_000_000,
                    "2026-07-21T12:00:01Z",
                ))
                .await
                .unwrap();
            let second = store
                .reserve_finite_private_usage(reserve(
                    "req-postgres-epoch-2",
                    30_000_000,
                    "2026-07-21T12:01:00Z",
                ))
                .await
                .unwrap();

            let settle = |reservation_id: String, request_id: &str, units: i64, now: &str| {
                SettleFinitePrivateReservationInput {
                    reservation_id,
                    request_id: request_id.to_string(),
                    settlement: crate::FinitePrivateSettlementKind::Actual,
                    prompt_tokens: Some(units),
                    completion_tokens: Some(0),
                    usage_units: Some(units),
                    usage_formula_version: "v1".to_string(),
                    upstream_status: Some(200),
                    upstream_error_class: None,
                    now: Some(now.to_string()),
                }
            };
            store
                .settle_finite_private_reservation(settle(
                    first.reservation_id.unwrap(),
                    "req-postgres-epoch-1",
                    20_000_000,
                    "2026-07-21T12:01:10Z",
                ))
                .await
                .unwrap();
            store
                .settle_finite_private_reservation(settle(
                    second.reservation_id.unwrap(),
                    "req-postgres-epoch-2",
                    55_000_000,
                    "2026-07-21T12:01:20Z",
                ))
                .await
                .unwrap();

            let status = store
                .finite_private_usage_status_for_api_key(
                    "fpk_live_postgres_epoch",
                    true,
                    Some("2026-07-21T12:02:00Z".to_string()),
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(status.burst_used_units, 75_000_000);
            assert_eq!(
                status
                    .notice
                    .as_ref()
                    .map(|notice| notice.threshold_remaining_percent),
                Some(25)
            );

            let (raw, connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });
            let epochs = raw
                .query(
                    "SELECT DISTINCT burst_window_epoch
                     FROM finite_private_reservations
                     WHERE grant_id = $1
                     ORDER BY burst_window_epoch",
                    &[&issued.grant.id],
                )
                .await
                .unwrap();
            assert_eq!(
                epochs.len(),
                1,
                "same-window reservations must share one epoch"
            );
            let settled_count: i64 = raw
                .query_one(
                    "SELECT COUNT(*) FROM finite_private_reservations
                     WHERE grant_id = $1 AND status = 'settled'",
                    &[&issued.grant.id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(settled_count, 2);
            drop(raw);
            connection.abort();
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
            let provisioned_owner_key = store
                .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                    request_id: lease.request.id.clone(),
                    runner_id: format!("runner-admin-ops-{run}"),
                    lease_token: format!("lease-admin-ops-{run}"),
                    source_host_id: Some("admin-ops-host".to_string()),
                    source_machine_id: Some(machine_id.clone()),
                    now: None,
                })
                .await
                .unwrap();
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
            let owner_account = store
                .finite_private_admin_state()
                .await
                .unwrap()
                .accounts
                .into_iter()
                .find(|account| account.email == owner_email)
                .expect("provisioned owner should have a correlated Finite Private account");
            assert_eq!(owner_account.grant.id, provisioned_owner_key.grant.id);
            assert!(owner_account.projects.iter().any(|project| {
                project.id == project_id
                    && project.agent_runtime_id.as_deref() == Some(runtime_id.as_str())
            }));

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
                    retirement_snapshot: None,
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

            let usage = store
                .finite_private_usage_status_for_api_key(
                    &raw_key,
                    true,
                    Some("2026-07-21T12:00:00Z".to_string()),
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(usage.burst_limit_units, 100_000_000);
            assert!(usage.free_daily_reset_available);
            let daily_reset = store
                .claim_finite_private_daily_reset_for_api_key(
                    &raw_key,
                    Some("2026-07-21T12:01:00Z".to_string()),
                )
                .await
                .unwrap();
            assert!(daily_reset.performed);
            let repeated_reset = store
                .claim_finite_private_daily_reset_for_api_key(
                    &raw_key,
                    Some("2026-07-21T12:02:00Z".to_string()),
                )
                .await
                .unwrap();
            assert!(!repeated_reset.performed);

            let before_assignment = store
                .finite_private_admin_state()
                .await
                .unwrap()
                .accounts
                .into_iter()
                .find(|account| account.email == friend_email)
                .unwrap()
                .grant;
            let assigned = store
                .admin_assign_finite_private_limit_profile(
                    AdminAssignFinitePrivateLimitProfileInput {
                        admin_verified_email: admin_email.clone(),
                        grant_id: issued.grant.id.clone(),
                        limit_profile_id: crate::FINITE_PRIVATE_5X_LIMIT_PROFILE.to_string(),
                        now: None,
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                assigned.limit_profile_id,
                crate::FINITE_PRIVATE_5X_LIMIT_PROFILE
            );
            assert_eq!(
                assigned.current_window_used_units,
                before_assignment.current_window_used_units
            );
            assert_eq!(
                assigned.burst_window_epoch,
                before_assignment.burst_window_epoch
            );
            let assigned_usage = store
                .finite_private_usage_status_for_api_key(
                    &raw_key,
                    true,
                    Some("2026-07-21T12:03:00Z".to_string()),
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(assigned_usage.burst_limit_units, 500_000_000);

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
            let account = admin_state
                .accounts
                .iter()
                .find(|account| account.email == friend_email)
                .unwrap();
            assert_eq!(account.grant.id, issued.grant.id);
            assert_eq!(account.api_keys.len(), 2);
            assert!(admin_state.profiles.iter().any(|profile| {
                profile.id == crate::FINITE_PRIVATE_5X_LIMIT_PROFILE
                    && profile.burst_limit_units == 500_000_000
            }));
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
            assert!(reset.current_window_started_at.is_some());

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
                "finite_private.grant.admin_assign_limit_profile",
            ] {
                assert!(
                    admin_actions.contains(&expected.to_string()),
                    "missing Postgres audit action {expected}"
                );
            }

            let archive_input = |compute_absent| AdminArchiveUnrecoverableRuntimeInput {
                admin_verified_email: admin_email.clone(),
                admin_workos_user_id: format!("workos_admin_ops_admin_{run}"),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: "admin-ops-host".to_string(),
                expected_source_machine_id: machine_id.clone(),
                expected_owner_email: owner_email.clone(),
                operator_observed_compute_absent: compute_absent,
                operator_observed_durable_state_absent: true,
                owner_acknowledged_unrecoverable: true,
                now: None,
            };
            assert!(matches!(
                store
                    .admin_archive_unrecoverable_runtime(archive_input(false))
                    .await
                    .unwrap_err(),
                CoreError::UnrecoverableRuntimeArchiveAcknowledgementRequired
            ));
            let archive = store
                .admin_archive_unrecoverable_runtime(archive_input(true))
                .await
                .unwrap();
            assert_eq!(archive.agent_runtime_id, runtime_id);
            let archived_overview = store
                .admin_runtime_overviews()
                .await
                .unwrap()
                .into_iter()
                .find(|overview| overview.agent_runtime_id == runtime_id)
                .unwrap();
            assert!(!archived_overview.runtime_link_active);
            assert!(
                store
                    .finite_private_admin_audit_events()
                    .await
                    .unwrap()
                    .iter()
                    .any(|event| {
                        event.action == "runtime.admin_archive_unrecoverable"
                            && event.actor == admin_email
                            && event.target_id == runtime_id
                    })
            );
        })
        .await;
    }

    /// Runner-ferried standing readiness (2026-08 audit synthesis, H1 slice
    /// 3): a report writes the runtime row's latest-report columns scoped to
    /// the runner credential's host, and the admin overview projects
    /// ready / not_ready(+reason) / unknown(stale) at read time — no sweeper.
    #[tokio::test]
    async fn postgres_runtime_health_reports_record_scope_and_project() {
        with_isolated_postgres(|store| async move {
            let run = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
                .to_string();
            let host = format!("health-report-host-{run}");
            let launch_code = issue_test_launch_code(&store, "2026-08-24T12:00:00Z").await;
            store
                .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                    id: format!("artifact-health-report-{run}"),
                    kind: RuntimeArtifactKind::OciImage,
                    reference: format!(
                        "ghcr.io/finitecomputer/finite-agent-runtime:health-report@sha256:{}",
                        "7".repeat(64)
                    ),
                    version_label: "health-report-v1".to_string(),
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
                    verified_email: format!("health-report-owner-{run}@finite.vip"),
                    workos_user_id: format!("workos_health_report_owner_{run}"),
                    display_name: "Health Report Agent".to_string(),
                    launch_code,
                    idempotency_key: format!("health-report-{run}"),
                    now: None,
                })
                .await
                .unwrap();
            let lease = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: format!("runner-health-{run}"),
                    source_host_id: None,
                    lease_token: format!("lease-health-{run}"),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: None,
                })
                .await
                .unwrap()
                .expect("health report request should lease");
            assert_eq!(lease.request.id, created.request.id);
            let completed = store
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: lease.request.id.clone(),
                    runner_id: format!("runner-health-{run}"),
                    lease_token: format!("lease-health-{run}"),
                    source_host_id: host.clone(),
                    source_machine_id: format!("health-report-agent-{run}"),
                    runtime_artifact_id: Some(format!("artifact-health-report-{run}")),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: Some("http://127.0.0.1:41001/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("Health Report Agent".to_string()),
                    hostname: None,
                    runtime_host: Some("http://127.0.0.1:41001".to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: None,
                    hermes_available: Some(true),
                    published_app_urls: Vec::new(),
                    now: None,
                })
                .await
                .unwrap();
            let runtime_id = completed.request.agent_runtime_id.clone().unwrap();
            async fn overview_health(
                store: &CoreStore,
                runtime_id: &str,
            ) -> crate::RuntimeHealthProjection {
                store
                    .admin_runtime_overviews()
                    .await
                    .unwrap()
                    .into_iter()
                    .find(|overview| overview.agent_runtime_id == runtime_id)
                    .unwrap()
                    .runtime_health
            }
            let report =
                |ready: bool, reason: Option<&str>, observed_at: &str, now: Option<&str>| {
                    RecordRuntimeHealthReportInput {
                        source_host_id: host.clone(),
                        agent_runtime_id: runtime_id.clone(),
                        ready,
                        reason: reason.map(str::to_string),
                        observed_at: observed_at.to_string(),
                        agent_npub: Some(format!("npub1{}", "q".repeat(58))),
                        report_interval_seconds: Some(60),
                        now: now.map(str::to_string),
                    }
                };

            // No report yet: the named unknown state, never a frozen ready.
            let health = overview_health(&store, &runtime_id).await;
            assert_eq!(health.status, crate::RuntimeHealthStatus::Unknown);
            assert_eq!(health.reported_at, None);

            // A fresh ready report projects ready with the pinned npub as
            // anti-squat evidence.
            let ack = store
                .record_runtime_health_report(report(true, None, "2026-08-24T11:59:00Z", None))
                .await
                .unwrap();
            assert_eq!(ack.agent_runtime_id, runtime_id);
            let health = overview_health(&store, &runtime_id).await;
            assert_eq!(health.status, crate::RuntimeHealthStatus::Ready);
            assert_eq!(
                health.agent_npub.as_deref(),
                Some(format!("npub1{}", "q".repeat(58)).as_str())
            );

            // A fresh not-ready report surfaces its reason.
            store
                .record_runtime_health_report(report(
                    false,
                    Some("model endpoint 503"),
                    "2026-08-24T12:00:00Z",
                    None,
                ))
                .await
                .unwrap();
            let health = overview_health(&store, &runtime_id).await;
            assert_eq!(health.status, crate::RuntimeHealthStatus::NotReady);
            assert_eq!(health.reason.as_deref(), Some("model endpoint 503"));

            // A report recorded long ago (runner stopped reporting) crosses
            // the 3x cadence deadline and projects unknown again.
            store
                .record_runtime_health_report(report(
                    true,
                    None,
                    "2020-01-01T00:00:00Z",
                    Some("2020-01-01T00:00:00Z"),
                ))
                .await
                .unwrap();
            let health = overview_health(&store, &runtime_id).await;
            assert_eq!(health.status, crate::RuntimeHealthStatus::Unknown);

            // Scope: the credential's host guards the write. Another host's
            // runtime id and an unknown id both fail closed as not-found.
            let wrong_host = RecordRuntimeHealthReportInput {
                source_host_id: format!("other-host-{run}"),
                ..report(true, None, "2026-08-24T12:01:00Z", None)
            };
            assert!(matches!(
                store.record_runtime_health_report(wrong_host).await,
                Err(CoreError::ProjectRuntimeNotFound)
            ));
            let unknown_runtime = RecordRuntimeHealthReportInput {
                agent_runtime_id: format!("runtime-missing-{run}"),
                ..report(true, None, "2026-08-24T12:01:00Z", None)
            };
            assert!(matches!(
                store.record_runtime_health_report(unknown_runtime).await,
                Err(CoreError::ProjectRuntimeNotFound)
            ));

            // Bounded fields reject out-of-shape reports.
            let bad_npub = RecordRuntimeHealthReportInput {
                agent_npub: Some("not-an-npub".to_string()),
                ..report(true, None, "2026-08-24T12:01:00Z", None)
            };
            assert!(matches!(
                store.record_runtime_health_report(bad_npub).await,
                Err(CoreError::InvalidRuntimeHealthReport)
            ));
            let bad_interval = RecordRuntimeHealthReportInput {
                report_interval_seconds: Some(86_400),
                ..report(true, None, "2026-08-24T12:01:00Z", None)
            };
            assert!(matches!(
                store.record_runtime_health_report(bad_interval).await,
                Err(CoreError::InvalidRuntimeHealthReport)
            ));
            let long_reason = RecordRuntimeHealthReportInput {
                reason: Some("x".repeat(crate::MAX_RUNTIME_HEALTH_REPORT_REASON_CHARS + 1)),
                ..report(false, None, "2026-08-24T12:01:00Z", None)
            };
            assert!(matches!(
                store.record_runtime_health_report(long_reason).await,
                Err(CoreError::InvalidRuntimeHealthReport)
            ));
            let bad_observed = RecordRuntimeHealthReportInput {
                observed_at: "not-a-time".to_string(),
                ..report(true, None, "2026-08-24T12:01:00Z", None)
            };
            assert!(matches!(
                store.record_runtime_health_report(bad_observed).await,
                Err(CoreError::InvalidTimestamp)
            ));
        })
        .await;
    }

    /// The migration runs inside CORE_SCHEMA_SQL at every Core startup, so
    /// reapplying it against an already-migrated database must be a no-op.
    #[tokio::test]
    async fn postgres_runtime_health_reports_migration_reapplies_cleanly() {
        with_isolated_postgres(|db| async move {
            let (raw, connection) = tokio_postgres::connect(&db.url, NoTls).await.unwrap();
            let handle = tokio::spawn(async move {
                let _ = connection.await;
            });
            raw.batch_execute(include_str!(
                "../migrations/0022_runtime_health_reports.sql"
            ))
            .await
            .unwrap();
            raw.batch_execute(include_str!(
                "../migrations/0022_runtime_health_reports.sql"
            ))
            .await
            .unwrap();
            drop(raw);
            handle.abort();
        })
        .await;
    }

    /// Fail-closed repair for a Runtime whose destroy stored a verified
    /// retirement receipt but whose offboarding never ran: the compute-absent
    /// attestation, exact binding, and owner must match, an in-flight control
    /// blocks the repair, a missing receipt refuses, and success completes the
    /// normal offboarding boundary (link, membership, relay credential, keys,
    /// departure fact, audit) without touching the receipt row.
    #[tokio::test]
    async fn postgres_admin_offboard_retired_runtime_completes_verified_retirement() {
        with_isolated_postgres(|store| async move {
            let run = "offboard-retired";
            let owner_email = format!("{run}-owner@finite.vip");
            let admin_email = format!("{run}-admin@finite.vip");
            let machine_id = format!("{run}-agent-001");
            let host = "offboard-retired-host";
            let launch_code = issue_test_launch_code(&store, "2026-07-21T12:00:00Z").await;
            store
                .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                    id: "artifact-offboard-retired-v1".to_string(),
                    kind: RuntimeArtifactKind::OciImage,
                    reference: format!(
                        "ghcr.io/finitecomputer/finite-agent-runtime:offboard-retired-v1@sha256:{}",
                        "4".repeat(64)
                    ),
                    version_label: "offboard-retired-v1".to_string(),
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
            let created = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: owner_email.clone(),
                    workos_user_id: format!("workos_{run}_owner"),
                    display_name: "Offboard Retired Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: format!("{run}-submit"),
                    now: None,
                })
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
                .expect("offboard-retired request should lease");
            assert_eq!(lease.request.id, created.request.id);
            let provisioned = store
                .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                    request_id: lease.request.id.clone(),
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("lease-{run}"),
                    source_host_id: Some(host.to_string()),
                    source_machine_id: Some(machine_id.clone()),
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
                    source_machine_id: machine_id.clone(),
                    runtime_artifact_id: Some("artifact-offboard-retired-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: Some("http://127.0.0.1:41002/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("Offboard Retired Agent".to_string()),
                    hostname: None,
                    runtime_host: Some(host.to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: vec!["http://127.0.0.1:41002/contact".to_string()],
                    now: None,
                })
                .await
                .unwrap();
            let runtime_id = completed.request.agent_runtime_id.clone().unwrap();
            let project_id = completed.project.id.clone();

            // Stage the anomaly: a destroy whose verified receipt is stored but
            // whose offboarding never ran (link still active, endpoint set).
            let retirement_capable =
                serde_json::to_string(&RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                    runtime_retirement: true,
                    ..*kata_runtime_capabilities().v1()
                }))
                .unwrap();
            store
                .exec(&format!(
                    "UPDATE agent_runtimes SET runtime_capabilities = '{retirement_capable}'::jsonb \
                     WHERE id = '{runtime_id}'"
                ))
                .await;
            let destroy = store
                .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                    admin_verified_email: admin_email.clone(),
                    admin_workos_user_id: format!("workos_{run}_admin"),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: host.to_string(),
                    expected_source_machine_id: machine_id.clone(),
                    now: Some("2026-07-21T12:01:00Z".to_string()),
                })
                .await
                .unwrap();
            let destroy_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("ctl-destroy-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some(host.to_string()),
                    runner_capacity: Some(crate::RunnerLeaseCapacity {
                        runner_classes: vec![crate::RunnerClass::Kata],
                        runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                            RuntimeCapabilitiesV1 {
                                runtime_retirement: true,
                                ..*kata_runtime_capabilities().v1()
                            },
                        )),
                        ..crate::RunnerLeaseCapacity::default()
                    }),
                    now: Some("2026-07-21T12:01:30Z".to_string()),
                })
                .await
                .unwrap()
                .expect("retirement should lease to a capable Kata runner");
            assert_eq!(destroy_lease.request.id, destroy.id);
            let destroy_spec = runtime_spec_v1(destroy_lease.runtime_spec.as_ref().unwrap());

            let input = |compute_absent: bool| AdminOffboardRetiredRuntimeInput {
                admin_verified_email: admin_email.clone(),
                admin_workos_user_id: format!("workos_{run}_admin"),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: host.to_string(),
                expected_source_machine_id: machine_id.clone(),
                expected_owner_email: owner_email.clone(),
                operator_observed_compute_absent: compute_absent,
                now: Some("2026-07-21T12:05:00Z".to_string()),
            };

            // The compute-absent attestation is required.
            assert!(matches!(
                store
                    .admin_offboard_retired_runtime(input(false))
                    .await
                    .unwrap_err(),
                CoreError::RetiredRuntimeOffboardAcknowledgementRequired
            ));
            assert!(store.active_runtime_for_project(&project_id).await.is_some());

            // An in-flight control operation blocks the repair.
            assert!(matches!(
                store
                    .admin_offboard_retired_runtime(input(true))
                    .await
                    .unwrap_err(),
                CoreError::RuntimeControlOperationConflict
            ));
            // The older Core completed the destroy without offboarding.
            store
                .exec(&format!(
                    "UPDATE runtime_control_requests \
                     SET status = 'stopped', lease_token = NULL, lease_expires_at = NULL, \
                         completed_at = CURRENT_TIMESTAMP \
                     WHERE id = '{}'",
                    destroy.id
                ))
                .await;

            // Without a stored receipt the repair refuses.
            assert!(matches!(
                store
                    .admin_offboard_retired_runtime(input(true))
                    .await
                    .unwrap_err(),
                CoreError::RetiredRuntimeOffboardReceiptMissing
            ));

            // The verified receipt the destroy stored before its offboarding
            // was lost.
            store
                .exec(&format!(
                    "INSERT INTO runtime_retirement_snapshots (
                       request_id, project_id, agent_runtime_id, durable_state_id,
                       runtime_artifact_id, schema_version, backend, locator,
                       zip_bytes, zip_sha256, manifest_sha256, created_at,
                       verified_at, recovery_authority_id, retention_policy, stored_at
                     ) VALUES (
                       '{}', '{}', '{}', '{}',
                       '{}', 'runtime_retirement_snapshot.v1', 'borg', '{}',
                       8192, '{}', '{}', '2026-07-21T12:02:00Z',
                       '2026-07-21T12:03:00Z', 'finite-assisted-test',
                       'indefinite_until_purge', CURRENT_TIMESTAMP
                     )",
                    destroy.id,
                    project_id,
                    runtime_id,
                    destroy_spec.durable_state_id,
                    destroy_spec.runtime_artifact_id,
                    crate::runtime_retirement_archive_locator(&destroy.id),
                    "a".repeat(64),
                    "b".repeat(64),
                ))
                .await;
            let receipt_row_before = store
                .row("runtime_retirement_snapshots", &destroy.id)
                .await
                .expect("staged receipt must read back");

            // The owner and the exact binding must match.
            let mut wrong_owner = input(true);
            wrong_owner.expected_owner_email = "someone-else@finite.vip".to_string();
            assert!(matches!(
                store
                    .admin_offboard_retired_runtime(wrong_owner)
                    .await
                    .unwrap_err(),
                CoreError::RetiredRuntimeOffboardOwnerMismatch
            ));
            let mut wrong_binding = input(true);
            wrong_binding.expected_source_machine_id = "replacement-agent".to_string();
            assert!(matches!(
                store
                    .admin_offboard_retired_runtime(wrong_binding)
                    .await
                    .unwrap_err(),
                CoreError::RuntimeSpecMismatch
            ));

            let receipt = store
                .admin_offboard_retired_runtime(input(true))
                .await
                .unwrap();
            assert_eq!(receipt.project_id, project_id);
            assert_eq!(receipt.agent_runtime_id, runtime_id);
            assert_eq!(receipt.retirement_request_id, destroy.id);
            assert_eq!(
                receipt.retirement_locator,
                crate::runtime_retirement_archive_locator(&destroy.id)
            );
            assert_eq!(receipt.revoked_finite_private_key_count, 1);

            // Offboarding completed: the link is inactive, the membership is
            // archived, and the runtime-scoped key is revoked.
            assert!(store.active_runtime_for_project(&project_id).await.is_none());
            assert!(store.project(&project_id).await.is_some());
            assert!(store.agent_runtime(&runtime_id).await.is_some());
            assert!(
                store
                    .all("project_room_memberships")
                    .await
                    .iter()
                    .any(|membership| {
                        membership["project_id"] == project_id.as_str()
                            && !membership["archived_at"].is_null()
                    })
            );
            let key_after = store
                .finite_private_admin_state()
                .await
                .unwrap()
                .api_keys
                .into_iter()
                .find(|key| key.id == provisioned.api_key.id)
                .unwrap();
            assert_eq!(key_after.status, FinitePrivateApiKeyStatus::Revoked);

            // The receipt row is untouched.
            let receipt_row_after = store
                .row("runtime_retirement_snapshots", &destroy.id)
                .await
                .unwrap();
            assert_eq!(receipt_row_after, receipt_row_before);

            // The repair and the key revocation are audited.
            let events = store.finite_private_admin_audit_events().await.unwrap();
            assert!(events.iter().any(|event| {
                event.action == "runtime.admin_offboard_retired"
                    && event.actor == admin_email
                    && event.target_id == runtime_id
            }));
            assert!(events.iter().any(|event| {
                event.action == "finite_private.runtime.offboard_retired_revoke_keys"
                    && event.target_id == runtime_id
            }));

            // A repair rerun fails closed on the inactive link.
            assert!(matches!(
                store
                    .admin_offboard_retired_runtime(input(true))
                    .await
                    .unwrap_err(),
                CoreError::ProjectRuntimeNotFound
            ));
        })
        .await;
    }

    /// A stageable retirement fixture: a launched, retirement-capable Runtime
    /// with one provisioned runtime key and an enqueued exact destroy. Returns
    /// the project, runtime, destroy request, and leased RuntimeSpec ids the
    /// destroy completion must bind to.
    async fn stage_retirement_in_flight(
        store: &TestDb,
        run: &str,
        host: &str,
    ) -> (String, String, crate::RuntimeControlRequest, String, String) {
        let owner_email = format!("{run}-owner@finite.vip");
        let admin_email = format!("{run}-admin@finite.vip");
        let machine_id = format!("{run}-agent-001");
        let launch_code = issue_test_launch_code(store, "2026-07-21T12:00:00Z").await;
        store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: format!("artifact-{run}-v1"),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/finite-agent-runtime:{run}-v1@sha256:{}",
                    "6".repeat(64)
                ),
                version_label: format!("{run}-v1"),
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
            .request_agent_creation(RequestAgentCreationInput {
                verified_email: owner_email.clone(),
                workos_user_id: format!("workos_{run}_owner"),
                display_name: format!("{run} Agent"),
                launch_code,
                idempotency_key: format!("{run}-submit"),
                now: None,
            })
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
            .expect("creation request should lease");
        let completed = store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: lease.request.id.clone(),
                runner_id: format!("runner-{run}"),
                lease_token: format!("lease-{run}"),
                source_host_id: host.to_string(),
                source_machine_id: machine_id.clone(),
                runtime_artifact_id: Some(format!("artifact-{run}-v1")),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: Some("http://127.0.0.1:41004/contact".to_string()),
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some(format!("{run} Agent")),
                hostname: None,
                runtime_host: Some(host.to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: vec!["http://127.0.0.1:41004/contact".to_string()],
                now: None,
            })
            .await
            .unwrap();
        let runtime_id = completed.request.agent_runtime_id.clone().unwrap();
        let project_id = completed.project.id.clone();
        let retirement_capable =
            serde_json::to_string(&RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                runtime_retirement: true,
                ..*kata_runtime_capabilities().v1()
            }))
            .unwrap();
        store
            .exec(&format!(
                "UPDATE agent_runtimes SET runtime_capabilities = '{retirement_capable}'::jsonb \
                 WHERE id = '{runtime_id}'"
            ))
            .await;
        let destroy = store
            .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                admin_verified_email: admin_email.clone(),
                admin_workos_user_id: format!("workos_{run}_admin"),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: host.to_string(),
                expected_source_machine_id: machine_id.clone(),
                now: Some("2026-07-21T12:01:00Z".to_string()),
            })
            .await
            .unwrap();
        let destroy_lease = store
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: format!("runner-{run}"),
                lease_token: format!("ctl-destroy-{run}"),
                lease_seconds: Some(600),
                source_host_id: Some(host.to_string()),
                runner_capacity: Some(crate::RunnerLeaseCapacity {
                    runner_classes: vec![crate::RunnerClass::Kata],
                    runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                        RuntimeCapabilitiesV1 {
                            runtime_retirement: true,
                            ..*kata_runtime_capabilities().v1()
                        },
                    )),
                    ..crate::RunnerLeaseCapacity::default()
                }),
                now: Some("2026-07-21T12:01:30Z".to_string()),
            })
            .await
            .unwrap()
            .expect("retirement should lease to a capable Kata runner");
        assert_eq!(destroy_lease.request.id, destroy.id);
        let destroy_spec = runtime_spec_v1(destroy_lease.runtime_spec.as_ref().unwrap());
        (
            project_id,
            runtime_id,
            destroy,
            destroy_spec.durable_state_id.clone(),
            destroy_spec.runtime_artifact_id.clone(),
        )
    }

    async fn offboarding_phase_of(store: &TestDb, runtime_id: &str) -> serde_json::Value {
        store
            .row("agent_runtimes", runtime_id)
            .await
            .expect("runtime row must read back")["offboarding_phase"]
            .clone()
    }

    /// The destroy lifecycle records every phase forward: enqueue writes
    /// retirement_requested, and one completion transaction carries the
    /// runtime through receipt_verified, compute_removed, and
    /// link_deactivated to the terminal archived. The phase never regresses:
    /// a backward write fails closed and names both phases, while restating
    /// the recorded phase is an idempotent no-op for replayed completions.
    #[tokio::test]
    async fn postgres_destroy_completion_records_forward_only_offboarding_phases() {
        with_isolated_postgres(|store| async move {
            let run = "phase-forward";
            let host = "phase-forward-host";
            let (project_id, runtime_id, destroy, durable_state_id, spec_artifact_id) =
                stage_retirement_in_flight(&store, run, host).await;

            // Enqueueing the destroy recorded the first phase.
            assert_eq!(
                offboarding_phase_of(&store, &runtime_id).await,
                json!("retirement_requested")
            );

            // A duplicate request dedupes to the same destroy and keeps the
            // phase.
            let deduped = store
                .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                    admin_verified_email: format!("{run}-admin@finite.vip"),
                    admin_workos_user_id: format!("workos_{run}_admin"),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: host.to_string(),
                    expected_source_machine_id: format!("{run}-agent-001"),
                    now: Some("2026-07-21T12:01:20Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(deduped.id, destroy.id);
            assert_eq!(
                offboarding_phase_of(&store, &runtime_id).await,
                json!("retirement_requested")
            );

            let receipt = RuntimeRetirementSnapshotReceipt {
                schema: crate::RUNTIME_RETIREMENT_SNAPSHOT_SCHEMA.to_string(),
                request_id: destroy.id.clone(),
                project_id: project_id.clone(),
                agent_runtime_id: runtime_id.clone(),
                durable_state_id,
                runtime_artifact_id: spec_artifact_id,
                backend: crate::RUNTIME_RETIREMENT_BACKEND_BORG.to_string(),
                locator: crate::runtime_retirement_archive_locator(&destroy.id),
                zip_bytes: 8192,
                zip_sha256: "a".repeat(64),
                manifest_sha256: "b".repeat(64),
                created_at: "2026-07-21T12:02:00Z".to_string(),
                verified_at: "2026-07-21T12:03:00Z".to_string(),
                recovery_authority_id: "finite-assisted-test".to_string(),
                retention_policy: crate::RUNTIME_RETIREMENT_RETENTION_INDEFINITE.to_string(),
            };
            let completion = CompleteRuntimeControlRequestInput {
                request_id: destroy.id.clone(),
                runner_id: format!("runner-{run}"),
                lease_token: format!("ctl-destroy-{run}"),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: None,
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: Some(receipt),
                now: Some("2026-07-21T12:04:00Z".to_string()),
            };
            store
                .complete_runtime_control_request(completion.clone())
                .await
                .unwrap();

            // One transaction carried the runtime to the terminal phase.
            assert_eq!(
                offboarding_phase_of(&store, &runtime_id).await,
                json!("archived")
            );
            assert!(
                store
                    .active_runtime_for_project(&project_id)
                    .await
                    .is_none()
            );

            // A replayed identical completion stays idempotent and terminal.
            store
                .complete_runtime_control_request(completion)
                .await
                .expect("identical completion replay must be idempotent");
            assert_eq!(
                offboarding_phase_of(&store, &runtime_id).await,
                json!("archived")
            );

            // The phase never moves backward; restating it is a no-op.
            let client = store.connection().await.unwrap();
            let regression = set_offboarding_phase(
                &**client,
                &runtime_id,
                OffboardingPhase::RetirementRequested,
                "2026-07-21T12:05:00Z",
            )
            .await
            .unwrap_err();
            assert!(matches!(
                regression,
                CoreError::OffboardingPhaseRegression {
                    current: OffboardingPhase::Archived,
                    attempted: OffboardingPhase::RetirementRequested,
                }
            ));
            set_offboarding_phase(
                &**client,
                &runtime_id,
                OffboardingPhase::Archived,
                "2026-07-21T12:05:00Z",
            )
            .await
            .expect("restating the recorded phase must be an idempotent no-op");
            drop(client);
            assert_eq!(
                offboarding_phase_of(&store, &runtime_id).await,
                json!("archived")
            );
        })
        .await;
    }

    /// The half-retired ghost from the audit: the destroy stored a verified
    /// receipt and removed compute under a pre-phase-machine Core, but the
    /// offboarding never ran (link still active). The 0020 backfill classifies
    /// the row from its legacy flags, `runtime-retire-exact` resumes from the
    /// recorded phase instead of minting a new destroy (the uncapped retry
    /// wedge is unrepresentable), and `runtime-offboard-retired-exact`
    /// completes the offboarding through the same phase machine.
    #[tokio::test]
    async fn postgres_retire_exact_resumes_a_partially_retired_runtime_from_its_phase() {
        with_isolated_postgres(|store| async move {
            let run = "phase-resume";
            let host = "phase-resume-host";
            let owner_email = format!("{run}-owner@finite.vip");
            let admin_email = format!("{run}-admin@finite.vip");
            let machine_id = format!("{run}-agent-001");
            let (project_id, runtime_id, destroy, durable_state_id, spec_artifact_id) =
                stage_retirement_in_flight(&store, run, host).await;

            // The legacy ghost: the destroy succeeded and its verified receipt
            // is stored, but the offboarding never ran. Clearing the phase
            // simulates a row written before the phase column existed.
            store
                .exec(&format!(
                    "UPDATE runtime_control_requests \
                     SET status = 'succeeded', lease_token = NULL, lease_expires_at = NULL, \
                         completed_at = CURRENT_TIMESTAMP \
                     WHERE id = '{}'",
                    destroy.id
                ))
                .await;
            store
                .exec(&format!(
                    "INSERT INTO runtime_retirement_snapshots (
                       request_id, project_id, agent_runtime_id, durable_state_id,
                       runtime_artifact_id, schema_version, backend, locator,
                       zip_bytes, zip_sha256, manifest_sha256, created_at,
                       verified_at, recovery_authority_id, retention_policy, stored_at
                     ) VALUES (
                       '{}', '{}', '{}', '{}',
                       '{}', 'runtime_retirement_snapshot.v1', 'borg', '{}',
                       8192, '{}', '{}', '2026-07-21T12:02:00Z',
                       '2026-07-21T12:03:00Z', 'finite-assisted-test',
                       'indefinite_until_purge', CURRENT_TIMESTAMP
                     )",
                    destroy.id,
                    project_id,
                    runtime_id,
                    durable_state_id,
                    spec_artifact_id,
                    crate::runtime_retirement_archive_locator(&destroy.id),
                    "a".repeat(64),
                    "b".repeat(64),
                ))
                .await;
            store
                .exec(&format!(
                    "UPDATE agent_runtimes SET offboarding_phase = NULL WHERE id = '{runtime_id}'"
                ))
                .await;

            // Re-applying the migration maps the legacy flags exactly once:
            // receipt stored plus an active link is compute_removed.
            store
                .exec(include_str!(
                    "../migrations/0020_runtime_offboarding_phases.sql"
                ))
                .await;
            assert_eq!(
                offboarding_phase_of(&store, &runtime_id).await,
                json!("compute_removed")
            );

            // runtime-retire-exact resumes from the recorded phase: it refuses
            // to mint a new destroy and names the resume point instead of
            // looping against the absent container.
            let resume = store
                .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                    admin_verified_email: admin_email.clone(),
                    admin_workos_user_id: format!("workos_{run}_admin"),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: host.to_string(),
                    expected_source_machine_id: machine_id.clone(),
                    now: Some("2026-07-21T12:06:00Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(
                resume,
                CoreError::RuntimeOffboardingResumeRequired {
                    phase: OffboardingPhase::ComputeRemoved,
                }
            ));
            assert_eq!(store.all_runtime_control_requests().await.len(), 1);

            // The owner-facing destroy path is gated by the same phase.
            let owner_resume = store
                .request_runtime_destroy(RequestRuntimeDestroyInput {
                    verified_email: owner_email.clone(),
                    workos_user_id: format!("workos_{run}_owner"),
                    project_id: project_id.clone(),
                    now: Some("2026-07-21T12:06:30Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(
                owner_resume,
                CoreError::RuntimeOffboardingResumeRequired {
                    phase: OffboardingPhase::ComputeRemoved,
                }
            ));
            assert_eq!(store.all_runtime_control_requests().await.len(), 1);

            // The resume command completes the offboarding boundary and the
            // phase machine records the terminal archived phase.
            let receipt = store
                .admin_offboard_retired_runtime(AdminOffboardRetiredRuntimeInput {
                    admin_verified_email: admin_email.clone(),
                    admin_workos_user_id: format!("workos_{run}_admin"),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: host.to_string(),
                    expected_source_machine_id: machine_id.clone(),
                    expected_owner_email: owner_email.clone(),
                    operator_observed_compute_absent: true,
                    now: Some("2026-07-21T12:07:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(receipt.retirement_request_id, destroy.id);
            assert_eq!(
                offboarding_phase_of(&store, &runtime_id).await,
                json!("archived")
            );
            assert!(
                store
                    .active_runtime_for_project(&project_id)
                    .await
                    .is_none()
            );

            // A rerun fails closed and the terminal phase is untouched.
            assert!(matches!(
                store
                    .admin_offboard_retired_runtime(AdminOffboardRetiredRuntimeInput {
                        admin_verified_email: admin_email.clone(),
                        admin_workos_user_id: format!("workos_{run}_admin"),
                        project_id: project_id.clone(),
                        expected_agent_runtime_id: runtime_id.clone(),
                        expected_source_host_id: host.to_string(),
                        expected_source_machine_id: machine_id.clone(),
                        expected_owner_email: owner_email.clone(),
                        operator_observed_compute_absent: true,
                        now: Some("2026-07-21T12:08:00Z".to_string()),
                    })
                    .await
                    .unwrap_err(),
                CoreError::ProjectRuntimeNotFound
            ));
            assert_eq!(
                offboarding_phase_of(&store, &runtime_id).await,
                json!("archived")
            );
        })
        .await;
    }

    /// The unrecoverable-archive boundary crosses no receipt phases, but the
    /// runtime record still lands on the single terminal state.
    #[tokio::test]
    async fn postgres_archive_unrecoverable_records_the_terminal_phase() {
        with_isolated_postgres(|store| async move {
            let run = "phase-archive";
            let owner_email = format!("{run}-owner@finite.vip");
            let admin_email = format!("{run}-admin@finite.vip");
            let machine_id = format!("{run}-agent-001");
            let host = "phase-archive-host";
            let launch_code = issue_test_launch_code(&store, "2026-07-21T12:00:00Z").await;
            store
                .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                    id: format!("artifact-{run}-v1"),
                    kind: RuntimeArtifactKind::OciImage,
                    reference: format!(
                        "ghcr.io/finitecomputer/finite-agent-runtime:{run}-v1@sha256:{}",
                        "7".repeat(64)
                    ),
                    version_label: format!("{run}-v1"),
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
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: owner_email.clone(),
                    workos_user_id: format!("workos_{run}_owner"),
                    display_name: format!("{run} Agent"),
                    launch_code,
                    idempotency_key: format!("{run}-submit"),
                    now: None,
                })
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
                .expect("creation request should lease");
            let completed = store
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: lease.request.id.clone(),
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("lease-{run}"),
                    source_host_id: host.to_string(),
                    source_machine_id: machine_id.clone(),
                    runtime_artifact_id: Some(format!("artifact-{run}-v1")),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: None,
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some(format!("{run} Agent")),
                    hostname: None,
                    runtime_host: Some(host.to_string()),
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
            assert_eq!(
                offboarding_phase_of(&store, &runtime_id).await,
                serde_json::Value::Null
            );

            store
                .admin_archive_unrecoverable_runtime(AdminArchiveUnrecoverableRuntimeInput {
                    admin_verified_email: admin_email.clone(),
                    admin_workos_user_id: format!("workos_{run}_admin"),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: host.to_string(),
                    expected_source_machine_id: machine_id.clone(),
                    expected_owner_email: owner_email.clone(),
                    operator_observed_compute_absent: true,
                    operator_observed_durable_state_absent: true,
                    owner_acknowledged_unrecoverable: true,
                    now: Some("2026-07-21T12:05:00Z".to_string()),
                })
                .await
                .unwrap();
            assert_eq!(
                offboarding_phase_of(&store, &runtime_id).await,
                json!("archived")
            );
        })
        .await;
    }

    /// The read-only pre-deploy census query from the PR body, executed
    /// verbatim against synthetic fixtures: a live runtime lands in the
    /// (no receipt, no active destroy, active link) bucket and a
    /// half-retired ghost in the (receipt, no active destroy, active link)
    /// bucket.
    #[tokio::test]
    async fn postgres_offboarding_census_groups_legacy_flag_combinations() {
        with_isolated_postgres(|store| async move {
            // The half-retired ghost: the destroy succeeded with a stored
            // verified receipt, but its offboarding never ran (link active).
            let (ghost_project_id, ghost_runtime_id, ghost_destroy, _, _) =
                stage_retirement_in_flight(&store, "census-ghost", "census-ghost-host").await;
            store
                .exec(&format!(
                    "UPDATE runtime_control_requests \
                     SET status = 'succeeded', lease_token = NULL, lease_expires_at = NULL, \
                         completed_at = CURRENT_TIMESTAMP \
                     WHERE id = '{}'",
                    ghost_destroy.id
                ))
                .await;
            store
                .exec(&format!(
                    "INSERT INTO runtime_retirement_snapshots (
                       request_id, project_id, agent_runtime_id, durable_state_id,
                       runtime_artifact_id, schema_version, backend, locator,
                       zip_bytes, zip_sha256, manifest_sha256, created_at,
                       verified_at, recovery_authority_id, retention_policy, stored_at
                     ) VALUES (
                       '{}', '{}', '{}', 'census-durable-state',
                       'artifact-census-ghost-v1', 'runtime_retirement_snapshot.v1', 'borg', '{}',
                       8192, '{}', '{}', '2026-07-21T12:02:00Z',
                       '2026-07-21T12:03:00Z', 'finite-assisted-test',
                       'indefinite_until_purge', CURRENT_TIMESTAMP
                     )",
                    ghost_destroy.id,
                    ghost_project_id,
                    ghost_runtime_id,
                    crate::runtime_retirement_archive_locator(&ghost_destroy.id),
                    "a".repeat(64),
                    "b".repeat(64),
                ))
                .await;

            // A live runtime with no offboarding evidence at all. The ghost
            // fixture already promoted the artifact this launch binds.
            let live_launch_code = issue_test_launch_code(&store, "2026-07-21T12:00:00Z").await;
            store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: "census-live-owner@finite.vip".to_string(),
                    workos_user_id: "workos_census_live_owner".to_string(),
                    display_name: "Census Live Agent".to_string(),
                    launch_code: live_launch_code,
                    idempotency_key: "census-live-submit".to_string(),
                    now: None,
                })
                .await
                .unwrap();
            let live_lease = store
                .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                    runner_id: "runner-census-live".to_string(),
                    source_host_id: None,
                    lease_token: "lease-census-live".to_string(),
                    lease_seconds: Some(300),
                    runner_capacity: None,
                    now: None,
                })
                .await
                .unwrap()
                .expect("live creation request should lease");
            store
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: live_lease.request.id.clone(),
                    runner_id: "runner-census-live".to_string(),
                    lease_token: "lease-census-live".to_string(),
                    source_host_id: "census-live-host".to_string(),
                    source_machine_id: "census-live-agent-001".to_string(),
                    runtime_artifact_id: Some("artifact-census-ghost-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: Some("http://127.0.0.1:41006/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("Census Live Agent".to_string()),
                    hostname: None,
                    runtime_host: Some("census-live-host".to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: vec!["http://127.0.0.1:41006/contact".to_string()],
                    now: None,
                })
                .await
                .unwrap();

            // The exact read-only census query shipped in the PR body.
            let client = store.connection().await.unwrap();
            let rows = client
                .query(
                    "SELECT
                       EXISTS (SELECT 1 FROM runtime_retirement_snapshots s
                               WHERE s.agent_runtime_id = r.id) AS has_verified_receipt,
                       EXISTS (SELECT 1 FROM runtime_control_requests c
                               WHERE c.agent_runtime_id = r.id
                                 AND c.kind = 'destroy'
                                 AND c.status IN ('requested', 'running')) AS destroy_request_active,
                       EXISTS (SELECT 1 FROM project_runtime_links l
                               WHERE l.agent_runtime_id = r.id AND l.active) AS link_active,
                       EXISTS (SELECT 1 FROM project_runtime_links l
                               WHERE l.agent_runtime_id = r.id) AS any_link_exists,
                       EXISTS (SELECT 1 FROM project_runtime_links l
                               WHERE l.project_id = r.project_id AND l.active) AS project_has_active_link,
                       count(*) AS runtimes
                     FROM agent_runtimes r
                     GROUP BY 1, 2, 3, 4, 5
                     ORDER BY 1, 2, 3, 4, 5",
                    &[],
                )
                .await
                .unwrap();
            let census: Vec<(bool, bool, bool, bool, bool, i64)> = rows
                .iter()
                .map(|row| {
                    (
                        row.get("has_verified_receipt"),
                        row.get("destroy_request_active"),
                        row.get("link_active"),
                        row.get("any_link_exists"),
                        row.get("project_has_active_link"),
                        row.get("runtimes"),
                    )
                })
                .collect();
            assert_eq!(
                census,
                vec![
                    // The live runtime: no offboarding evidence -> stays NULL.
                    (false, false, true, true, true, 1),
                    // The half-retired ghost: receipt verified, no active
                    // destroy, link still active -> maps to compute_removed.
                    (true, false, true, true, true, 1),
                ]
            );
        })
        .await;
    }

    /// A verified retirement receipt is terminal: once a Runtime's destroy
    /// stored its receipt, no registration path may flip its retired link back
    /// to active. Launch and re-registration of a live Runtime (no receipt)
    /// are unaffected.
    #[tokio::test]
    async fn postgres_verified_retirement_receipt_blocks_link_reactivation() {
        with_isolated_postgres(|store| async move {
            let run = "reactivate-retired";
            let owner_email = format!("{run}-owner@finite.vip");
            let admin_email = format!("{run}-admin@finite.vip");
            let machine_id = format!("{run}-agent-001");
            let host = "reactivate-retired-host";
            let launch_code = issue_test_launch_code(&store, "2026-07-21T12:00:00Z").await;
            store
                .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                    id: "artifact-reactivate-retired-v1".to_string(),
                    kind: RuntimeArtifactKind::OciImage,
                    reference: format!(
                        "ghcr.io/finitecomputer/finite-agent-runtime:reactivate-retired-v1@sha256:{}",
                        "5".repeat(64)
                    ),
                    version_label: "reactivate-retired-v1".to_string(),
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
            let created = store
                .request_agent_creation(RequestAgentCreationInput {
                    verified_email: owner_email.clone(),
                    workos_user_id: format!("workos_{run}_owner"),
                    display_name: "Reactivate Retired Agent".to_string(),
                    launch_code: launch_code.clone(),
                    idempotency_key: format!("{run}-submit"),
                    now: None,
                })
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
                .expect("reactivate-retired request should lease");
            let completed = store
                .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                    request_id: lease.request.id.clone(),
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("lease-{run}"),
                    source_host_id: host.to_string(),
                    source_machine_id: machine_id.clone(),
                    runtime_artifact_id: Some("artifact-reactivate-retired-v1".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    provider_runtime_handle: None,
                    contact_endpoint: Some("http://127.0.0.1:41003/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("Reactivate Retired Agent".to_string()),
                    hostname: None,
                    runtime_host: Some(host.to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: Some("finite-private".to_string()),
                    hermes_available: Some(true),
                    published_app_urls: vec!["http://127.0.0.1:41003/contact".to_string()],
                    now: None,
                })
                .await
                .unwrap();
            let runtime_id = completed.request.agent_runtime_id.clone().unwrap();
            let project_id = completed.project.id.clone();

            // Launch with no receipt activates the link.
            assert!(store.active_runtime_for_project(&project_id).await.is_some());

            // Re-registration of the live Runtime (a runner retry while the
            // request is still launching) is unaffected.
            let register_input = |lease_token: String, now: &str| RegisterAgentCreationRuntimeInput {
                request_id: created.request.id.clone(),
                runner_id: format!("runner-{run}"),
                lease_token,
                source_host_id: host.to_string(),
                source_machine_id: machine_id.clone(),
                runtime_artifact_id: Some("artifact-reactivate-retired-v1".to_string()),
                state_schema_version: Some("state-v1".to_string()),
                provider_runtime_handle: None,
                contact_endpoint: None,
                runtime_capabilities: Some(kata_runtime_capabilities()),
                display_name: Some("Reactivate Retired Agent".to_string()),
                hostname: None,
                runtime_host: Some(host.to_string()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: Some("finite-private".to_string()),
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                now: Some(now.to_string()),
            };
            store
                .exec(&format!(
                    "UPDATE agent_creation_requests \
                     SET status = 'launching', lease_token = 'lease-reactivate-{run}', \
                         lease_expires_at = NULL \
                     WHERE id = '{}'",
                    created.request.id
                ))
                .await;
            store
                .register_agent_creation_runtime(register_input(
                    format!("lease-reactivate-{run}"),
                    "2026-07-21T12:06:00Z",
                ))
                .await
                .unwrap();
            assert!(store.active_runtime_for_project(&project_id).await.is_some());
            store
                .exec(&format!(
                    "UPDATE agent_creation_requests \
                     SET status = 'running', lease_token = NULL, lease_expires_at = NULL \
                     WHERE id = '{}'",
                    created.request.id
                ))
                .await;

            // Retire the Runtime: a verified receipt is stored and offboarding
            // completes, deactivating the link.
            let retirement_capable =
                serde_json::to_string(&RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
                    runtime_retirement: true,
                    ..*kata_runtime_capabilities().v1()
                }))
                .unwrap();
            store
                .exec(&format!(
                    "UPDATE agent_runtimes SET runtime_capabilities = '{retirement_capable}'::jsonb \
                     WHERE id = '{runtime_id}'"
                ))
                .await;
            let destroy = store
                .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                    admin_verified_email: admin_email.clone(),
                    admin_workos_user_id: format!("workos_{run}_admin"),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: host.to_string(),
                    expected_source_machine_id: machine_id.clone(),
                    now: Some("2026-07-21T12:07:00Z".to_string()),
                })
                .await
                .unwrap();
            let destroy_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("ctl-destroy-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some(host.to_string()),
                    runner_capacity: Some(crate::RunnerLeaseCapacity {
                        runner_classes: vec![crate::RunnerClass::Kata],
                        runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                            RuntimeCapabilitiesV1 {
                                runtime_retirement: true,
                                ..*kata_runtime_capabilities().v1()
                            },
                        )),
                        ..crate::RunnerLeaseCapacity::default()
                    }),
                    now: Some("2026-07-21T12:07:30Z".to_string()),
                })
                .await
                .unwrap()
                .expect("retirement should lease to a capable Kata runner");
            let destroy_spec = runtime_spec_v1(destroy_lease.runtime_spec.as_ref().unwrap());
            store
                .exec(&format!(
                    "UPDATE runtime_control_requests \
                     SET status = 'stopped', lease_token = NULL, lease_expires_at = NULL, \
                         completed_at = CURRENT_TIMESTAMP \
                     WHERE id = '{}'",
                    destroy.id
                ))
                .await;
            store
                .exec(&format!(
                    "INSERT INTO runtime_retirement_snapshots (
                       request_id, project_id, agent_runtime_id, durable_state_id,
                       runtime_artifact_id, schema_version, backend, locator,
                       zip_bytes, zip_sha256, manifest_sha256, created_at,
                       verified_at, recovery_authority_id, retention_policy, stored_at
                     ) VALUES (
                       '{}', '{}', '{}', '{}',
                       '{}', 'runtime_retirement_snapshot.v1', 'borg', '{}',
                       8192, '{}', '{}', '2026-07-21T12:08:00Z',
                       '2026-07-21T12:08:30Z', 'finite-assisted-test',
                       'indefinite_until_purge', CURRENT_TIMESTAMP
                     )",
                    destroy.id,
                    project_id,
                    runtime_id,
                    destroy_spec.durable_state_id,
                    destroy_spec.runtime_artifact_id,
                    crate::runtime_retirement_archive_locator(&destroy.id),
                    "a".repeat(64),
                    "b".repeat(64),
                ))
                .await;
            store
                .admin_offboard_retired_runtime(AdminOffboardRetiredRuntimeInput {
                    admin_verified_email: admin_email.clone(),
                    admin_workos_user_id: format!("workos_{run}_admin"),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: host.to_string(),
                    expected_source_machine_id: machine_id.clone(),
                    expected_owner_email: owner_email.clone(),
                    operator_observed_compute_absent: true,
                    now: Some("2026-07-21T12:08:45Z".to_string()),
                })
                .await
                .unwrap();
            assert!(store.active_runtime_for_project(&project_id).await.is_none());

            // A late registration retry for the retired Runtime fails closed:
            // the whole registration rolls back and the link stays retired.
            store
                .exec(&format!(
                    "UPDATE agent_creation_requests \
                     SET status = 'launching', lease_token = 'lease-reactivate-{run}', \
                         lease_expires_at = NULL \
                     WHERE id = '{}'",
                    created.request.id
                ))
                .await;
            let runtime_row_before = store.row("agent_runtimes", &runtime_id).await.unwrap();
            // The retired Runtime advertises its retirement capability; the
            // retry must match it to reach the link-activation guard.
            let mut retry = register_input(
                format!("lease-reactivate-{run}"),
                "2026-07-21T12:09:00Z",
            );
            retry.runtime_capabilities = Some(RuntimeCapabilitiesEnvelope::V1(
                RuntimeCapabilitiesV1 {
                    runtime_retirement: true,
                    ..*kata_runtime_capabilities().v1()
                },
            ));
            let error = store
                .register_agent_creation_runtime(retry)
                .await
                .unwrap_err();
            assert!(
                matches!(error, CoreError::RuntimeRetirementSnapshotConflict),
                "unexpected error: {error:?}"
            );
            assert!(store.active_runtime_for_project(&project_id).await.is_none());
            assert_eq!(
                store.row("agent_runtimes", &runtime_id).await.unwrap(),
                runtime_row_before,
                "the refused registration must roll back its runtime upsert too"
            );
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
                        placement: RuntimePlacement::for_hosting_tier(HostingTier::Standard),
                        requested_hosting_tier: None,
                        profile_picture_url: None,
                        owner_chat_account_id: None,
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
                    contact_endpoint: Some("http://127.0.0.1:41001/contact".to_string()),
                    runtime_capabilities: Some(kata_runtime_capabilities()),
                    display_name: Some("RC Agent".to_string()),
                    hostname: None,
                    runtime_host: Some(host.to_string()),
                    runtime_status: Some(RuntimeSummaryStatus::Online),
                    active_inference_profile: None,
                    hermes_available: Some(true),
                    published_app_urls: vec!["http://127.0.0.1:41001/contact".to_string()],
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
                    retirement_snapshot: None,
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
            assert_eq!(
                overview_online.runtime_status,
                store
                    .agent_runtime(&runtime_id)
                    .await
                    .unwrap()
                    .host_facts
                    .runtime_status
            );
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
                    hermes_source_ref: Some(
                        "nix:packages.x86_64-linux.hermes-agent-runtime".to_string(),
                    ),
                    finite_platform_plugin_ref: Some("plugin-v2".to_string()),
                    state_schema_version: "state-v1".to_string(),
                    base_image: None,
                    recover_known_good_chat: true,
                    promoted: true,
                    now: None,
                })
                .await
                .unwrap();
            let changed_binding = store
                .admin_request_runtime_upgrade_exact(AdminRuntimeUpgradeExactInput {
                    admin_verified_email: format!("admin-{run}@finite.vip"),
                    admin_workos_user_id: format!("admin-workos-{run}"),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: "runtime-replaced-after-plan".to_string(),
                    expected_source_host_id: host.to_string(),
                    expected_source_machine_id: machine.to_string(),
                    target_runtime_artifact_id: "artifact-rc-v2".to_string(),
                    now: None,
                })
                .await
                .unwrap_err();
            assert!(matches!(changed_binding, CoreError::RuntimeSpecMismatch));
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
            assert_eq!(
                store.runtime_control_request(&upgrade.id).await.unwrap(),
                upgrade,
                "operator polling reads the exact persisted request"
            );
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
            let upgrade_store = store
                .store
                .clone()
                .with_runtime_environment(BTreeMap::from([(
                    "FINITE_BRAIN_SERVER_URL".to_string(),
                    "https://brain.finite.computer".to_string(),
                )]))
                .unwrap()
                .with_runtime_secret_references(vec![
                    "FAL_KEY".to_string(),
                    "XAI_API_KEY".to_string(),
                ])
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
            let upgrade_lease = upgrade_store
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
            assert_eq!(
                runtime_spec_v1(upgrade_lease.runtime_spec.as_ref().unwrap()).secret_references,
                vec!["FINITE_PRIVATE_API_KEY", "FAL_KEY", "XAI_API_KEY"]
            );
            assert_eq!(
                runtime_spec_v1(upgrade_lease.runtime_spec.as_ref().unwrap()).environment,
                BTreeMap::from([(
                    "FINITE_BRAIN_SERVER_URL".to_string(),
                    "https://brain.finite.computer".to_string(),
                )])
            );
            raw.execute(
                "UPDATE runtime_artifacts
                 SET retired_at = GREATEST(clock_timestamp(), promoted_at)
                 WHERE id = 'artifact-rc-v2'",
                &[],
            )
            .await
            .unwrap();
            upgrade_store
                .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                    request_id: upgrade.id.clone(),
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("ctl-upgrade-{run}"),
                    runtime_artifact_id: Some("artifact-rc-v2".to_string()),
                    state_schema_version: Some("state-v1".to_string()),
                    runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                        RuntimeCapabilitiesV1 {
                            recover_known_good_chat: true,
                            runtime_retirement: true,
                            ..*kata_runtime_capabilities().v1()
                        },
                    )),
                    runtime_host: Some("http://127.0.0.1:41002".to_string()),
                    published_app_urls: Some(vec!["http://127.0.0.1:41002/contact".to_string()]),
                    retirement_snapshot: None,
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
            let refreshed_contact_endpoint: Option<String> = raw
                .query_one(
                    "SELECT contact_endpoint FROM agent_runtimes WHERE id = $1",
                    &[&runtime_id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(
                refreshed_contact_endpoint.as_deref(),
                Some("http://127.0.0.1:41002/contact")
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
                upgraded_spec["spec"]["secretReferences"],
                serde_json::json!(["FINITE_PRIVATE_API_KEY", "FAL_KEY", "XAI_API_KEY"])
            );
            assert_eq!(
                upgraded_spec["spec"]["environment"]["FINITE_BRAIN_SERVER_URL"],
                "https://brain.finite.computer"
            );
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

            // Retirement requires a receipt bound to this exact request and
            // RuntimeSpec. Postgres stores that receipt and performs the
            // target-scoped offboarding in the same transaction.
            let changed_retirement_binding = store
                .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                    admin_verified_email: format!("admin-{run}@finite.vip"),
                    admin_workos_user_id: format!("admin-workos-{run}"),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: "runtime-replaced-after-review".to_string(),
                    expected_source_host_id: host.to_string(),
                    expected_source_machine_id: machine.to_string(),
                    now: Some("2026-07-10T12:03:50Z".to_string()),
                })
                .await
                .unwrap_err();
            assert!(matches!(
                changed_retirement_binding,
                CoreError::RuntimeSpecMismatch
            ));
            let destroy = store
                .admin_request_runtime_retire_exact(AdminRuntimeRetireExactInput {
                    admin_verified_email: format!("admin-{run}@finite.vip"),
                    admin_workos_user_id: format!("admin-workos-{run}"),
                    project_id: project_id.clone(),
                    expected_agent_runtime_id: runtime_id.clone(),
                    expected_source_host_id: host.to_string(),
                    expected_source_machine_id: machine.to_string(),
                    now: Some("2026-07-10T12:04:00Z".to_string()),
                })
                .await
                .unwrap();
            let destroy_lease = store
                .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                    runner_id: format!("runner-{run}"),
                    lease_token: format!("ctl-destroy-{run}"),
                    lease_seconds: Some(60),
                    source_host_id: Some(host.to_string()),
                    runner_capacity: Some(crate::RunnerLeaseCapacity {
                        runner_classes: vec![crate::RunnerClass::Kata],
                        runtime_capabilities: Some(RuntimeCapabilitiesEnvelope::V1(
                            RuntimeCapabilitiesV1 {
                                runtime_retirement: true,
                                ..*kata_runtime_capabilities().v1()
                            },
                        )),
                        ..crate::RunnerLeaseCapacity::default()
                    }),
                    now: Some("2026-07-10T12:04:10Z".to_string()),
                })
                .await
                .unwrap()
                .expect("retirement should lease to a capable Kata runner");
            assert_eq!(destroy_lease.request.id, destroy.id);
            let destroy_spec = runtime_spec_v1(destroy_lease.runtime_spec.as_ref().unwrap());
            let receipt = RuntimeRetirementSnapshotReceipt {
                schema: crate::RUNTIME_RETIREMENT_SNAPSHOT_SCHEMA.to_string(),
                request_id: destroy.id.clone(),
                project_id: project_id.clone(),
                agent_runtime_id: runtime_id.clone(),
                durable_state_id: destroy_spec.durable_state_id.clone(),
                runtime_artifact_id: destroy_spec.runtime_artifact_id.clone(),
                backend: crate::RUNTIME_RETIREMENT_BACKEND_BORG.to_string(),
                locator: crate::runtime_retirement_archive_locator(&destroy.id),
                zip_bytes: 8192,
                zip_sha256: "a".repeat(64),
                manifest_sha256: "b".repeat(64),
                created_at: "2026-07-10T12:04:20Z".to_string(),
                verified_at: "2026-07-10T12:04:30Z".to_string(),
                recovery_authority_id: "finite-assisted-test".to_string(),
                retention_policy: crate::RUNTIME_RETIREMENT_RETENTION_INDEFINITE.to_string(),
            };
            let completion = CompleteRuntimeControlRequestInput {
                request_id: destroy.id.clone(),
                runner_id: format!("runner-{run}"),
                lease_token: format!("ctl-destroy-{run}"),
                runtime_artifact_id: None,
                state_schema_version: None,
                runtime_capabilities: None,
                runtime_host: None,
                published_app_urls: None,
                retirement_snapshot: Some(receipt.clone()),
                now: Some("2026-07-10T12:04:40Z".to_string()),
            };
            store
                .complete_runtime_control_request(completion.clone())
                .await
                .unwrap();
            store
                .complete_runtime_control_request(completion)
                .await
                .expect("identical Postgres completion replay must be idempotent");

            let (raw, raw_connection) = tokio_postgres::connect(&store.url, NoTls).await.unwrap();
            let raw_connection = tokio::spawn(async move {
                let _ = raw_connection.await;
            });
            let stored_snapshot = postgres_runtime_retirement_snapshot(&raw, &destroy.id)
                .await
                .unwrap()
                .expect("retirement receipt must be stored");
            assert_eq!(stored_snapshot.receipt, receipt);
            let active_link_count: i64 = raw
                .query_one(
                    "SELECT COUNT(*) FROM project_runtime_links
                     WHERE project_id = $1 AND agent_runtime_id = $2 AND active = TRUE",
                    &[&project_id, &runtime_id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(active_link_count, 0);
            drop(raw);
            raw_connection.abort();

            let visible_after = store
                .visible_projects_for_workos_user(&workos)
                .await
                .unwrap()
                .into_iter()
                .map(|visible| visible.project.id)
                .collect::<BTreeSet<_>>();
            assert_eq!(visible_after, BTreeSet::from([unrelated_project_id]));
            let key_after_destroy = store
                .finite_private_admin_state()
                .await
                .unwrap()
                .api_keys
                .into_iter()
                .find(|key| key.id == provisioned.api_key.id)
                .unwrap();
            assert_eq!(key_after_destroy.status, FinitePrivateApiKeyStatus::Revoked);
        })
        .await;
    }

    /// Row-scoped reconcile + claim against Postgres: reconcile mints an import
    /// candidate resolved by its natural key (source_import_key) with a surrogate
    /// id, a re-reconcile updates the same row, and claim materializes a project +
    /// runtime (fresh surrogate ids) that the owner can then see. Re-claim is
    /// idempotent and a missing candidate id is reported, not fabricated.
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
                                "SELECT {key}::text, core_rfc3339(updated_at) AS updated_at FROM {table} ORDER BY 1"
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
