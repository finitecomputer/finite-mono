use super::*;

pub(super) async fn select_agent_runtime<C>(
    client: &C,
    runtime_id: &str,
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
pub(super) async fn select_agent_runtime_by_source_import_key<C>(
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

pub(super) async fn upsert_agent_runtime_row<C>(
    client: &C,
    runtime: &AgentRuntime,
) -> CoreResult<()>
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

pub(super) async fn ensure_runtime_source_available<C>(
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

pub(super) async fn delete_runtime_rows<C>(client: &C, runtime_id: &str) -> CoreResult<()>
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
