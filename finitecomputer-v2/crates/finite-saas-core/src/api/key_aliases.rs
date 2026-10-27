//! Private key aliases for the signed-in account. The viewer is always the
//! verified WorkOS account; no route accepts an owner, account or email.

use super::*;
use crate::account_labels::{
    KeyAlias, KeyAliasLookupRequest, KeyAliasesResponse, MAX_ALIAS_LOOKUP_KEYS, SetKeyAliasRequest,
    canonical_public_key, normalize_alias,
};
use crate::store::KeyAliasError;
use axum::body::Bytes;

pub(super) const LOOKUP_PATH: &str = "/api/core/v1/me/key-aliases/lookup";
pub(super) const ALIAS_PATH: &str = "/api/core/v1/me/key-aliases/{public_key}";

fn alias_error(error: KeyAliasError) -> ApiError {
    match error {
        KeyAliasError::AccountNotLinked => ApiError::forbidden("account_not_linked"),
        KeyAliasError::LimitReached => ApiError::conflict("alias_limit_reached"),
        KeyAliasError::Store(error) => error.into(),
    }
}

fn key_from_path(value: &str) -> Result<String, ApiError> {
    canonical_public_key(value).ok_or_else(|| ApiError::bad_request("invalid public key"))
}

pub(super) async fn lookup(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<KeyAliasesResponse>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    let request: KeyAliasLookupRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::bad_request("invalid alias lookup body"))?;
    if request.public_keys.is_empty() || request.public_keys.len() > MAX_ALIAS_LOOKUP_KEYS {
        return Err(ApiError::bad_request("publicKeys must contain 1-100 keys"));
    }
    let mut keys = request
        .public_keys
        .iter()
        .map(|key| canonical_public_key(key))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| ApiError::bad_request("invalid public key"))?;
    keys.sort();
    keys.dedup();
    let aliases = state
        .store
        .key_aliases_for_viewer(&identity.workos_user_id, &keys)
        .await?;
    Ok(Json(KeyAliasesResponse { aliases }))
}

pub(super) async fn set(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(public_key): Path<String>,
    body: Bytes,
) -> Result<Json<KeyAlias>, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    let key = key_from_path(&public_key)?;
    let request: SetKeyAliasRequest =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("invalid alias body"))?;
    let alias = normalize_alias(&request.alias).ok_or_else(|| {
        ApiError::bad_request("alias must be 1-64 characters without control characters")
    })?;
    let now = crate::current_time_iso()?;
    let saved = state
        .store
        .set_key_alias(&identity.workos_user_id, &key, &alias, &now)
        .await
        .map_err(alias_error)?;
    Ok(Json(saved))
}

pub(super) async fn clear(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(public_key): Path<String>,
) -> Result<StatusCode, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    let key = key_from_path(&public_key)?;
    state
        .store
        .clear_key_alias(&identity.workos_user_id, &key)
        .await
        .map_err(alias_error)?;
    Ok(StatusCode::NO_CONTENT)
}
