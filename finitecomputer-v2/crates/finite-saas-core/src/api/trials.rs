use super::*;
use crate::trials::*;

pub(super) async fn list(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    require_admin_identity(&state, &headers).await?;
    Ok((
        [("cache-control", "no-store, private")],
        Json(state.store.list_trial_campaigns().await?),
    ))
}
pub(super) async fn create(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<CreateTrialCampaign>,
) -> Result<impl IntoResponse, ApiError> {
    let admin = require_admin_identity(&state, &headers).await?;
    Ok((
        [("cache-control", "no-store, private")],
        Json(
            state
                .store
                .create_trial_campaign(input, &admin.workos_user_id)
                .await?,
        ),
    ))
}
pub(super) async fn increase_capacity(
    State(state): State<CoreApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<IncreaseTrialCapacity>,
) -> Result<StatusCode, ApiError> {
    let admin = require_admin_identity(&state, &headers).await?;
    state
        .store
        .increase_trial_capacity(&id, input, &admin.workos_user_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn update_code(
    State(state): State<CoreApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<UpdateTrialCode>,
) -> Result<StatusCode, ApiError> {
    let admin = require_admin_identity(&state, &headers).await?;
    state
        .store
        .update_trial_code(&id, input, &admin.workos_user_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn offer(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<TrialCodeRequest>,
) -> Result<Json<TrialOffer>, ApiError> {
    require_verified_identity(&state, &headers).await?;
    Ok(Json(state.store.trial_offer(&input.code).await?))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReserveRequest {
    workos_user_id: String,
    reservation: ReserveTrial,
}

pub(super) async fn reserve(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<ReserveRequest>,
) -> Result<Json<TrialReservation>, ApiError> {
    // Only the dashboard's Stripe adapter can attest to a real Checkout Session.
    // Account tokens alone must never create arbitrary, non-expiring seat holds.
    require_service_auth(&state, &headers)?;
    Ok(Json(
        state
            .store
            .reserve_trial(&input.workos_user_id, input.reservation)
            .await?,
    ))
}
pub(super) async fn expire(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Json(input): Json<ExpireTrial>,
) -> Result<StatusCode, ApiError> {
    require_service_auth(&state, &headers)?;
    state.store.expire_trial(input).await?;
    Ok(StatusCode::NO_CONTENT)
}
