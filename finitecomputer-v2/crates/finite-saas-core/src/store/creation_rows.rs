use super::*;

/// Idempotency lookup by the natural key `(owner_user_id, idempotency_key)` —
/// the same tuple the `agent_creation_requests` UNIQUE constraint enforces. The
/// request's primary key is a surrogate, so dedupe is done by looking the row up
/// here, never by rederiving the id from the idempotency inputs.
pub(super) async fn select_agent_creation_request_by_idempotency<C>(
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

pub(super) async fn locked_agent_creation_request<C>(
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

pub(super) async fn upsert_agent_creation_request_row<C>(
    client: &C,
    request: &AgentCreationRequest,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    upsert_agent_creation_request_row_with_conflict(
        client,
        request,
        AgentCreationInsertConflict::UpsertById,
    )
    .await?;
    Ok(())
}

/// Insert one `agent_creation_requests` row under the given conflict policy.
/// Returns `false` only for `SingleFlight` when a concurrent committed row
/// already satisfies the conflicting unique constraint (the row was NOT
/// written).
pub(super) async fn upsert_agent_creation_request_row_with_conflict<C>(
    client: &C,
    request: &AgentCreationRequest,
    conflict: AgentCreationInsertConflict,
) -> CoreResult<bool>
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
    let statement = match conflict {
        AgentCreationInsertConflict::UpsertById => {
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
               updated_at = EXCLUDED.updated_at"
        }
        AgentCreationInsertConflict::SingleFlight => {
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
             ON CONFLICT DO NOTHING
             RETURNING id"
        }
    };
    let params: [&(dyn tokio_postgres::types::ToSql + Sync); 25] = [
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
    ];
    match conflict {
        AgentCreationInsertConflict::UpsertById => client
            .execute(statement, &params)
            .await
            .map_err(store_error)
            .map(|_| true),
        AgentCreationInsertConflict::SingleFlight => client
            .query_opt(statement, &params)
            .await
            .map_err(store_error)
            .map(|row| row.is_some()),
    }
}

pub(super) async fn update_agent_creation_runtime_registered<C>(
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

pub(super) async fn update_agent_creation_completed<C>(
    client: &C,
    request_id: &str,
    runtime_id: &str,
    now: &str,
) -> CoreResult<AgentCreationRequest>
where
    C: GenericClient + Sync,
{
    // Completion may cross registration and activation in one transaction.
    client
        .execute(
            "UPDATE agent_creation_requests SET agent_runtime_id=$2 WHERE id=$1",
            &[&request_id, &runtime_id],
        )
        .await
        .map_err(store_error)?;
    runtime_credentials::bind_bootstrap(client, request_id).await?;
    // The creation lease is cleared below. Preserve proof that the credential
    // belongs to the exact completing lease in the same transaction.
    let lease: Option<String> = client
        .query_one(
            "SELECT lease_token FROM agent_creation_requests WHERE id=$1",
            &[&request_id],
        )
        .await
        .map_err(store_error)?
        .get(0);
    let lease_hash = lease.as_deref().map(runtime_credentials::digest);
    // Acquire the credential lock before evaluating wall-clock expiry: a
    // wait on concurrent revocation must not extend launch authority.
    client
        .query_opt(
            "SELECT agent_runtime_id FROM runtime_core_credentials
         WHERE agent_runtime_id=$1 FOR UPDATE",
            &[&runtime_id],
        )
        .await
        .map_err(store_error)?;
    // Cold relocation completes through this creation writer, including
    // same-host replacements whose runtime and provider names stay equal.
    // Source identity equality must never revive an older incarnation.
    client
        .execute(
            "UPDATE runtime_core_credentials SET revoked=TRUE
         WHERE agent_runtime_id=$1 AND creation_request_id<>$2",
            &[&runtime_id, &request_id],
        )
        .await
        .map_err(store_error)?;
    client
        .execute(
            "UPDATE runtime_core_credentials
         SET activated = COALESCE(lease_sha256 = $3, FALSE)
             AND EXISTS (SELECT 1 FROM agent_creation_requests
                         WHERE id=$2 AND lease_expires_at > clock_timestamp())
         WHERE agent_runtime_id=$1 AND creation_request_id=$2 AND NOT revoked",
            &[&runtime_id, &request_id, &lease_hash],
        )
        .await
        .map_err(store_error)?;
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
