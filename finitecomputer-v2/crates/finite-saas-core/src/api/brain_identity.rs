//! Private Brain identity listener (FIN-122). These routes are served only by
//! `brain_identity_router` on its own bind address; neither the account
//! router nor the runtime router mounts them.

use super::*;
mod hosted;
use crate::brain_identity::hosted::HOSTED_OBSERVATION_PATH;
use crate::brain_identity::scoped::{
    DESCRIPTIONS_VERSION_V3, ScopedDescriptionsRequest, ScopedDescriptionsResponse,
};
use crate::brain_identity::{
    BrainAccountObservationRequest, BrainAccountObservationResponse, BrainIdentityConfig,
    BrainIdentityDescriptionsRequest, BrainIdentityDescriptionsResponse, DESCRIPTION_BUDGET,
    DESCRIPTION_CREDENTIAL_HEADER, DESCRIPTIONS_PATH, MAX_DESCRIPTION_BODY_BYTES,
    MAX_DESCRIPTION_RESPONSE_BYTES, MAX_OBSERVATION_BODY_BYTES, OBSERVATION_CREDENTIAL_HEADER,
    OBSERVATION_ISSUER, OBSERVATION_PATH, OBSERVATION_VERSION,
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
            HOSTED_OBSERVATION_PATH,
            post(hosted::record_hosted_observation)
                .layer(DefaultBodyLimit::max(MAX_OBSERVATION_BODY_BYTES)),
        )
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
        .map_err(observation_error)?;
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
) -> Result<Json<Value>, ApiError> {
    if !state.config.description_credential_matches(
        header_value(&headers, DESCRIPTION_CREDENTIAL_HEADER).as_deref(),
    ) {
        return Err(ApiError::unauthorized("invalid description credential"));
    }
    // Detect version-specific semantic violations before strict DTO parsing;
    // these must never resemble an old Core's negotiation error (P2).
    let value: Value = serde_json::from_slice(&body)
        .map_err(|_| ApiError::bad_request("invalid descriptions request body"))?;
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::bad_request("invalid descriptions request body"))?;
    let version = object.get("version").and_then(Value::as_str).unwrap_or("");
    let is_v3 = version == DESCRIPTIONS_VERSION_V3;
    let allowed = [
        "version",
        "brainServer",
        "brainId",
        "requestedByPublicKeyHex",
        "publicKeysHex",
    ];
    if object.keys().any(|key| {
        !(allowed.contains(&key.as_str()) || is_v3 && key == "participatingPublicKeysHex")
    }) {
        return Err(ApiError::bad_request(
            "unexpected version-specific descriptions field",
        ));
    }
    let checked_at = crate::current_time_iso()?;
    let (response, brain_id, requested_by, count) = if is_v3 {
        if !object.contains_key("participatingPublicKeysHex") {
            return Err(ApiError::bad_request("v3 participating subset is required"));
        }
        let request: ScopedDescriptionsRequest = serde_json::from_slice(&body)
            .map_err(|_| ApiError::bad_request("invalid v3 descriptions fields"))?;
        request
            .validate(&state.config.brain_server)
            .map_err(ApiError::bad_request)?;
        let results = tokio::time::timeout(
            DESCRIPTION_BUDGET,
            state.api.store.describe_scoped_brain_identities(&request),
        )
        .await
        .map_err(|_| ApiError::service_unavailable("identity description budget exceeded"))??;
        let response = ScopedDescriptionsResponse {
            version: request.version,
            brain_id: request.brain_id.clone(),
            checked_at,
            results,
        };
        (
            serde_json::to_value(response).map_err(|e| CoreError::Store(e.to_string()))?,
            request.brain_id,
            request.requested_by_public_key_hex,
            request.public_keys_hex.len(),
        )
    } else {
        let request: BrainIdentityDescriptionsRequest = serde_json::from_slice(&body)
            .map_err(|_| ApiError::bad_request("invalid descriptions request body"))?;
        request
            .validate(&state.config.brain_server)
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
        let results = tokio::time::timeout(
            DESCRIPTION_BUDGET,
            state.api.store.describe_brain_identities(&request),
        )
        .await
        .map_err(|_| ApiError::service_unavailable("identity description budget exceeded"))??;
        let response = BrainIdentityDescriptionsResponse {
            version: request.version,
            brain_id: request.brain_id.clone(),
            checked_at,
            results,
        };
        (
            serde_json::to_value(response).map_err(|e| CoreError::Store(e.to_string()))?,
            request.brain_id,
            request.requested_by_public_key_hex,
            request.public_keys_hex.len(),
        )
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
        brain_id = %brain_id,
        requested_by = %requested_by,
        keys = count,
        "brain identity descriptions"
    );
    Ok(Json(response))
}

fn observation_error(error: BrainObservationError) -> ApiError {
    match error {
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
    }
}
