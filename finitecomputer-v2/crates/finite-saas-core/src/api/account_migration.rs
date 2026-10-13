//! Service route for the new Finite product's import flow to record that an
//! account moved. It is authorized only by the dedicated
//! `FC_CORE_ACCOUNT_MIGRATION_API_TOKEN` bearer: never the general service
//! token, a Runner credential, or a WorkOS user session. Like every other
//! `/api/core` route it is served on Core's loopback listener; the public edge
//! does not route it.

use super::*;
use axum::body::Bytes;

/// Upper bound for a WorkOS user id on this route; real ids are ~30 bytes.
const MAX_WORKOS_USER_ID_BYTES: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarkAccountMigratedRequest {
    pub workos_user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountMigrationResponse {
    pub workos_user_id: String,
    /// The first recorded mark. Replays return it unchanged.
    pub migrated_at: String,
    /// False when an earlier call had already recorded the migration.
    pub newly_marked: bool,
}

pub(super) async fn mark_account_migrated(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<AccountMigrationResponse>, ApiError> {
    let Some(expected) = state.auth.account_migration_api_token() else {
        return Err(ApiError::service_unavailable(
            "account migration marking is not configured",
        ));
    };
    // Authorize before parsing so an unauthenticated caller learns nothing
    // about the request shape.
    require_route_credential(&headers, expected, "invalid account migration credential")?;
    let request: MarkAccountMigratedRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::bad_request("invalid account migration request body"))?;
    let workos_user_id = request.workos_user_id.trim();
    if workos_user_id.is_empty()
        || workos_user_id.len() > MAX_WORKOS_USER_ID_BYTES
        || workos_user_id.chars().any(char::is_control)
    {
        return Err(ApiError::bad_request("workosUserId is invalid"));
    }
    let mark = state
        .store
        .mark_account_migrated(workos_user_id, None)
        .await?
        .ok_or_else(|| ApiError::not_found("unknown account"))?;
    tracing::info!(
        workos_user_id = %mark.workos_user_id,
        newly_marked = mark.newly_marked,
        "account migration recorded"
    );
    Ok(Json(AccountMigrationResponse {
        workos_user_id: mark.workos_user_id,
        migrated_at: mark.migrated_at,
        newly_marked: mark.newly_marked,
    }))
}
