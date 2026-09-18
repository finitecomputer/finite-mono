use super::*;

/// Does `existing` carry exactly this relocation attempt (envelope and target
/// host), making it safe to REUSE instead of inserting another row? Used both
/// by the up-front active-relocation check and by the post-conflict re-read
/// after `ON CONFLICT DO NOTHING` lost a race, so both decision points apply
/// the same identity test.
fn relocation_attempt_matches(
    existing: &AgentCreationRequest,
    relocation: &RuntimeRelocationEnvelope,
    target_source_host_id: &str,
) -> bool {
    existing.relocation.as_ref() == Some(relocation)
        && existing.target_source_host_id.as_deref() == Some(target_source_host_id)
}

pub(super) async fn postgres_admin_request_runtime_relocate_exact<C>(
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
    // A same-host relocation is the recovery lane for a Runtime whose
    // container is gone from a host that is still up: a restart requires the
    // container to exist, so recreating compute against the durable tree
    // where it already lives is a relocation whose source and target host
    // coincide. Only the compute-absent attestation gives that shape a
    // meaning; without it a same-host "relocation" would be a restart under
    // another name and is refused as before. The target Runner proves the
    // absence for itself (no provider record binds the tree, nothing writes
    // into it) before it launches.
    if target_source_host_id == runtime.source_host_id && !input.operator_observed_compute_absent {
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
        if relocation_attempt_matches(&existing, &relocation, target_source_host_id.as_str()) {
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
    let current_spec = repair_persisted_runtime_spec(
        client,
        &runtime,
        &current_creation.id,
        current_creation
            .runtime_spec
            .clone()
            .ok_or(CoreError::RuntimeSpecMismatch)?,
        &now,
    )
    .await?;
    let artifact_id = runtime
        .runtime_artifact_id
        .as_deref()
        .ok_or(CoreError::MissingRuntimeArtifactId)?;
    let artifact = select_runtime_artifact(client, artifact_id)
        .await?
        .ok_or(CoreError::RuntimeArtifactNotFound)?;
    let request_id = new_agent_creation_request_id()?;
    let runtime_spec = runtime_operation_spec_v1(
        &current_spec,
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
    let target_source_host = target_source_host_id.clone();
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
        relocation: Some(relocation.clone()),
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
    // Single-flight per project: the check above and this insert share one
    // transaction, but two concurrent identical operator attempts can both
    // pass the check before either commits. The insert's ON CONFLICT DO
    // NOTHING defers to the partial unique index
    // `agent_creation_requests_one_active_relocation_per_runtime`, so the
    // loser sees `false` and re-reads the winner's committed row — reusing it
    // when it carries this exact envelope and target, refusing otherwise —
    // instead of erroring on the unique violation or inserting a second
    // active row.
    let inserted = upsert_agent_creation_request_row_with_conflict(
        client,
        &request,
        AgentCreationInsertConflict::SingleFlight,
    )
    .await?;
    if !inserted {
        let winner = client
            .query_opt(active_sql, &[&runtime.id])
            .await
            .map_err(store_error)?
            .ok_or(CoreError::RuntimeControlOperationConflict)?;
        let existing = agent_creation_request_from_row(&winner)?;
        if relocation_attempt_matches(&existing, &relocation, target_source_host.as_str()) {
            return Ok(existing);
        }
        return Err(CoreError::RuntimeControlOperationConflict);
    }
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
