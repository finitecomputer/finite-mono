//! `PUT /api/core/v1/me/front-door-preference`: the signed-in user's "use the
//! new Finite by default" setting. Authorized like every other `/me` route by
//! the caller's fresh WorkOS session, and it changes only the caller's own
//! `users` row. It is a routing preference for the legacy dashboard's front
//! door, not a security boundary; `/me` exposes it read-only as `migrated_at`.

use super::*;
use axum::body::Bytes;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrontDoorPreferenceRequest {
    pub use_new_finite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrontDoorPreferenceResponse {
    pub use_new_finite: bool,
    /// When the preference was turned on (`null` while off), as in `/me`.
    pub migrated_at: Option<String>,
}

pub(super) async fn set_front_door_preference(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<FrontDoorPreferenceResponse>, ApiError> {
    // Authenticate before parsing so an anonymous caller learns nothing about
    // the request shape.
    let identity = require_verified_identity(&state, &headers).await?;
    let request: FrontDoorPreferenceRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::bad_request("invalid front door preference body"))?;
    link_verified_identity(&state, &identity).await?;
    let preference = state
        .store
        .set_front_door_preference(&identity.workos_user_id, request.use_new_finite, None)
        .await?
        .ok_or_else(|| ApiError::not_found("unknown account"))?;
    tracing::info!(
        use_new_finite = preference.use_new_finite,
        "front door preference set"
    );
    Ok(Json(FrontDoorPreferenceResponse {
        use_new_finite: preference.use_new_finite,
        migrated_at: preference.migrated_at,
    }))
}
