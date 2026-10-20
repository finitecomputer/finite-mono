use super::*;

pub(super) async fn create_agent_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<CreateAgentRequest>,
) -> Result<Json<RequestAgentCreationResult>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    // A launch code grants its own tier. Preserve the Standard default for
    // ordinary signup, while allowing code holders to use the issued tier.
    let requested_hosting_tier = input.hosting_tier.or_else(|| {
        input
            .launch_code
            .trim()
            .is_empty()
            .then_some(crate::HostingTier::Standard)
    });
    Ok(Json(
        state
            .store
            .request_agent_creation_configured(
                RequestAgentCreationInput {
                    verified_email: identity.email,
                    workos_user_id: identity.workos_user_id,
                    display_name: input.display_name,
                    launch_code: input.launch_code,
                    idempotency_key: input.idempotency_key,
                    now: None,
                },
                AgentCreationConfiguration {
                    placement: state.agent_creation_placement,
                    requested_hosting_tier,
                    profile_picture_url: input.profile_picture_url,
                    owner_chat_account_id: input.owner_chat_account_id,
                },
            )
            .await?,
    ))
}

pub(super) async fn request_runtime_restart(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<RuntimeControlRequestView>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    let request = state
        .store
        .request_runtime_restart(RequestRuntimeRestartInput {
            verified_email: identity.email,
            workos_user_id: identity.workos_user_id,
            project_id,
            now: input.now,
        })
        .await?;
    Ok(Json(RuntimeControlRequestView::from(request)))
}

pub(super) async fn request_runtime_recover_known_good_chat(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<RuntimeControlRequestView>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    let request = state
        .store
        .request_runtime_recover_known_good_chat(RequestRuntimeRecoverKnownGoodChatInput {
            verified_email: identity.email,
            workos_user_id: identity.workos_user_id,
            project_id,
            now: input.now,
        })
        .await?;
    Ok(Json(RuntimeControlRequestView::from(request)))
}

pub(super) async fn request_runtime_stop(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<RuntimeControlRequestView>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    let request = state
        .store
        .request_runtime_stop(crate::RequestRuntimeStopInput {
            verified_email: identity.email,
            workos_user_id: identity.workos_user_id,
            project_id,
            now: input.now,
        })
        .await?;
    Ok(Json(RuntimeControlRequestView::from(request)))
}

pub(super) async fn request_runtime_destroy(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<RuntimeControlRequestView>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    if !state.runtime_retirement_enabled {
        return Err(CoreError::RuntimeRetirementNotEnabled.into());
    }
    let request = state
        .store
        .request_runtime_destroy(crate::RequestRuntimeDestroyInput {
            verified_email: identity.email,
            workos_user_id: identity.workos_user_id,
            project_id,
            now: input.now,
        })
        .await?;
    Ok(Json(RuntimeControlRequestView::from(request)))
}

pub(super) async fn cancel_agent_creation_request(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<CancelAgentCreationRequest>,
) -> Result<Json<AgentCreationRequest>, ApiError> {
    require_service_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .cancel_agent_creation_request(CancelAgentCreationRequestInput {
                request_id,
                now: input.now,
            })
            .await?,
    ))
}
