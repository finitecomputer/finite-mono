use super::*;
use crate::brain_identity::hosted::{HOSTED_OBSERVATION_VERSION, HostedAccountObservationRequest};

pub(super) async fn record_hosted_observation(
    State(state): State<BrainIdentityState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<BrainAccountObservationResponse>, ApiError> {
    if !state.config.observation_credential_matches(
        header_value(&headers, OBSERVATION_CREDENTIAL_HEADER).as_deref(),
    ) {
        return Err(ApiError::unauthorized("invalid observation credential"));
    }
    let identity = require_verified_identity(&state.api, &headers).await?;
    let request: HostedAccountObservationRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::bad_request("invalid hosted observation request body"))?;
    request
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let outcome = state
        .api
        .store
        .record_hosted_account_observation(
            &identity.workos_user_id,
            OBSERVATION_ISSUER,
            &request,
            time::OffsetDateTime::now_utc(),
        )
        .await
        .map_err(observation_error)?;
    tracing::info!(
        operation = "hosted_account_observation",
        outcome = outcome.as_str(),
        "hosted account observation"
    );
    Ok(Json(BrainAccountObservationResponse {
        version: HOSTED_OBSERVATION_VERSION.to_string(),
        operation_id: request.operation_id,
        outcome,
    }))
}
