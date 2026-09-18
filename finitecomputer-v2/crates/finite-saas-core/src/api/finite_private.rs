use super::*;

pub(super) async fn approve_finite_private_grant(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<ApproveFinitePrivateGrantRequest>,
) -> Result<Json<FinitePrivateGrant>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .approve_finite_private_grant(crate::ApproveFinitePrivateGrantInput {
                verified_email: input.verified_email,
                workos_user_id: input.workos_user_id,
                limit_profile_id: input.limit_profile_id,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn issue_finite_private_api_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(grant_id): Path<String>,
    Json(input): Json<IssueFinitePrivateApiKeyRequest>,
) -> Result<Json<FinitePrivateApiKey>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .issue_finite_private_api_key(IssueFinitePrivateApiKeyInput {
                grant_id,
                raw_key: input.raw_key,
                project_id: input.project_id,
                agent_runtime_id: input.agent_runtime_id,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn revoke_finite_private_grant(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(grant_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<FinitePrivateGrant>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .revoke_finite_private_grant(RevokeFinitePrivateGrantInput {
                grant_id,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn reset_finite_private_usage_window(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(grant_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<FinitePrivateGrant>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .reset_finite_private_usage_window(ResetFinitePrivateUsageWindowInput {
                grant_id,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn revoke_finite_private_api_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<FinitePrivateApiKey>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .revoke_finite_private_api_key(RevokeFinitePrivateApiKeyInput {
                key_id,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn rotate_finite_private_api_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Json(input): Json<RotateFinitePrivateApiKeyRequest>,
) -> Result<Json<FinitePrivateApiKey>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .rotate_finite_private_api_key(RotateFinitePrivateApiKeyInput {
                key_id,
                raw_key: input.raw_key,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn finite_private_admin_audit_events(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<FinitePrivateAdminAuditEvent>>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(state.store.finite_private_admin_audit_events().await?))
}

pub(super) async fn finite_private_admin_state(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<FinitePrivateAdminState>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(state.store.finite_private_admin_state().await?))
}

pub(super) async fn finite_private_admin_request_details(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Query(query): Query<FinitePrivateRequestDiagnosticQuery>,
) -> Result<Json<FinitePrivateRequestDiagnosticPage>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .finite_private_request_diagnostics_page(query)
            .await?,
    ))
}

pub(super) async fn admin_issue_finite_private_friend_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<AdminIssueFinitePrivateFriendKeyRequest>,
) -> Result<Json<AdminIssuedFinitePrivateKeyResponse>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    let raw_api_key = crate::generate_finite_private_api_key()?;
    let AdminIssuedFinitePrivateKey { grant, api_key } = state
        .store
        .admin_issue_finite_private_friend_key(AdminIssueFinitePrivateFriendKeyInput {
            admin_verified_email: identity.email,
            friend_email: input.email,
            limit_profile_id: input.limit_profile_id,
            raw_key: raw_api_key.clone(),
            now: input.now,
        })
        .await?;
    Ok(Json(AdminIssuedFinitePrivateKeyResponse {
        grant: Some(grant),
        api_key,
        raw_api_key,
        raw_api_key_note: RAW_API_KEY_NOTE.to_string(),
    }))
}

pub(super) async fn admin_rotate_finite_private_api_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<AdminIssuedFinitePrivateKeyResponse>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    let raw_api_key = crate::generate_finite_private_api_key()?;
    let api_key = state
        .store
        .admin_rotate_finite_private_api_key(AdminRotateFinitePrivateApiKeyInput {
            admin_verified_email: identity.email,
            key_id,
            raw_key: raw_api_key.clone(),
            now: input.now,
        })
        .await?;
    Ok(Json(AdminIssuedFinitePrivateKeyResponse {
        grant: None,
        api_key,
        raw_api_key,
        raw_api_key_note: RAW_API_KEY_NOTE.to_string(),
    }))
}

pub(super) async fn admin_revoke_finite_private_api_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<FinitePrivateApiKey>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .admin_revoke_finite_private_api_key(AdminRevokeFinitePrivateApiKeyInput {
                admin_verified_email: identity.email,
                key_id,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn admin_reset_finite_private_usage_window(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(grant_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<FinitePrivateGrant>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .admin_reset_finite_private_usage_window(AdminResetFinitePrivateUsageWindowInput {
                admin_verified_email: identity.email,
                grant_id,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn admin_assign_finite_private_limit_profile(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(grant_id): Path<String>,
    Json(input): Json<AdminAssignFinitePrivateLimitProfileRequest>,
) -> Result<Json<FinitePrivateGrant>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .admin_assign_finite_private_limit_profile(AdminAssignFinitePrivateLimitProfileInput {
                admin_verified_email: identity.email,
                grant_id,
                limit_profile_id: input.limit_profile_id,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn finite_private_usage_health(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_finite_private_usage_auth(&state, &headers)?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn reserve_finite_private_usage(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<ReserveFinitePrivateUsageRequest>,
) -> Result<Json<FinitePrivateUsageDecision>, ApiError> {
    require_finite_private_usage_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .reserve_finite_private_usage(ReserveFinitePrivateUsageInput {
                request_id: input.request_id,
                presented_api_key: input.presented_api_key,
                endpoint: input.endpoint,
                model: input.model,
                estimated_prompt_tokens: input.estimated_prompt_tokens,
                estimated_completion_tokens: input.estimated_completion_tokens,
                estimated_usage_units: input.estimated_usage_units,
                usage_formula_version: input.usage_formula_version,
                dashboard_url: input
                    .dashboard_url
                    .unwrap_or_else(|| "https://finite.computer/dashboard".to_string()),
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn settle_finite_private_reservation(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(reservation_id): Path<String>,
    Json(input): Json<SettleFinitePrivateReservationRequest>,
) -> Result<Json<SettleFinitePrivateReservationResult>, ApiError> {
    require_finite_private_usage_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .settle_finite_private_reservation(SettleFinitePrivateReservationInput {
                reservation_id,
                request_id: input.request_id,
                settlement: input.settlement,
                prompt_tokens: input.prompt_tokens,
                completion_tokens: input.completion_tokens,
                usage_units: input.usage_units,
                usage_formula_version: input.usage_formula_version,
                upstream_status: input.upstream_status,
                upstream_error_class: input.upstream_error_class,
                now: input.now,
            })
            .await?,
    ))
}

pub(super) async fn record_finite_private_request_diagnostic(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<RecordFinitePrivateRequestDiagnosticRequest>,
) -> Result<Json<FinitePrivateRequestDiagnostic>, ApiError> {
    require_finite_private_usage_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .record_finite_private_request_diagnostic(RecordFinitePrivateRequestDiagnosticInput {
                reservation_id: input.reservation_id,
                request_id: input.request_id,
                prompt_tokens: input.prompt_tokens,
                completion_tokens: input.completion_tokens,
                first_output_ms: input.first_output_ms,
                first_answer_ms: input.first_answer_ms,
                duration_ms: input.duration_ms,
                termination_reason: input.termination_reason,
                measurement_quality: input.measurement_quality,
                observed_at: input.observed_at,
            })
            .await?,
    ))
}

pub(super) async fn prune_finite_private_request_diagnostics(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_finite_private_usage_auth(&state, &headers)?;
    let deleted = state
        .store
        .prune_finite_private_request_diagnostics()
        .await?;
    Ok(Json(json!({ "deleted": deleted })))
}

pub(super) async fn finite_private_usage_status_for_api_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<FinitePrivateUsageStatus>, ApiError> {
    let presented_api_key = bearer_token(&headers)
        .ok_or_else(|| ApiError::unauthorized("invalid Finite Private API key"))?;
    let status = state
        .store
        .finite_private_usage_status_for_api_key(&presented_api_key, true, None)
        .await?
        .ok_or_else(|| ApiError::unauthorized("invalid Finite Private API key"))?;
    Ok(Json(status))
}

pub(super) async fn claim_finite_private_daily_reset_for_api_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<FinitePrivateDailyResetResult>, ApiError> {
    let presented_api_key = bearer_token(&headers)
        .ok_or_else(|| ApiError::unauthorized("invalid Finite Private API key"))?;
    Ok(Json(
        state
            .store
            .claim_finite_private_daily_reset_for_api_key(&presented_api_key, None)
            .await
            .map_err(|error| match error {
                CoreError::InvalidFinitePrivateApiKey => {
                    ApiError::unauthorized("invalid Finite Private API key")
                }
                other => ApiError::from(other),
            })?,
    ))
}

pub(super) async fn finite_private_usage_status_for_user(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Option<FinitePrivateUsageStatus>>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .finite_private_usage_status_for_workos_user(&identity.workos_user_id, None)
            .await?,
    ))
}

pub(super) async fn claim_finite_private_daily_reset_for_user(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Option<FinitePrivateDailyResetResult>>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .claim_finite_private_daily_reset_for_workos_user(&identity.workos_user_id, None)
            .await?,
    ))
}

pub(super) async fn provision_finite_private_runtime_key(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<ProvisionFinitePrivateRuntimeKeyRequest>,
) -> Result<Json<ProvisionFinitePrivateRuntimeKeyResult>, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    let source_host_id =
        authorize_runner_source_host(&credential, input.source_host_id.as_deref())?;
    Ok(Json(
        state
            .store
            .provision_finite_private_runtime_key(ProvisionFinitePrivateRuntimeKeyInput {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                source_host_id: Some(source_host_id),
                source_machine_id: input.source_machine_id,
                now: input.now,
            })
            .await?,
    ))
}
