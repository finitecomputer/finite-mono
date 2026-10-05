//! Private Brain identity listener (FIN-122). These routes are served only by
//! `brain_identity_router` on its own bind address; neither the account
//! router nor the runtime router mounts them.

use super::*;
use crate::brain_identity::{
    BrainAccountObservationRequest, BrainAccountObservationResponse, BrainIdentityConfig,
    BrainIdentityDescriptionsRequest, BrainIdentityDescriptionsResponse, DESCRIPTION_BUDGET,
    DESCRIPTION_CREDENTIAL_HEADER, DESCRIPTIONS_PATH, DESCRIPTIONS_VERSION,
    MAX_DESCRIPTION_BODY_BYTES, MAX_DESCRIPTION_RESPONSE_BYTES, MAX_OBSERVATION_BODY_BYTES,
    OBSERVATION_CREDENTIAL_HEADER, OBSERVATION_ISSUER, OBSERVATION_PATH, OBSERVATION_VERSION,
};
use crate::store::BrainObservationError;
use axum::body::Bytes;
use axum::extract::DefaultBodyLimit;

#[derive(Clone)]
struct BrainIdentityState {
    api: CoreApiState,
    config: BrainIdentityConfig,
}

/// The service-owned private router for Brain identity. Bind it to its own
/// loopback/private address; never merge it into the account router.
pub fn brain_identity_router(
    store: CoreStore,
    auth: CoreAuth,
    config: BrainIdentityConfig,
) -> Router {
    let api = CoreApiState {
        store,
        auth,
        standard_stripe_price_id: None,
        agent_creation_placement: None,
        runtime_upgrades_enabled: false,
        runtime_retirement_enabled: false,
        hosted_hermes_origins: HostedHermesOrigins::default(),
        native_sessions: Default::default(),
    };
    Router::new()
        .route(
            OBSERVATION_PATH,
            post(record_observation).layer(DefaultBodyLimit::max(MAX_OBSERVATION_BODY_BYTES)),
        )
        .route(
            DESCRIPTIONS_PATH,
            post(describe_identities).layer(DefaultBodyLimit::max(MAX_DESCRIPTION_BODY_BYTES)),
        )
        .with_state(BrainIdentityState { api, config })
}

async fn record_observation(
    State(state): State<BrainIdentityState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<BrainAccountObservationResponse>, ApiError> {
    // The issuer credential is checked first and separately from the
    // account bearer, which arrives in `Authorization`.
    if !state.config.observation_credential_matches(
        header_value(&headers, OBSERVATION_CREDENTIAL_HEADER).as_deref(),
    ) {
        return Err(ApiError::unauthorized("invalid observation credential"));
    }
    let identity = require_verified_identity(&state.api, &headers).await?;
    let request: BrainAccountObservationRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::bad_request("invalid observation request body"))?;
    request
        .validate(&state.config.brain_server)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let outcome = state
        .api
        .store
        .record_brain_account_observation(
            &identity.workos_user_id,
            OBSERVATION_ISSUER,
            &request,
            time::OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|error| match error {
            BrainObservationError::Stale => ApiError::bad_request("observation_expired"),
            BrainObservationError::OperationReused => ApiError::conflict("operation_id_reused"),
            BrainObservationError::KeyAssociatedElsewhere => {
                ApiError::conflict("key_associated_elsewhere")
            }
            BrainObservationError::KeyIsAgentRecord => {
                ApiError::conflict("key_conflicts_with_agent_record")
            }
            BrainObservationError::AgentNotOwned => ApiError::forbidden("agent_not_owned"),
            BrainObservationError::AccountNotLinked => ApiError::forbidden("account_not_linked"),
            BrainObservationError::Store(error) => error.into(),
        })?;
    tracing::info!(
        brain_id = %request.brain_id,
        action = ?request.action_kind,
        outcome = outcome.as_str(),
        "brain account observation"
    );
    Ok(Json(BrainAccountObservationResponse {
        version: OBSERVATION_VERSION.to_string(),
        operation_id: request.operation_id,
        outcome,
    }))
}

async fn describe_identities(
    State(state): State<BrainIdentityState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<BrainIdentityDescriptionsResponse>, ApiError> {
    if !state.config.description_credential_matches(
        header_value(&headers, DESCRIPTION_CREDENTIAL_HEADER).as_deref(),
    ) {
        return Err(ApiError::unauthorized("invalid description credential"));
    }
    let request: BrainIdentityDescriptionsRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::bad_request("invalid descriptions request body"))?;
    request
        .validate(&state.config.brain_server)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let checked_at = crate::current_time_iso()?;
    let results = tokio::time::timeout(
        DESCRIPTION_BUDGET,
        state.api.store.describe_brain_identities(&request),
    )
    .await
    .map_err(|_| ApiError::service_unavailable("identity description budget exceeded"))??;
    let response = BrainIdentityDescriptionsResponse {
        version: DESCRIPTIONS_VERSION.to_string(),
        brain_id: request.brain_id.clone(),
        checked_at,
        results,
    };
    if serde_json::to_vec(&response)
        .map_err(|error| CoreError::Store(error.to_string()))?
        .len()
        > MAX_DESCRIPTION_RESPONSE_BYTES
    {
        return Err(ApiError::service_unavailable(
            "identity description response exceeded its bound",
        ));
    }
    // Audit: the calling admin's public key, the Brain and counts. Routine
    // logs never carry contact details.
    tracing::info!(
        brain_id = %request.brain_id,
        requested_by = %request.requested_by_public_key_hex,
        keys = request.public_keys_hex.len(),
        "brain identity descriptions"
    );
    Ok(Json(response))
}
