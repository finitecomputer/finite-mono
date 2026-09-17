use super::*;

impl CoreStore {
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
}

/// Repair a persisted RuntimeSpec that names the durable root by the source
/// machine instead of the Agent Runtime.
///
/// Expand-generation synthesis for a pre-RuntimeSpec Kata runtime once
/// persisted `source_machine_id` as the durable state id, and every operation
/// spec since carried it forward (`runtime_operation_spec_v1` copies the
/// current id). The Runner derives a runtime's durable root from that id
/// alone (work_root/kata/<durable_state_id>) and migrates a machine-named
/// directory it discovers by container name; a spec that names the machine
/// plans the machine-named path as the root, so the one-time migration never
/// runs for exactly the runtime it exists for, and the lifecycle probe reads
/// that bind as a mismatch. The durable state id is the Agent Runtime id, as
/// for every spec Core builds itself.
///
/// The repair happens where the persisted spec is next read into a control
/// lease or a relocation envelope, and is written back in the same
/// transaction so every later read (upgrade and destroy completion, the
/// retirement receipt check) sees the value the Runner was handed. Only the
/// known-wrong shape is touched: an id that is neither the runtime id nor the
/// source machine is not this repair's to guess at and is left as persisted.
pub(super) async fn repair_persisted_runtime_spec<C>(
    client: &C,
    runtime: &AgentRuntime,
    creation_id: &str,
    spec: RuntimeSpecEnvelope,
    now: &str,
) -> CoreResult<RuntimeSpecEnvelope>
where
    C: GenericClient + Sync,
{
    let persisted = runtime_spec_v1(&spec);
    if persisted.durable_state_id == runtime.id
        || persisted.durable_state_id != runtime.source_machine_id
    {
        return Ok(spec);
    }
    let repaired = match spec {
        RuntimeSpecEnvelope::V1(mut spec) => {
            spec.durable_state_id = runtime.id.clone();
            RuntimeSpecEnvelope::V1(spec)
        }
    };
    let value = serde_json::to_value(&repaired).map_err(json_error)?;
    client
        .execute(
            "UPDATE agent_creation_requests
             SET runtime_spec = $2, updated_at = $3::text::timestamptz
             WHERE id = $1",
            &[&creation_id, &value, &now],
        )
        .await
        .map_err(store_error)?;
    Ok(repaired)
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
                let creation_id: String = row.get("id");
                let persisted = serde_json::from_value(value).map_err(json_error)?;
                repair_persisted_runtime_spec(client, &runtime, &creation_id, persisted, &now)
                    .await?
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
                    // Pre-RuntimeSpec Kata launches mounted a directory named
                    // by source_machine_id. That name is NOT a spec input: the
                    // Runner derives a runtime's durable root from its durable
                    // state id alone (work_root/kata/<durable_state_id>) and
                    // treats a machine-named directory as a migration
                    // candidate it discovers by container name and renames,
                    // once, to the runtime-id root. Synthesizing the machine
                    // name here would make the machine-named path the planned
                    // root, so that migration would never run for exactly the
                    // runtime it exists for. The durable state id is the Agent
                    // Runtime id, as for every persisted spec.
                    &runtime.id,
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

pub(super) fn verify_runtime_control_lease(
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

pub(super) async fn verify_postgres_runtime_control_lease_at<C>(
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
