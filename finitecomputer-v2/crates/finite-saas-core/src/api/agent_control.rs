//! Narrow owner-authorized forwarding to an agent-owned control router.
//! Core does not interpret connection schemas or edit agent settings.
use super::*;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    source_host_id: String,
    source_machine_id: String,
    endpoint: String,
    token: String,
}

fn unavailable() -> ApiError {
    ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: "Independent agent control is unavailable for this runtime".into(),
        correlation_id: None,
    }
}

fn target(directory: &std::path::Path, runtime: &AgentRuntime) -> Result<Target, ApiError> {
    if !runtime
        .id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(unavailable());
    }
    let path = directory.join(format!("{}.json", runtime.id));
    let metadata = fs::symlink_metadata(&path).map_err(|_| unavailable())?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 || metadata.len() > 16384 {
        return Err(unavailable());
    }
    let target: Target = serde_json::from_slice(&fs::read(path).map_err(|_| unavailable())?)
        .map_err(|_| unavailable())?;
    let url = reqwest::Url::parse(&target.endpoint).map_err(|_| unavailable())?;
    if target.source_host_id != runtime.source_host_id
        || target.source_machine_id != runtime.source_machine_id
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || target.token.len() < 32
        || target.token.len() > 256
        || target.token.bytes().any(|b| b.is_ascii_whitespace())
    {
        return Err(unavailable());
    }
    Ok(target)
}

pub(super) async fn get(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(identifier): Path<String>,
) -> Result<Response, ApiError> {
    forward(state, headers, identifier, None).await
}

pub(super) async fn post(
    State(state): State<CoreApiState>,
    headers: HeaderMap,
    Path(identifier): Path<String>,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    forward(state, headers, identifier, Some(body)).await
}

async fn forward(
    state: CoreApiState,
    headers: HeaderMap,
    identifier: String,
    body: Option<axum::body::Bytes>,
) -> Result<Response, ApiError> {
    let identity = require_verified_identity(&state, &headers).await?;
    // Every request reads current authority. No SWR cache, caller-selected URL,
    // administrative credential in the browser, or imported legacy-row bypass.
    let runtime = state
        .store
        .owned_control_runtime(&identity.workos_user_id, &identifier)
        .await?
        .ok_or_else(|| ApiError::not_found("agent runtime was not found"))?;
    let directory = state.agent_control_directory.ok_or_else(unavailable)?;
    let target = target(&directory, &runtime)?;
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(45));
    if let Some(path) = state.agent_control_ca_file {
        let cert = reqwest::Certificate::from_pem(&fs::read(path).map_err(|_| unavailable())?)
            .map_err(|_| unavailable())?;
        builder = builder.add_root_certificate(cert);
    }
    let client = builder.build().map_err(|_| unavailable())?;
    let suffix = if body.is_some() { "/commands" } else { "" };
    let url = format!(
        "{}/v1/runtimes/{}/connections{suffix}",
        target.endpoint.trim_end_matches('/'),
        runtime.id
    );
    let mut request = if let Some(body) = body {
        client
            .post(url)
            .header("content-type", "application/json")
            .body(body)
    } else {
        client.get(url)
    };
    request = request.bearer_auth(&target.token);
    let mut upstream = request.send().await.map_err(|_| unavailable())?;
    if upstream.status().is_redirection() || upstream.status() == StatusCode::UNAUTHORIZED {
        return Err(unavailable());
    }
    let status = upstream.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = upstream.chunk().await.map_err(|_| unavailable())? {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err(unavailable());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((
        status,
        [
            ("cache-control", "no-store"),
            ("content-type", "application/json"),
        ],
        bytes,
    )
        .into_response())
}

pub(super) fn directory_from_env() -> Option<PathBuf> {
    std::env::var_os("FC_CORE_AGENT_CONTROL_DIRECTORY").map(PathBuf::from)
}
