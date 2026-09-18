use super::*;

impl CoreStore {
    pub async fn record_provider_operation_transition(
        &self,
        input: RecordProviderOperationTransitionInput,
    ) -> CoreResult<ProviderOperationEnvelope> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_record_provider_operation_transition(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }
}

async fn postgres_record_provider_operation_transition<C>(
    client: &C,
    input: RecordProviderOperationTransitionInput,
) -> CoreResult<ProviderOperationEnvelope>
where
    C: GenericClient + Sync,
{
    if matches!(
        input.transition,
        ProviderOperationTransition::ProviderHandleRecorded { .. }
            | ProviderOperationTransition::Ready
    ) {
        return Err(CoreError::ProviderOperationBoundaryNotReached);
    }
    let request = locked_agent_creation_request(client, &input.request_id).await?;
    let now = current_time_iso()?;
    verify_agent_creation_lease_active(client, &request, &input.runner_id, &input.lease_token)
        .await?;
    let project = select_project(client, &request.project_id)
        .await?
        .ok_or_else(|| missing_request_project_error(&request))?;
    let placement = request
        .placement
        .or(project.placement)
        .or_else(|| RuntimePlacement::from_legacy_runner_class(request.runner_class))
        .ok_or(CoreError::ProviderOperationIdentityMismatch)?;
    if placement != input.placement {
        return Err(CoreError::ProviderOperationIdentityMismatch);
    }
    let existing = select_provider_operation(client, &input.request_id).await?;
    let previous_len = existing
        .as_ref()
        .map(|operation| operation.v1().transitions.len())
        .unwrap_or_default();
    let updated = append_provider_operation_transition(
        existing.as_ref(),
        &input.request_id,
        &input.correlation_id,
        input.placement,
        input.transition,
        &now,
    )?;
    persist_provider_operation_delta(client, previous_len, &updated).await?;
    select_provider_operation(client, &input.request_id)
        .await?
        .ok_or(CoreError::ProviderOperationTransitionConflict)
}

pub(super) async fn select_provider_operation<C>(
    client: &C,
    request_id: &str,
) -> CoreResult<Option<ProviderOperationEnvelope>>
where
    C: GenericClient + Sync,
{
    let Some(header) = client
        .query_opt(
            "SELECT agent_creation_request_id, schema_name, correlation_id,
                    placement_runner_class, runtime_resource_class
             FROM agent_creation_provider_operations
             WHERE agent_creation_request_id = $1",
            &[&request_id],
        )
        .await
        .map_err(store_error)?
    else {
        return Ok(None);
    };
    let schema_name: String = header.get("schema_name");
    if schema_name != "provider_operation.v1" {
        return Err(CoreError::Store(format!(
            "unsupported provider operation schema {schema_name}"
        )));
    }
    let placement_runner_class: String = header.get("placement_runner_class");
    let runtime_resource_class: String = header.get("runtime_resource_class");
    let placement = RuntimePlacement {
        runner_class: parse_runner_class(&placement_runner_class).ok_or_else(|| {
            CoreError::Store(format!(
                "invalid provider operation runner class {placement_runner_class}"
            ))
        })?,
        runtime_resource_class: parse_runtime_resource_class(&runtime_resource_class).ok_or_else(
            || {
                CoreError::Store(format!(
                    "invalid provider operation resource class {runtime_resource_class}"
                ))
            },
        )?,
    };
    let rows = client
        .query(
            "SELECT sequence, transition, core_rfc3339(recorded_at) AS recorded_at
             FROM agent_creation_provider_operation_transitions
             WHERE agent_creation_request_id = $1
             ORDER BY sequence",
            &[&request_id],
        )
        .await
        .map_err(store_error)?;
    let mut transitions = Vec::with_capacity(rows.len());
    for (expected, row) in rows.into_iter().enumerate() {
        let sequence: i32 = row.get("sequence");
        if sequence != expected as i32 {
            return Err(CoreError::ProviderOperationTransitionConflict);
        }
        let value: Value = row.get("transition");
        transitions.push(ProviderOperationTransitionRecord {
            sequence: sequence as u32,
            transition: serde_json::from_value(value).map_err(json_error)?,
            recorded_at: row.get("recorded_at"),
        });
    }
    Ok(Some(ProviderOperationEnvelope::V1(ProviderOperationV1 {
        agent_creation_request_id: header.get("agent_creation_request_id"),
        correlation_id: header.get("correlation_id"),
        placement,
        transitions,
    })))
}

pub(super) async fn persist_provider_operation_delta<C>(
    client: &C,
    previous_len: usize,
    operation: &ProviderOperationEnvelope,
) -> CoreResult<()>
where
    C: GenericClient + Sync,
{
    let operation = operation.v1();
    let Some(last) = operation.transitions.last() else {
        return Err(CoreError::ProviderOperationTransitionConflict);
    };
    if operation.transitions.len() == previous_len {
        return Ok(());
    }
    if operation.transitions.len() != previous_len + 1 || last.sequence as usize != previous_len {
        return Err(CoreError::ProviderOperationTransitionConflict);
    }
    client
        .execute(
            "INSERT INTO agent_creation_provider_operations (
                 agent_creation_request_id, schema_name, correlation_id,
                 placement_runner_class, runtime_resource_class, created_at, updated_at
             ) VALUES ($1, 'provider_operation.v1', $2, $3, $4,
                       $5::text::timestamptz, $5::text::timestamptz)
             ON CONFLICT (agent_creation_request_id) DO UPDATE
             SET updated_at = EXCLUDED.updated_at",
            &[
                &operation.agent_creation_request_id,
                &operation.correlation_id,
                &operation.placement.runner_class.as_str(),
                &operation.placement.runtime_resource_class.as_str(),
                &last.recorded_at,
            ],
        )
        .await
        .map_err(store_error)?;
    let transition = serde_json::to_value(&last.transition).map_err(json_error)?;
    client
        .execute(
            "INSERT INTO agent_creation_provider_operation_transitions (
                 agent_creation_request_id, sequence, transition, recorded_at
             ) VALUES ($1, $2, $3, $4::text::timestamptz)",
            &[
                &operation.agent_creation_request_id,
                &(last.sequence as i32),
                &transition,
                &last.recorded_at,
            ],
        )
        .await
        .map_err(store_error)?;
    Ok(())
}
