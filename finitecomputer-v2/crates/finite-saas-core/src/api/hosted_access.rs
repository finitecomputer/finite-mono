use super::*;
use crate::store::hosted_hermes::{HostedReport, SetHostedAccess};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::{access_token_with_subject, core_auth};
    use crate::store::runtime_credentials::tests::{complete, provision, register, requested};
    use crate::test_support::with_isolated_postgres;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;
    #[tokio::test]
    async fn hosted_api_allows_nonadmin_owner_and_separates_runtime_listener() {
        with_isolated_postgres(|db| async move {
            let request = requested(&db).await;
            let secret = db
                .provision_runtime_credential(provision(&request))
                .await
                .unwrap()
                .secret;
            let runtime = register(&db, &request).await;
            complete(&db, &request).await.unwrap();
            let origins =
                HostedHermesOrigins::from_json(r#"{"auth-host":"https://agent.example.test"}"#)
                    .unwrap();
            let app = router_with_hosted_hermes_origins(
                db.store.clone(),
                core_auth("service", "runner", "usage"),
                None,
                origins.clone(),
            );
            let agents = runtime_router(db.store.clone(), origins);
            let owner = access_token_with_subject(
                "runtime-auth-user",
                "runtime-auth@finite.test",
                true,
                None,
            );
            let other = access_token_with_subject("other-user", "other@finite.test", true, None);
            let path = format!("/api/core/v1/me/runtimes/{runtime}/hosted-access");
            for (token, status) in [
                (&owner, StatusCode::OK),
                (&other, StatusCode::NOT_FOUND),
                (&secret, StatusCode::UNAUTHORIZED),
            ] {
                let response = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri(&path)
                            .header("authorization", format!("Bearer {token}"))
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), status);
            }
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("PUT")
                        .uri(&path)
                        .header("authorization", format!("Bearer {owner}"))
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"enabled":true,"expectedGeneration":1}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["cache-control"], "no-store");
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 32768).await.unwrap())
                    .unwrap();
            assert_eq!(body["enabled"], true);
            assert!(body.get("password").is_none());
            let pull = "/api/core/v1/runtime/hosted-hermes";
            for (router, path, token, status) in [
                (&agents, pull, &secret, StatusCode::OK),
                (&agents, pull, &owner, StatusCode::UNAUTHORIZED),
                (&app, pull, &secret, StatusCode::NOT_FOUND),
                (&agents, path.as_str(), &owner, StatusCode::NOT_FOUND),
                (
                    &agents,
                    "/api/core/v1/agent-creation-requests",
                    &secret,
                    StatusCode::NOT_FOUND,
                ),
            ] {
                let response = router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri(path)
                            .header("authorization", format!("Bearer {token}"))
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), status);
            }
            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!(
                            "/api/core/v1/me/runtimes/{runtime}/hosted-hermes-session"
                        ))
                        .header("authorization", format!("Bearer {owner}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CONFLICT); // application has not reported
        })
        .await;
    }
}
