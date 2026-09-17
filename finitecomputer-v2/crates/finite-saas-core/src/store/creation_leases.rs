use super::*;

impl CoreStore {
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
    let in_flight_capacity = input
        .runner_capacity
        .as_ref()
        .map(crate::in_flight_capacity_bounds)
        .transpose()?
        .flatten();
    let core_in_flight_before = if let Some(capacity) = in_flight_capacity {
        // Serialize the provider-capacity count and the request lease across
        // Core replicas. Row-level SKIP LOCKED alone can admit two different
        // requests after they both observe the same pre-provider count.
        client
            .query_one(
                "SELECT runner_class
                 FROM runner_capacity_fences
                 WHERE runner_class = $1
                 FOR UPDATE",
                &[&capacity.runner_class.as_str()],
            )
            .await
            .map_err(store_error)?;
        let row = client
            .query_one(
                "SELECT COUNT(*)::bigint AS count
                 FROM agent_creation_requests
                 WHERE status = 'launching' AND runner_class = $1",
                &[&capacity.runner_class.as_str()],
            )
            .await
            .map_err(store_error)?;
        u32::try_from(row.get::<_, i64>("count"))
            .map_err(|_| CoreError::InvalidInFlightCapacityReservation)?
    } else {
        0
    };
    let may_reserve_new = in_flight_capacity.is_none_or(|capacity| {
        capacity
            .provider_inventory_count
            .saturating_add(core_in_flight_before)
            < capacity.max_sandbox_count
    });
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
                        (status = 'requested' AND $7::bool)
                        OR (
                          status = 'launching'
                          AND (lease_expires_at IS NULL OR lease_expires_at <= $4::text::timestamptz)
                        )
                      )
                  AND (
                        target_source_host_id IS NULL
                        OR target_source_host_id = $5
                      )
                  AND (
                        target_source_host_id = $5
                        OR NOT EXISTS (
                            SELECT 1 FROM launch_code_host_targets targets
                            WHERE targets.source_host_id = $5
                              AND NOT EXISTS (SELECT 1 FROM launch_host_reservation_releases released
                                              WHERE released.source_host_id = targets.source_host_id)
                        )
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
                &may_reserve_new,
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
    let in_flight_capacity_reservation = if let Some(capacity) = in_flight_capacity {
        let row = client
            .query_one(
                "SELECT COUNT(*)::bigint AS count
                 FROM agent_creation_requests
                 WHERE status = 'launching' AND runner_class = $1",
                &[&capacity.runner_class.as_str()],
            )
            .await
            .map_err(store_error)?;
        let core_in_flight_count = u32::try_from(row.get::<_, i64>("count"))
            .map_err(|_| CoreError::InvalidInFlightCapacityReservation)?;
        Some(crate::in_flight_capacity_reservation(
            &request,
            placement,
            capacity,
            core_in_flight_count,
        )?)
    } else {
        None
    };
    Ok(Some(AgentCreationLease {
        project,
        request,
        provider_operation,
        in_flight_capacity_reservation,
    }))
}

pub(super) fn verify_agent_creation_lease(
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

pub(super) async fn verify_agent_creation_lease_active<C>(
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
