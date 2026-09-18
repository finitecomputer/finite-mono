use super::*;

impl CoreStore {
    pub async fn complete_agent_creation_request(
        &self,
        input: CompleteAgentCreationRequestInput,
    ) -> CoreResult<AgentCreationLease> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_complete_agent_creation_request(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }

    pub async fn register_agent_creation_runtime(
        &self,
        input: RegisterAgentCreationRuntimeInput,
    ) -> CoreResult<AgentCreationLease> {
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        let result = postgres_register_agent_creation_runtime(&*tx, input).await?;
        self.finish(tx).await?;
        Ok(result)
    }
}

async fn postgres_register_agent_creation_runtime<C>(
    client: &C,
    input: RegisterAgentCreationRuntimeInput,
) -> CoreResult<AgentCreationLease>
where
    C: GenericClient + Sync,
{
    let now = input.now.clone().unwrap_or(current_time_iso()?);
    let source_host_id = normalize_source_host_id(&input.source_host_id)?;
    let source_machine_id = normalize_id_part(&input.source_machine_id);
    if source_machine_id.is_empty() {
        return Err(CoreError::MissingSourceMachineId);
    }
    let artifact_id = trim_to_option(input.runtime_artifact_id.as_deref())
        .ok_or(CoreError::MissingRuntimeArtifactId)?;
    let artifact = select_runtime_artifact(client, &artifact_id)
        .await?
        .ok_or(CoreError::RuntimeArtifactNotFound)?;
    ensure_artifact_launchable(&artifact)?;
    let state_schema_version = trim_to_option(input.state_schema_version.as_deref())
        .unwrap_or_else(|| artifact.state_schema_version.clone());
    let request = locked_agent_creation_request(client, &input.request_id).await?;
    verify_agent_creation_lease(&request, &input.runner_id, &input.lease_token)?;
    runtime_credentials::validate_bootstrap_source(
        client,
        &input.request_id,
        &source_host_id,
        &source_machine_id,
    )
    .await?;
    let provider_operation = select_provider_operation(client, &input.request_id).await?;
    let provider_operation_now = provider_operation
        .as_ref()
        .map(|_| current_time_iso())
        .transpose()?;
    if provider_operation_now.is_some() {
        verify_agent_creation_lease_active(client, &request, &input.runner_id, &input.lease_token)
            .await?;
    }
    let project = select_project(client, &request.project_id)
        .await?
        .ok_or_else(|| missing_request_project_error(&request))?;
    let source_import_key = source_import_key(&source_host_id, &source_machine_id);
    ensure_runtime_source_available(client, &source_import_key, &project.id).await?;
    // New-generation requests preallocate the Core runtime id inside their
    // persisted RuntimeSpec. N-1 rows retain source-key adoption semantics.
    let runtime_by_source =
        select_agent_runtime_by_source_import_key(client, &source_import_key).await?;
    let placement = request.placement.or(project.placement).or(runtime_by_source
        .as_ref()
        .and_then(|runtime| runtime.placement));
    let runtime_id = if let Some(runtime_spec) = request.runtime_spec.as_ref() {
        let placement = placement.ok_or(CoreError::RuntimeSpecMismatch)?;
        let spec = runtime_spec_v1(runtime_spec);
        validate_runtime_spec_binding(
            runtime_spec,
            Some(&request.id),
            &project.id,
            &spec.agent_runtime_id,
            placement,
            &artifact,
        )?;
        if request.agent_runtime_id.as_deref() != Some(spec.agent_runtime_id.as_str())
            || runtime_by_source
                .as_ref()
                .is_some_and(|runtime| runtime.id != spec.agent_runtime_id)
        {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        spec.agent_runtime_id.clone()
    } else {
        runtime_by_source
            .as_ref()
            .map(|runtime| runtime.id.clone())
            .map(Ok)
            .unwrap_or_else(new_agent_runtime_id)?
    };
    let runtime_by_id = select_agent_runtime(client, &runtime_id).await?;
    validate_runtime_relocation_registration(
        &request,
        runtime_by_id.as_ref(),
        &source_host_id,
        &source_machine_id,
    )?;
    if request.relocation.is_some() {
        // Relocation registration is deliberately non-mutating. Completion
        // below is the single transaction that replaces the source binding.
        return Ok(AgentCreationLease {
            project,
            request,
            provider_operation,
            in_flight_capacity_reservation: None,
        });
    }
    let existing_runtime = match runtime_by_source {
        Some(runtime) => Some(runtime),
        None => runtime_by_id.filter(|runtime| runtime.source_import_key == source_import_key),
    };
    let (provider_runtime_handle, provider_runtime_handle_history) = merge_provider_runtime_handle(
        existing_runtime.as_ref(),
        input.provider_runtime_handle.clone(),
        placement,
    )?;
    let contact_endpoint = normalize_runtime_contact_endpoint(input.contact_endpoint.as_deref())?
        .or_else(|| existing_runtime.as_ref()?.contact_endpoint.clone());
    let bounded_runtime_capabilities =
        bound_runtime_capabilities_to_artifact(input.runtime_capabilities.clone(), &artifact);
    validate_runtime_capabilities_policy(bounded_runtime_capabilities.as_ref(), placement)?;
    let runtime_capabilities =
        merge_runtime_capabilities(existing_runtime.as_ref(), bounded_runtime_capabilities)?;
    let runtime = AgentRuntime {
        id: runtime_id.clone(),
        project_id: project.id.clone(),
        source_host_id: source_host_id.clone(),
        source_machine_id,
        source_import_key,
        runtime_artifact_id: Some(artifact.id),
        state_schema_version: Some(state_schema_version),
        placement,
        provider_runtime_handle,
        provider_runtime_handle_history,
        contact_endpoint,
        runtime_capabilities,
        host_facts: runtime_host_facts_from_register_input(&input, &request, &source_host_id),
        created_at: existing_runtime
            .map(|runtime| runtime.created_at)
            .unwrap_or_else(|| now.clone()),
        updated_at: now.clone(),
    };
    let updated_provider_operation = provider_operation_at_runtime_boundary(
        provider_operation.as_ref(),
        runtime.provider_runtime_handle.as_ref(),
        false,
        provider_operation_now.as_deref().unwrap_or(&now),
    )?;
    if let Some(operation) = updated_provider_operation.as_ref() {
        persist_provider_operation_delta(
            client,
            provider_operation
                .as_ref()
                .map(|operation| operation.v1().transitions.len())
                .unwrap_or_default(),
            operation,
        )
        .await?;
    }
    let provider_operation_ack = if updated_provider_operation.is_some() {
        select_provider_operation(client, &input.request_id).await?
    } else {
        provider_operation.clone()
    };
    upsert_agent_runtime_row(client, &runtime).await?;
    activate_project_runtime_link(client, &project.id, &runtime_id, &now).await?;
    let request =
        update_agent_creation_runtime_registered(client, &input.request_id, &runtime_id, &now)
            .await?;
    runtime_credentials::bind_bootstrap(client, &input.request_id).await?;
    Ok(AgentCreationLease {
        project,
        request,
        provider_operation: provider_operation_ack,
        in_flight_capacity_reservation: None,
    })
}

async fn postgres_complete_agent_creation_request<C>(
    client: &C,
    input: CompleteAgentCreationRequestInput,
) -> CoreResult<AgentCreationLease>
where
    C: GenericClient + Sync,
{
    let now = input.now.clone().unwrap_or(current_time_iso()?);
    let source_host_id = normalize_source_host_id(&input.source_host_id)?;
    let source_machine_id = normalize_id_part(&input.source_machine_id);
    if source_machine_id.is_empty() {
        return Err(CoreError::MissingSourceMachineId);
    }
    let request = locked_agent_creation_request(client, &input.request_id).await?;
    verify_agent_creation_lease(&request, &input.runner_id, &input.lease_token)?;
    runtime_credentials::validate_bootstrap_source(
        client,
        &input.request_id,
        &source_host_id,
        &source_machine_id,
    )
    .await?;
    let provider_operation = select_provider_operation(client, &input.request_id).await?;
    let provider_operation_now = provider_operation
        .as_ref()
        .map(|_| current_time_iso())
        .transpose()?;
    if provider_operation_now.is_some() {
        verify_agent_creation_lease_active(client, &request, &input.runner_id, &input.lease_token)
            .await?;
    }
    let existing_runtime = match request.agent_runtime_id.as_deref() {
        Some(runtime_id) => select_agent_runtime(client, runtime_id).await?,
        None => None,
    };
    let artifact_id = trim_to_option(input.runtime_artifact_id.as_deref())
        .or_else(|| existing_runtime.as_ref()?.runtime_artifact_id.clone())
        .ok_or(CoreError::MissingRuntimeArtifactId)?;
    let artifact = select_runtime_artifact(client, &artifact_id)
        .await?
        .ok_or(CoreError::RuntimeArtifactNotFound)?;
    ensure_artifact_launchable(&artifact)?;
    let state_schema_version = trim_to_option(input.state_schema_version.as_deref())
        .or_else(|| existing_runtime.as_ref()?.state_schema_version.clone())
        .unwrap_or_else(|| artifact.state_schema_version.clone());
    let project = select_project(client, &request.project_id)
        .await?
        .ok_or_else(|| missing_request_project_error(&request))?;
    validate_runtime_relocation_registration(
        &request,
        existing_runtime.as_ref(),
        &source_host_id,
        &source_machine_id,
    )?;
    let source_import_key = source_import_key(&source_host_id, &source_machine_id);
    ensure_runtime_source_available(client, &source_import_key, &project.id).await?;
    let runtime_by_source =
        select_agent_runtime_by_source_import_key(client, &source_import_key).await?;
    let placement = request
        .placement
        .or(project.placement)
        .or(existing_runtime
            .as_ref()
            .and_then(|runtime| runtime.placement))
        .or(runtime_by_source
            .as_ref()
            .and_then(|runtime| runtime.placement));
    let runtime_id = if let Some(runtime_spec) = request.runtime_spec.as_ref() {
        let placement = placement.ok_or(CoreError::RuntimeSpecMismatch)?;
        let spec = runtime_spec_v1(runtime_spec);
        validate_runtime_spec_binding(
            runtime_spec,
            Some(&request.id),
            &project.id,
            &spec.agent_runtime_id,
            placement,
            &artifact,
        )?;
        if request.agent_runtime_id.as_deref() != Some(spec.agent_runtime_id.as_str())
            || runtime_by_source
                .as_ref()
                .is_some_and(|runtime| runtime.id != spec.agent_runtime_id)
        {
            return Err(CoreError::RuntimeSpecMismatch);
        }
        spec.agent_runtime_id.clone()
    } else {
        runtime_by_source
            .as_ref()
            .map(|runtime| runtime.id.clone())
            .map(Ok)
            .unwrap_or_else(new_agent_runtime_id)?
    };
    let runtime_by_id = select_agent_runtime(client, &runtime_id).await?;
    let existing_runtime = existing_runtime.or(runtime_by_source).or(runtime_by_id);
    let (provider_runtime_handle, provider_runtime_handle_history) = merge_provider_runtime_handle(
        existing_runtime.as_ref(),
        input.provider_runtime_handle.clone(),
        placement,
    )?;
    let contact_endpoint = normalize_runtime_contact_endpoint(input.contact_endpoint.as_deref())?
        .or_else(|| existing_runtime.as_ref()?.contact_endpoint.clone());
    let bounded_runtime_capabilities =
        bound_runtime_capabilities_to_artifact(input.runtime_capabilities.clone(), &artifact);
    validate_runtime_capabilities_policy(bounded_runtime_capabilities.as_ref(), placement)?;
    let runtime_capabilities =
        merge_runtime_capabilities(existing_runtime.as_ref(), bounded_runtime_capabilities)?;
    let runtime = AgentRuntime {
        id: runtime_id.clone(),
        project_id: project.id.clone(),
        source_host_id: source_host_id.clone(),
        source_machine_id,
        source_import_key,
        runtime_artifact_id: Some(artifact.id),
        state_schema_version: Some(state_schema_version),
        placement,
        provider_runtime_handle,
        provider_runtime_handle_history,
        contact_endpoint,
        runtime_capabilities,
        host_facts: runtime_host_facts_from_complete_input(&input, &request, &source_host_id),
        created_at: existing_runtime
            .map(|runtime| runtime.created_at)
            .unwrap_or_else(|| now.clone()),
        updated_at: now.clone(),
    };
    let updated_provider_operation = provider_operation_at_runtime_boundary(
        provider_operation.as_ref(),
        runtime.provider_runtime_handle.as_ref(),
        true,
        provider_operation_now.as_deref().unwrap_or(&now),
    )?;
    if let Some(operation) = updated_provider_operation.as_ref() {
        let previous_len = provider_operation
            .as_ref()
            .map(|operation| operation.v1().transitions.len())
            .unwrap_or_default();
        // Completion may atomically cross both server-owned boundaries.
        for length in previous_len..operation.v1().transitions.len() {
            let partial = ProviderOperationEnvelope::V1(ProviderOperationV1 {
                agent_creation_request_id: operation.v1().agent_creation_request_id.clone(),
                correlation_id: operation.v1().correlation_id.clone(),
                placement: operation.v1().placement,
                transitions: operation.v1().transitions[..=length].to_vec(),
            });
            persist_provider_operation_delta(client, length, &partial).await?;
        }
    }
    let provider_operation_ack = if updated_provider_operation.is_some() {
        select_provider_operation(client, &input.request_id).await?
    } else {
        provider_operation.clone()
    };
    upsert_agent_runtime_row(client, &runtime).await?;
    // Compute came up (a fresh launch, or the relocated incarnation on its
    // new host): whatever report is stored spoke for the previous one. The
    // pin is the principal this completion knows — the relocation's expected
    // principal, else the one the launch path verified.
    let seed_npub = request
        .relocation
        .as_ref()
        .map(|relocation| relocation.v1().expected_agent_npub.clone())
        .or_else(|| trim_to_option(input.agent_npub.as_deref()))
        .filter(|npub| valid_agent_npub(npub));
    reset_runtime_health(client, &runtime.id, HealthPin::Seed(seed_npub)).await?;
    if let Some(relocation) = request
        .relocation
        .as_ref()
        .map(RuntimeRelocationEnvelope::v1)
    {
        let updated = client
            .execute(
                "UPDATE agent_runtimes
                 SET source_host_id = $2,
                     source_machine_id = $3,
                     source_import_key = $4
                 WHERE id = $1
                   AND source_machine_id = $3
                   AND source_host_id IN ($5, $2)",
                &[
                    &runtime.id,
                    &runtime.source_host_id,
                    &runtime.source_machine_id,
                    &runtime.source_import_key,
                    &relocation.source_host_id,
                ],
            )
            .await
            .map_err(store_error)?;
        if updated != 1 {
            return Err(CoreError::RuntimeSpecMismatch);
        }
    }
    activate_project_runtime_link(client, &project.id, &runtime_id, &now).await?;
    let request =
        update_agent_creation_completed(client, &input.request_id, &runtime_id, &now).await?;
    Ok(AgentCreationLease {
        project,
        request,
        provider_operation: provider_operation_ack,
        in_flight_capacity_reservation: None,
    })
}

fn runtime_host_facts_from_register_input(
    input: &RegisterAgentCreationRuntimeInput,
    request: &AgentCreationRequest,
    source_host_id: &str,
) -> HostOwnedRuntimeFacts {
    HostOwnedRuntimeFacts {
        display_name: trim_to_option(input.display_name.as_deref())
            .unwrap_or_else(|| request.display_name.clone()),
        hostname: trim_to_option(input.hostname.as_deref()),
        runtime_host: trim_to_option(input.runtime_host.as_deref())
            .unwrap_or_else(|| source_host_id.to_string()),
        runtime_status: input
            .runtime_status
            .unwrap_or(RuntimeSummaryStatus::Unknown),
        active_inference_profile: trim_to_option(input.active_inference_profile.as_deref()),
        hermes_available: input.hermes_available,
        published_app_urls: input.published_app_urls.clone(),
    }
}

fn runtime_host_facts_from_complete_input(
    input: &CompleteAgentCreationRequestInput,
    request: &AgentCreationRequest,
    source_host_id: &str,
) -> HostOwnedRuntimeFacts {
    HostOwnedRuntimeFacts {
        display_name: trim_to_option(input.display_name.as_deref())
            .unwrap_or_else(|| request.display_name.clone()),
        hostname: trim_to_option(input.hostname.as_deref()),
        runtime_host: trim_to_option(input.runtime_host.as_deref())
            .unwrap_or_else(|| source_host_id.to_string()),
        runtime_status: input
            .runtime_status
            .unwrap_or(RuntimeSummaryStatus::Unknown),
        active_inference_profile: trim_to_option(input.active_inference_profile.as_deref()),
        hermes_available: input.hermes_available,
        published_app_urls: input.published_app_urls.clone(),
    }
}
