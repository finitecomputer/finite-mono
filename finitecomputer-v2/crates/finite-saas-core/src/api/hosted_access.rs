use super::*;
use crate::store::hosted_hermes::{HostedReport, SetHostedAccess};
#[cfg(test)]
mod tests;

pub(super) async fn hosted_route_targets(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let credential = require_runner_auth(&state, &headers)?;
    let mut targets = state
        .store
        .hosted_route_targets_for_host(&credential.source_host_id)
        .await?;
    targets.retain(|target| {
        state
            .hosted_hermes_origins
            .location(&credential.source_host_id, &target.runtime_id)
            .base_url
            .is_some()
    });
    Ok(([("cache-control", "no-store")], Json(targets)))
}

pub(super) async fn hosted_access(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(
            state
                .store
                .hosted_access(&id, &identity.workos_user_id)
                .await?,
        ),
    ))
}

pub(super) async fn set_hosted_access(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<SetHostedAccess>,
) -> Result<impl IntoResponse, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    // Configuration availability is not runtime readiness; applied reporting and
    // real native authentication are separate gates on the grant endpoint.
    let host = state
        .store
        .owned_runtime_source_host(&id, &identity.workos_user_id)
        .await?
        .ok_or_else(|| ApiError::not_found("agent runtime was not found"))?;
    if input.enabled
        && state
            .hosted_hermes_origins
            .location(&host, &id)
            .base_url
            .is_none()
    {
        return Err(ApiError::conflict("hosted access is not configured"));
    }
    Ok((
        [("cache-control", "no-store")],
        Json(
            state
                .store
                .set_hosted_access(&id, &identity.workos_user_id, input)
                .await?,
        ),
    ))
}

pub(super) async fn hosted_session(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    let before = state
        .store
        .hosted_login(&id, &identity.workos_user_id, &state.hosted_hermes_origins)
        .await?;
    let grant = state
        .native_sessions
        .grant(&before)
        .await
        .map_err(|_| ApiError::service_unavailable("native Hermes is unavailable"))?;
    // Recheck both account auth and durable assignment/intent after external IO.
    let identity = require_verified_identity(&state, &headers).await?;
    let after = state
        .store
        .hosted_login(&id, &identity.workos_user_id, &state.hosted_hermes_origins)
        .await?;
    if before != after {
        return Err(ApiError::conflict("hosted access changed"));
    }
    Ok(([("cache-control", "no-store")], Json(grant)))
}

#[derive(Clone)]
struct RuntimeState {
    store: CoreStore,
    origins: HostedHermesOrigins,
}

/// Dedicated agent-facing listener. Never mount the private/account router on
/// this listener or expose it by an edge route allowlist.
pub fn runtime_router(store: CoreStore, origins: HostedHermesOrigins) -> Router {
    Router::new()
        .route("/api/core/v1/runtime/environment", get(environment))
        .route("/api/core/v1/runtime/hosted-hermes", get(pull))
        .route("/api/core/v1/runtime/hosted-hermes/report", post(report))
        .layer(axum::middleware::map_response(
            |mut response: Response| async move {
                response
                    .headers_mut()
                    .insert("cache-control", HeaderValue::from_static("no-store"));
                response
            },
        ))
        .with_state(RuntimeState { store, origins })
}

async fn pull(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let token = bearer_token(&headers)
        .ok_or_else(|| ApiError::unauthorized("runtime credential required"))?;
    let desired = state
        .store
        .hosted_desired(&token, &state.origins)
        .await?
        .ok_or_else(|| ApiError::unauthorized("runtime credential rejected"))?;
    Ok(Json(desired))
}

async fn report(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(input): Json<HostedReport>,
) -> Result<impl IntoResponse, ApiError> {
    let token = bearer_token(&headers)
        .ok_or_else(|| ApiError::unauthorized("runtime credential required"))?;
    if !state.store.report_hosted(&token, input).await? {
        return Err(ApiError::unauthorized("runtime credential rejected"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn environment(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let token = bearer_token(&headers)
        .ok_or_else(|| ApiError::unauthorized("runtime credential required"))?;
    let environment = state
        .store
        .runtime_environment(&token)
        .await?
        .ok_or_else(|| ApiError::unauthorized("runtime credential rejected"))?;
    Ok(Json(environment))
}
