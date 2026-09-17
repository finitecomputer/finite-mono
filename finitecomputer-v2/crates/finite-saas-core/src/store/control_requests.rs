use super::*;

impl CoreStore {
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

pub(super) async fn postgres_enqueue_runtime_control_request_bound<C>(
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

pub(super) async fn postgres_admin_request_runtime_control<C>(
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
