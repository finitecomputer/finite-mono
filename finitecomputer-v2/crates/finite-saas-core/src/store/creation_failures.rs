use super::*;

impl CoreStore {
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
