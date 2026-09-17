use super::*;

pub(super) async fn runtime_artifact(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(artifact_id): Path<String>,
) -> Result<Json<RuntimeArtifact>, ApiError> {
    let _credential = require_runner_auth(&state, &headers)?;
    let Some(artifact) = state.store.runtime_artifact(&artifact_id).await? else {
        return Err(ApiError::not_found("runtime artifact is not configured"));
    };
    Ok(Json(artifact))
}

/// The runner's standing-readiness ferry (2026-08 audit synthesis, H1 slice
/// 3): one report per live runtime per poll interval, read from the guest's
/// `/contact`. The source host comes from the runner credential, never from
/// the body, so a runner can only report for runtimes on its own host. This
/// is outbound-only telemetry; it carries no command or desired state.
pub(super) async fn report_runtime_health(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<RuntimeHealthReportRequest>,
) -> Result<Json<RuntimeHealthReportAck>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .record_runtime_health_report(RecordRuntimeHealthReportInput {
                source_host_id: credential.source_host_id,
                agent_runtime_id: input.agent_runtime_id,
                ready: input.ready,
                reason: input.reason,
                observed_at: input.observed_at,
                agent_npub: input.agent_npub,
                report_interval_seconds: input.report_interval_seconds,
                now: input.now,
            })
            .await?,
    ))
}

/// The runner's startup registry reconcile: every runtime this runner should
/// be reporting on. Host-scoped by the credential, like reports, so a runner
/// only ever learns about (and polls) its own host's runtimes.
pub(super) async fn runtime_health_targets(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeHealthTargetList>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .runtime_health_targets_for_host(&credential.source_host_id)
            .await?,
    ))
}

pub(super) async fn upsert_runtime_artifact(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(artifact_id): Path<String>,
    Json(input): Json<UpsertRuntimeArtifactRequest>,
) -> Result<Json<RuntimeArtifact>, ApiError> {
    require_service_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: artifact_id,
                kind: input.kind,
                reference: input.reference,
                version_label: input.version_label,
                source_git_sha: input.source_git_sha,
                finitec_version: input.finitec_version,
                hermes_source_ref: input.hermes_source_ref,
                finite_platform_plugin_ref: input.finite_platform_plugin_ref,
                state_schema_version: input.state_schema_version,
                base_image: input.base_image,
                recover_known_good_chat: input.recover_known_good_chat,
                promoted: input.promoted,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn lease_agent_creation_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<LeaseAgentCreationRequest>,
) -> Result<Json<Option<AgentCreationLease>>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    let runner_capacity = authorize_runner_capacity(&credential, input.runner_capacity)?;
    Ok(Json(
        state
            .store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: input.runner_id,
                source_host_id: Some(credential.source_host_id),
                lease_token: input.lease_token,
                lease_seconds: input.lease_seconds,
                runner_capacity: Some(runner_capacity),
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn record_provider_operation_transition(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<RecordProviderOperationTransitionRequest>,
) -> Result<Json<ProviderOperationEnvelope>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    if !credential
        .runner_classes
        .contains(&input.placement.runner_class)
    {
        return Err(runner_binding_error());
    }
    Ok(Json(
        state
            .store
            .record_provider_operation_transition(RecordProviderOperationTransitionInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                correlation_id: input.correlation_id,
                placement: input.placement,
                transition: input.transition,
            })
            .await?,
    ))
}

pub(super) async fn lease_runtime_control_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<LeaseRuntimeControlRequest>,
) -> Result<Json<Option<crate::RuntimeControlLease>>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    let runner_capacity = authorize_runner_capacity(&credential, input.runner_capacity)?;
    let source_host_id =
        authorize_runner_source_host(&credential, input.source_host_id.as_deref())?;
    let lease = state
        .store
        .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
            runner_id: input.runner_id,
            lease_token: input.lease_token,
            lease_seconds: input.lease_seconds,
            source_host_id: Some(source_host_id),
            runner_capacity: Some(runner_capacity),
            now: input.now,
        })
        .await?;
    Ok(Json(lease))
}

pub(super) async fn complete_runtime_control_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<CompleteRuntimeControlRequest>,
) -> Result<Json<crate::RuntimeControlRequest>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    let runtime_capabilities = input
        .runtime_capabilities
        .map(|capabilities| authorize_runner_runtime_capabilities(&credential, Some(capabilities)))
        .transpose()?;
    Ok(Json(
        state
            .store
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                runtime_artifact_id: input.runtime_artifact_id,
                state_schema_version: input.state_schema_version,
                runtime_capabilities,
                runtime_host: input.runtime_host,
                published_app_urls: input.published_app_urls,
                retirement_snapshot: input.retirement_snapshot,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn fail_runtime_control_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<FailRuntimeControlRequest>,
) -> Result<Json<crate::RuntimeControlRequest>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    Ok(Json(
        state
            .store
            .fail_runtime_control_request(FailRuntimeControlRequestInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                failure_message: input.failure_message,
                failure_stage: input.failure_stage,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn renew_runtime_control_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<RenewRuntimeControlRequest>,
) -> Result<Json<crate::RuntimeControlRequest>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    Ok(Json(
        state
            .store
            .renew_runtime_control_request(RenewRuntimeControlRequestInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                lease_seconds: input.lease_seconds,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn retry_runtime_control_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<RetryRuntimeControlRequest>,
) -> Result<Json<crate::RuntimeControlRequest>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    Ok(Json(
        state
            .store
            .retry_runtime_control_request(RetryRuntimeControlRequestInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                failure_message: input.failure_message,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn complete_agent_creation_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<CompleteAgentCreationRequest>,
) -> Result<Json<AgentCreationLease>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    let source_host_id = authorize_runner_source_host(&credential, Some(&input.source_host_id))?;
    authorize_provider_runtime_handle(&credential, input.provider_runtime_handle.as_ref())?;
    let runtime_capabilities =
        authorize_runner_runtime_capabilities(&credential, input.runtime_capabilities)?;
    Ok(Json(
        state
            .store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                source_host_id,
                source_machine_id: input.source_machine_id,
                runtime_artifact_id: input.runtime_artifact_id,
                state_schema_version: input.state_schema_version,
                provider_runtime_handle: input.provider_runtime_handle,
                contact_endpoint: input.contact_endpoint,
                runtime_capabilities: Some(runtime_capabilities),
                display_name: input.display_name,
                hostname: input.hostname,
                runtime_host: input.runtime_host,
                runtime_status: input.runtime_status,
                active_inference_profile: input.active_inference_profile,
                hermes_available: input.hermes_available,
                published_app_urls: input.published_app_urls,
                agent_npub: input.agent_npub,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn register_agent_creation_runtime(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<RegisterAgentCreationRuntimeRequest>,
) -> Result<Json<AgentCreationLease>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    let source_host_id = authorize_runner_source_host(&credential, Some(&input.source_host_id))?;
    authorize_provider_runtime_handle(&credential, input.provider_runtime_handle.as_ref())?;
    let runtime_capabilities =
        authorize_runner_runtime_capabilities(&credential, input.runtime_capabilities)?;
    Ok(Json(
        state
            .store
            .register_agent_creation_runtime(RegisterAgentCreationRuntimeInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                source_host_id,
                source_machine_id: input.source_machine_id,
                runtime_artifact_id: input.runtime_artifact_id,
                state_schema_version: input.state_schema_version,
                provider_runtime_handle: input.provider_runtime_handle,
                contact_endpoint: input.contact_endpoint,
                runtime_capabilities: Some(runtime_capabilities),
                display_name: input.display_name,
                hostname: input.hostname,
                runtime_host: input.runtime_host,
                runtime_status: input.runtime_status,
                active_inference_profile: input.active_inference_profile,
                hermes_available: input.hermes_available,
                published_app_urls: input.published_app_urls,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn fail_agent_creation_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<FailAgentCreationRequest>,
) -> Result<Json<AgentCreationRequest>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    Ok(Json(
        state
            .store
            .fail_agent_creation_request(FailAgentCreationRequestInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                failure_message: input.failure_message,
                provisioned_finite_private_api_key_id: input.provisioned_finite_private_api_key_id,
                now: input.now,
            })
            .await?,
    ))
}
