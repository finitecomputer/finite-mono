use super::*;

impl CoreStore {
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
}

pub(super) async fn postgres_runtime_retirement_snapshot<C>(
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
    // The credential belongs to the creation, not the supervised process.
    // Restart/upgrade retain it; stop suspends it until a successful restart.
    // Explicit revocation remains irreversible. Replacement creations revoke
    // their predecessor in the creation-completion transaction instead.
    // Match registration's runtime-before-credential lock order.
    client
        .query_opt(
            "SELECT id FROM agent_runtimes WHERE id=$1 FOR UPDATE",
            &[&agent_runtime_id],
        )
        .await
        .map_err(store_error)?;
    client
        .execute(
            "UPDATE runtime_core_credentials
             SET revoked=revoked OR $2, activated=$3 AND NOT revoked AND NOT $2
             WHERE agent_runtime_id=$1",
            &[
                &agent_runtime_id,
                &destroy,
                &(status == RuntimeSummaryStatus::Online),
            ],
        )
        .await
        .map_err(store_error)?;
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
        if status == RuntimeSummaryStatus::Online {
            // Compute came up again: the stored report spoke for the previous
            // incarnation. The principal is the same runtime's, so the
            // attribution pin stays.
            reset_runtime_health(client, agent_runtime_id, HealthPin::Keep).await?;
        }
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
