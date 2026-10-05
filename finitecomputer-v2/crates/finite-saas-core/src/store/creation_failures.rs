use super::*;
use crate::{CancelRelocationExactInput, RelocationCancelOutcome};

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

    /// Cancel one exact relocation, or release a cancelled one whose target
    /// Runner is gone once the operator attests that target compute is stopped.
    pub async fn cancel_relocation_exact(
        &self,
        input: CancelRelocationExactInput,
    ) -> CoreResult<RelocationCancelOutcome> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let (request, released_on_attestation) =
            postgres_cancel_relocation_exact(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(RelocationCancelOutcome {
            released_on_attestation,
            ..request.into()
        })
    }
}

async fn postgres_fail_agent_creation_request<C>(
    client: &C,
    input: FailAgentCreationRequestInput,
) -> CoreResult<AgentCreationRequest>
where
    C: GenericClient + Sync,
{
    let now = input.now.clone().unwrap_or(current_time_iso()?);
    let failure_message = trim_to_option(Some(&input.failure_message))
        .ok_or(CoreError::MissingAgentCreationFailureMessage)?;
    let request = locked_agent_creation_request(client, &input.request_id).await?;
    if request.relocation.is_some() && request.status == AgentCreationRequestStatus::Cancelled {
        return release_cancelled_relocation(client, &request, &input, &now).await;
    }
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
    if is_relocation {
        revoke_pending_relocation_credential(client, &request.id).await?;
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
    if request.status == AgentCreationRequestStatus::Running
        || (is_relocation
            && request.status == AgentCreationRequestStatus::Cancelled
            && request.lease_token.is_some())
    {
        return Err(CoreError::AgentCreationRequestNotCancellable);
    }
    // Target compute may be running for a launching relocation. Its Runner
    // keeps the lease, and with it the refusal of source controls, until it
    // has stopped or removed that compute and recorded the failure.
    let keep_target_lease =
        is_relocation && request.status == AgentCreationRequestStatus::Launching;
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
    if is_relocation {
        revoke_pending_relocation_credential(client, &request.id).await?;
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
                 runner_id = CASE WHEN $4 THEN runner_id END,
                 lease_token = CASE WHEN $4 THEN lease_token END,
                 lease_expires_at = CASE WHEN $4 THEN lease_expires_at END,
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
            &[&input.request_id, &agent_runtime_id, &now, &keep_target_lease],
        )
        .await
        .map_err(store_error)?;
    agent_creation_request_from_row(&row)
}

/// Returns the request and whether a held lease was released on attestation.
async fn postgres_cancel_relocation_exact<C: GenericClient + Sync>(
    client: &C,
    input: CancelRelocationExactInput,
) -> CoreResult<(AgentCreationRequest, bool)> {
    let now = input.now.unwrap_or(current_time_iso()?);
    let request = locked_agent_creation_request(client, &input.relocation_request_id).await?;
    if request.relocation.is_none()
        || request.agent_runtime_id.as_deref() != Some(input.expected_agent_runtime_id.as_str())
        || request.target_source_host_id.as_deref()
            != Some(input.expected_target_source_host_id.as_str())
    {
        return Err(CoreError::ProviderOperationIdentityMismatch);
    }
    match request.status {
        AgentCreationRequestStatus::Requested | AgentCreationRequestStatus::Launching => {
            let cancelled = postgres_cancel_agent_creation_request(
                client,
                CancelAgentCreationRequestInput {
                    request_id: request.id,
                    now: Some(now),
                },
            )
            .await?;
            Ok((cancelled, false))
        }
        AgentCreationRequestStatus::Cancelled if request.lease_token.is_some() => {
            let expired: bool = client
                .query_one(
                    "SELECT COALESCE(lease_expires_at <= clock_timestamp(), FALSE)
                     FROM agent_creation_requests WHERE id = $1",
                    &[&request.id],
                )
                .await
                .map_err(store_error)?
                .get(0);
            if !input.confirm_target_compute_stopped || !expired {
                tracing::warn!(
                    creation_request_id = request.id,
                    attested = input.confirm_target_compute_stopped,
                    expired,
                    "relocation lease release refused"
                );
                return Err(CoreError::AgentCreationRequestNotCancellable);
            }
            Ok((
                release_relocation_lease(client, &request.id, &now).await?,
                true,
            ))
        }
        _ => Err(CoreError::AgentCreationRequestNotCancellable),
    }
}

/// The target Runner's failure record after a cancel releases the lease that
/// holds source controls closed; the request stays cancelled. A current Runner
/// sends it only after proving target shutdown, which Core cannot verify, and
/// a Runner from before that contract may send it after a failed cleanup.
async fn release_cancelled_relocation<C: GenericClient + Sync>(
    client: &C,
    request: &AgentCreationRequest,
    input: &FailAgentCreationRequestInput,
    now: &str,
) -> CoreResult<AgentCreationRequest> {
    if request.lease_token.is_none() {
        return Err(CoreError::AgentCreationRequestNotLaunching);
    }
    if request.runner_id.as_deref() != Some(input.runner_id.trim())
        || request.lease_token.as_deref() != Some(input.lease_token.trim())
    {
        return Err(CoreError::AgentCreationRequestLeaseConflict);
    }
    release_relocation_lease(client, &request.id, now).await
}

pub(super) async fn release_relocation_lease<C: GenericClient + Sync>(
    client: &C,
    request_id: &str,
    now: &str,
) -> CoreResult<AgentCreationRequest> {
    let row = client
        .query_one(
            "UPDATE agent_creation_requests
             SET lease_token = NULL,
                 lease_expires_at = NULL,
                 updated_at = $2::text::timestamptz
             WHERE id = $1
             RETURNING id, customer_org_id, owner_user_id, project_id, idempotency_key,
                       display_name, runner_class, hosting_tier, placement_runner_class,
                       runtime_resource_class, desired_runtime_artifact_id, runtime_spec, target_source_host_id, relocation_spec,
                       profile_picture_url,
                       owner_chat_account_id,
                       status, requested_launch_code, agent_runtime_id,
                       runner_id, lease_token, core_rfc3339(lease_expires_at) AS lease_expires_at, failure_message,
                       core_rfc3339(created_at) AS created_at, core_rfc3339(updated_at) AS updated_at",
            &[&request_id, &now],
        )
        .await
        .map_err(store_error)?;
    agent_creation_request_from_row(&row)
}

// Only the unbound successor belongs to a failed or cancelled attempt.
// The predecessor stays current for an exact new attempt.
async fn revoke_pending_relocation_credential<C: GenericClient + Sync>(
    client: &C,
    request_id: &str,
) -> CoreResult<()> {
    client
        .execute(
            "UPDATE runtime_core_credentials SET revoked=TRUE, activated=FALSE
         WHERE creation_request_id=$1 AND agent_runtime_id IS NULL",
            &[&request_id],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}
