use super::*;

pub(super) fn workos_api_error_at(stage: &'static str, error: WorkosAuthError) -> ApiError {
    tracing::warn!(stage, error = %error, "WorkOS authentication rejected");
    workos_api_error(error)
}

pub(super) fn workos_api_error(error: WorkosAuthError) -> ApiError {
    match error {
        WorkosAuthError::UnverifiedUser => ApiError::forbidden("a verified email is required"),
        WorkosAuthError::Unavailable => {
            ApiError::service_unavailable("account authentication is temporarily unavailable")
        }
        WorkosAuthError::InvalidToken
        | WorkosAuthError::UnknownUser
        | WorkosAuthError::InvalidUser => ApiError::unauthorized("invalid account session"),
    }
}

pub(super) fn require_service_auth(
    state: &CoreApiState,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    require_route_credential(
        headers,
        state.auth.service_api_token(),
        "invalid service credential",
    )
}

pub(super) fn require_runner_auth(
    state: &CoreApiState,
    headers: &HeaderMap,
) -> Result<VerifiedRunnerCredential, ApiError> {
    let presented =
        bearer_token(headers).ok_or_else(|| ApiError::unauthorized("invalid Runner credential"))?;
    state
        .auth
        .verify_runner_credential(&presented)
        .ok_or_else(|| ApiError::unauthorized("invalid Runner credential"))
}

pub(super) fn authorize_runner_id(
    credential: &VerifiedRunnerCredential,
    runner_id: &str,
) -> Result<(), ApiError> {
    if constant_time_token_eq(runner_id.trim(), &credential.runner_id) {
        Ok(())
    } else {
        Err(runner_binding_error())
    }
}

pub(super) fn authorize_runner_capacity(
    credential: &VerifiedRunnerCredential,
    capacity: Option<RunnerLeaseCapacity>,
) -> Result<RunnerLeaseCapacity, ApiError> {
    let Some(mut capacity) = capacity else {
        if credential.legacy_kata_compatibility {
            return Ok(RunnerLeaseCapacity {
                runner_classes: credential.runner_classes.clone(),
                runtime_capabilities: Some(legacy_kata_runtime_capabilities()),
                ..RunnerLeaseCapacity::default()
            });
        }
        return Err(runner_binding_error());
    };
    if capacity.runner_classes.is_empty()
        || capacity.runner_classes.len() != credential.runner_classes.len()
        || !capacity
            .runner_classes
            .iter()
            .all(|class| credential.runner_classes.contains(class))
    {
        return Err(runner_binding_error());
    }
    if capacity.runtime_capabilities.is_none() {
        if credential.legacy_kata_compatibility {
            capacity.runtime_capabilities = Some(legacy_kata_runtime_capabilities());
        } else {
            return Err(runner_binding_error());
        }
    }
    capacity
        .validate_runtime_capability_policy()
        .map_err(|_| runner_binding_error())?;
    Ok(capacity)
}

pub(super) fn legacy_kata_runtime_capabilities() -> RuntimeCapabilitiesEnvelope {
    RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
        restart: true,
        recover_known_good_chat: false,
        runtime_upgrade: true,
        stop: true,
        runtime_retirement: false,
    })
}

pub(super) fn authorize_runner_runtime_capabilities(
    credential: &VerifiedRunnerCredential,
    capabilities: Option<RuntimeCapabilitiesEnvelope>,
) -> Result<RuntimeCapabilitiesEnvelope, ApiError> {
    let capabilities = match capabilities {
        Some(capabilities) => capabilities,
        None if credential.legacy_kata_compatibility => legacy_kata_runtime_capabilities(),
        None => return Err(runner_binding_error()),
    };
    RunnerLeaseCapacity {
        runner_classes: credential.runner_classes.clone(),
        runtime_capabilities: Some(capabilities.clone()),
        ..RunnerLeaseCapacity::default()
    }
    .validate_runtime_capability_policy()
    .map_err(|_| runner_binding_error())?;
    Ok(capabilities)
}

pub(super) fn authorize_runner_source_host(
    credential: &VerifiedRunnerCredential,
    source_host_id: Option<&str>,
) -> Result<String, ApiError> {
    let source_host_id = match source_host_id {
        Some(source_host_id) => {
            normalize_source_host_id(source_host_id).map_err(|_| runner_binding_error())?
        }
        None if credential.legacy_kata_compatibility => credential.source_host_id.clone(),
        None => return Err(runner_binding_error()),
    };
    if constant_time_token_eq(&source_host_id, &credential.source_host_id) {
        Ok(source_host_id)
    } else {
        Err(runner_binding_error())
    }
}

pub(super) fn authorize_provider_runtime_handle(
    credential: &VerifiedRunnerCredential,
    handle: Option<&ProviderRuntimeHandleEnvelope>,
) -> Result<(), ApiError> {
    if handle.is_none_or(|handle| credential.runner_classes.contains(&handle.runner_class())) {
        Ok(())
    } else {
        Err(runner_binding_error())
    }
}

pub(super) fn runner_binding_error() -> ApiError {
    ApiError::forbidden("Runner credential is not authorized for this worker request")
}

pub(super) fn require_route_credential(
    headers: &HeaderMap,
    expected_token: &str,
    error_message: &'static str,
) -> Result<(), ApiError> {
    let expected = format!("Bearer {expected_token}");
    if header_value(headers, SERVICE_AUTH_HEADER)
        .as_deref()
        .is_some_and(|presented| constant_time_token_eq(presented, &expected))
    {
        return Ok(());
    }

    Err(ApiError::unauthorized(error_message))
}

pub(super) fn require_finite_private_usage_auth(
    state: &CoreApiState,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    require_route_credential(
        headers,
        state.auth.finite_private_usage_api_token(),
        "invalid finite private usage service token",
    )
}

pub(super) fn constant_time_token_eq(presented: &str, expected: &str) -> bool {
    let presented_digest = Sha256::digest(presented.as_bytes());
    let expected_digest = Sha256::digest(expected.as_bytes());
    bool::from(presented_digest.ct_eq(&expected_digest))
}

pub(super) fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = header_value(headers, SERVICE_AUTH_HEADER)?;
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

pub(super) fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
