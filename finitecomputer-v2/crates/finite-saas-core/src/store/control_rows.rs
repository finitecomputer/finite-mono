use super::*;

impl CoreStore {
    pub async fn runtime_control_request(
        &self,
        request_id: &str,
    ) -> CoreResult<RuntimeControlRequest> {
        let client = self.connection().await?;
        postgres_runtime_control_request(&**client, request_id).await
    }
}

pub(super) fn runtime_control_request_from_row(row: &Row) -> CoreResult<RuntimeControlRequest> {
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

pub(super) async fn locked_runtime_control_request<C>(
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
pub(super) async fn postgres_active_runtime_for_project<C>(
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
