use crate::*;

pub(crate) async fn resolve_identity_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Json<IdentityResponse>, ApiError> {
    validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: ResolveIdentityRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    let identity = resolve_and_record_identity(&state, &request.input)?;

    Ok(Json(identity.response))
}
