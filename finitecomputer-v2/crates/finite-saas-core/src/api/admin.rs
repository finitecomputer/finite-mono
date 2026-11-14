use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AccountEmailTargetLookup {
    email: String,
}

pub(super) async fn admin_account_email_target(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<AccountEmailTargetLookup>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(state.store.account_email_target(&input.email).await?))
}

pub(super) async fn admin_account_email_operation(
    State(state): State<CoreApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<AccountEmailChangePreview>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(state.store.account_email_operation(&id).await?))
}

pub(super) async fn admin_account_email_change(
    State(state): State<CoreApiState>,
    Path(action): Path<String>,
    headers: HeaderMap,
    Json(request): Json<AccountEmailChangeRequest>,
) -> Result<Json<AccountEmailChangePreview>, ApiError> {
    let admin = require_admin_identity(&state, &headers).await?;
    if action == "preview" {
        return Ok(Json(
            state.store.preview_account_email_change(request).await?,
        ));
    }
    if !matches!(action.as_str(), "prepare" | "complete" | "cancel") {
        return Err(ApiError::not_found("unknown email change action"));
    }
    let target = state
        .auth
        .workos()
        .verified_user(&request.workos_user_id)
        .await
        .map_err(|error| workos_api_error_at("email_change_target", error))?;
    let email = normalize_owner_email(Some(&target.email))
        .ok_or_else(|| ApiError::conflict("target email is invalid"))?;
    let result = match action.as_str() {
        "prepare" => {
            state
                .store
                .prepare_account_email_change(request, &admin.workos_user_id, &email)
                .await?
        }
        "complete" => {
            state
                .store
                .complete_account_email_change(request, &admin.workos_user_id, &email)
                .await?
        }
        _ => {
            state
                .store
                .cancel_account_email_change(request, &admin.workos_user_id, &email)
                .await?
        }
    };
    Ok(Json(result))
}

pub(super) async fn admin_runtimes(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AdminRuntimeOverview>>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(state.store.admin_runtime_overviews().await?))
}

pub(super) async fn admin_list_launch_code_batches(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<LaunchCodeBatchDetails>>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(state.store.list_launch_code_batches().await?))
}

pub(super) async fn admin_issue_launch_code_batch(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<IssueLaunchCodeBatchRequest>,
) -> Result<Response, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    let issued = state
        .store
        .issue_launch_code_batch(IssueLaunchCodeBatchInput {
            name: input.name,
            code_count: input.code_count,
            expires_in_hours: input.expires_in_hours,
            hosting_tier: input.hosting_tier,
            created_by_workos_user_id: identity.workos_user_id,
            now: None,
        })
        .await?;
    let mut response = Json(issued).into_response();
    response.headers_mut().insert(
        "cache-control",
        HeaderValue::from_static("no-store, private"),
    );
    Ok(response)
}

pub(super) async fn admin_revoke_launch_code_batch(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(batch_id): Path<String>,
) -> Result<Json<LaunchCodeBatchDetails>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    Ok(Json(
        state
            .store
            .revoke_launch_code_batch(RevokeLaunchCodeBatchInput {
                batch_id,
                revoked_by_workos_user_id: identity.workos_user_id,
                now: None,
            })
            .await?,
    ))
}

pub(super) async fn admin_request_runtime_restart(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<RuntimeControlRequestView>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    let request = state
        .store
        .admin_request_runtime_restart(AdminRuntimeControlInput {
            admin_verified_email: identity.email,
            admin_workos_user_id: identity.workos_user_id,
            project_id,
            now: input.now,
        })
        .await?;
    Ok(Json(RuntimeControlRequestView::from(request)))
}

pub(super) async fn admin_request_runtime_recover_known_good_chat(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<TimestampRequest>,
) -> Result<Json<RuntimeControlRequestView>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    let request = state
        .store
        .admin_request_runtime_recover_known_good_chat(AdminRuntimeControlInput {
            admin_verified_email: identity.email,
            admin_workos_user_id: identity.workos_user_id,
            project_id,
            now: input.now,
        })
        .await?;
    Ok(Json(RuntimeControlRequestView::from(request)))
}

pub(super) async fn admin_request_runtime_upgrade(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<AdminRuntimeUpgradeRequest>,
) -> Result<Json<RuntimeControlRequestView>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    if !state.runtime_upgrades_enabled {
        return Err(CoreError::RuntimeUpgradeNotEnabled.into());
    }
    let request = state
        .store
        .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
            admin_verified_email: identity.email,
            admin_workos_user_id: identity.workos_user_id,
            project_id,
            target_runtime_artifact_id: input.target_runtime_artifact_id,
            now: input.now,
        })
        .await?;
    Ok(Json(RuntimeControlRequestView::from(request)))
}

/// Operator creation of an ordinary Agent for the account named by
/// `ownerEmail`. Optional fields default to a self-created Standard Agent.
/// The owner's chat account is never taken from the body: Core provisions
/// the owner's WorkOS user and hosted-chat identity itself.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct AdminCreateAgentForOwnerRequest {
    owner_email: String,
    display_name: String,
    reason: String,
    idempotency_key: String,
    #[serde(default)]
    hosting_tier: Option<HostingTier>,
    #[serde(default)]
    runtime_resource_class: Option<crate::RuntimeResourceClass>,
    #[serde(default)]
    target_source_host_id: Option<String>,
    #[serde(default)]
    profile_picture_url: Option<String>,
    /// Required to add an Agent to an account that has already signed in.
    #[serde(default)]
    existing_account_confirmed: bool,
}

pub(super) async fn admin_create_agent_for_owner(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<AdminCreateAgentForOwnerRequest>,
) -> Result<Json<OperatorCreateAgentResult>, ApiError> {
    let identity = require_admin_identity(&state, &headers).await?;
    let provisioner = state.owner_identity.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(
            "operator creation needs the hosted chat device: set FC_HOSTED_WEB_DEVICE_URL and FINITECHAT_HOSTED_API_TOKEN for Core",
        )
    })?;
    if let Some(host) = input.target_source_host_id.as_deref()
        && !state.auth.has_kata_host(&normalize_source_host_id(host)?)
    {
        return Err(ApiError::conflict(
            "target host must match an active host-bound Kata credential",
        ));
    }
    let outcome = crate::owner_identity::operator_create_agent(
        &state.store,
        provisioner,
        OperatorCreateAgentInput {
            admin: OperatorIdentity {
                email: identity.email,
                workos_user_id: identity.workos_user_id,
            },
            surface: OperatorCreationSurface::AdminApi,
            owner_email: input.owner_email,
            display_name: input.display_name,
            reason: input.reason,
            idempotency_key: input.idempotency_key,
            hosting_tier: input.hosting_tier,
            runtime_resource_class: input.runtime_resource_class,
            target_source_host_id: input.target_source_host_id,
            owner_identity: None,
            existing_account_confirmed: input.existing_account_confirmed,
            cli_os_user: None,
            profile_picture_url: input.profile_picture_url,
            placement_override: state.agent_creation_placement,
            now: None,
        },
    )
    .await?;
    Ok(Json(outcome.result))
}

pub(super) async fn admin_operator_agent_creations(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<OperatorAgentCreationListing>>, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok(Json(state.store.operator_agent_creations().await?))
}
