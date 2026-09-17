use super::*;

// No Debug/Serialize: lease credentials must not become loggable response data.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProvisionRuntimeCredentialRequest {
    runner_id: String,
    lease_token: String,
}

pub(super) async fn provision_runtime_credential(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<ProvisionRuntimeCredentialRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    let result = state
        .store
        .provision_runtime_credential(
            crate::store::runtime_credentials::ProvisionRuntimeCredential {
                creation_request_id: request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                source_host_id: credential.source_host_id,
            },
        )
        .await?;
    Ok(([("cache-control", "no-store")], Json(result)))
}

pub(super) async fn provision_upgrade_credential(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(input): Json<ProvisionRuntimeCredentialRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    authorize_runner_id(&credential, &input.runner_id)?;
    let result = state
        .store
        .provision_upgrade_credential(
            crate::store::runtime_credentials::ProvisionUpgradeCredential {
                request_id,
                runner_id: input.runner_id,
                lease_token: input.lease_token,
                source_host_id: credential.source_host_id,
            },
        )
        .await?;
    Ok(([("cache-control", "no-store")], Json(result)))
}

pub(super) async fn hosted_hermes_location(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(runtime_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // Native access uses the same current account/Project authority as the app.
    let identity = require_verified_identity(&state, &headers).await?;
    let source_host = state
        .store
        .owned_runtime_source_host(&runtime_id, &identity.workos_user_id)
        .await?
        .ok_or_else(|| ApiError::not_found("agent runtime was not found"))?;
    let location: HostedHermesLocation = state
        .hosted_hermes_origins
        .location(&source_host, &runtime_id);
    Ok(([("cache-control", "no-store")], Json(location)))
}

pub(super) async fn resolve_runtime_route(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(identifier): Path<String>,
) -> Result<Json<RuntimeRouteResolution>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    state
        .store
        .link_verified_user(LinkVerifiedUserInput {
            verified_email: identity.email,
            workos_user_id: identity.workos_user_id.clone(),
            now: None,
        })
        .await?;
    let identifier = identifier.trim();
    let resolution = state
        .store
        .visible_projects_for_workos_user(&identity.workos_user_id)
        .await?
        .into_iter()
        .find_map(|project| {
            // Legacy-row guard: imported-bridge projects (see
            // `public_visible_projects`) are not routable product surfaces.
            if project.project.import_candidate_id.is_some() {
                return None;
            }
            let runtime = project.runtime?;
            (project.project.id == identifier
                || runtime.id == identifier
                || runtime.source_machine_id == identifier)
                .then_some(RuntimeRouteResolution {
                    project_id: project.project.id,
                    runtime_id: runtime.id,
                })
        })
        .ok_or_else(|| ApiError::not_found("agent runtime was not found"))?;
    Ok(Json(resolution))
}
