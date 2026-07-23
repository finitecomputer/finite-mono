use std::env;
use std::net::IpAddr;
use std::time::Duration;

use crate::{
    CliEnvironment, CliError, HealthCheck, HttpResponse, SyncOnceReport, find_agent_state,
    load_signer, mutate_agent_state, option_value, pending_working_tree_change_paths,
    read_agent_state, reconcile_local_search_paths, reconcile_search_changes,
    run_working_tree_sync, signed_http_auth_header,
};

pub(crate) const FINITE_BRAIN_SERVER_URL_ENV: &str = "FINITE_BRAIN_SERVER_URL";
pub(crate) const FINITE_BRAIN_PUBLIC_BASE_URL_ENV: &str = "FINITE_BRAIN_PUBLIC_BASE_URL";
pub(crate) const FINITE_BRAIN_DEVELOPMENT_HTTP_HOST_ENV: &str =
    "FINITE_BRAIN_DEVELOPMENT_HTTP_HOST";

pub(crate) fn check_http_health(url: &str) -> HealthCheck {
    let health_url = absolute_server_url(url, "/health");
    match http_request("GET", &health_url, None, None) {
        Ok(response) if response.status == 200 => {
            HealthCheck::ok(format!("server healthy at {url}"))
        }
        Ok(response) => HealthCheck::warn(format!(
            "server health returned {} at {url}",
            response.status
        )),
        Err(error) => HealthCheck::warn(format!("server health request failed: {error}")),
    }
}

pub(crate) fn sync_once(
    env: &CliEnvironment,
    args: &[String],
    activity_kind: &str,
) -> Result<SyncOnceReport, CliError> {
    sync_once_with_local_paths(env, args, activity_kind, None)
}

pub(crate) fn sync_once_with_local_paths(
    env: &CliEnvironment,
    args: &[String],
    activity_kind: &str,
    discovered_local_paths: Option<Vec<String>>,
) -> Result<SyncOnceReport, CliError> {
    let root = find_agent_state(&env.cwd).ok().flatten();
    let local_paths = discovered_local_paths.map_or_else(
        || {
            root.as_deref()
                .map(pending_working_tree_change_paths)
                .transpose()
        },
        |paths| Ok(Some(paths)),
    );
    let report = run_working_tree_sync(env, args, activity_kind);
    let reconciliation = root.as_deref().map(|root| match &report {
        Ok(report) => reconcile_search_changes(root, report),
        Err(_) => match &local_paths {
            Ok(Some(paths)) => reconcile_local_search_paths(root, paths),
            Ok(None) => Ok(0),
            Err(error) => Err(CliError::SearchIndex(format!(
                "local change discovery failed: {error}"
            ))),
        },
    });
    match reconciliation {
        Some(Err(error)) => {
            let message = error.to_string();
            let _ = mutate_agent_state(env, |state, now| {
                state.search_lifecycle.reconciliation_pending = true;
                state.search_lifecycle.consecutive_failures = state
                    .search_lifecycle
                    .consecutive_failures
                    .saturating_add(1)
                    .min(8);
                state.add_activity(
                    now,
                    "search.index.blocked",
                    format!("Search index reconciliation failed: {message}"),
                );
                Ok(())
            });
        }
        Some(Ok(_)) => {
            let _ = mutate_agent_state(env, |state, _| {
                state.search_lifecycle.reconciliation_pending = false;
                state.search_lifecycle.consecutive_failures = 0;
                Ok(())
            });
        }
        None => {}
    }
    report
}

pub(crate) fn signed_json_request(
    env: &CliEnvironment,
    args: &[String],
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, CliError> {
    let server_url = server_url_for_command(env, args)?;
    signed_json_request_to_server(env, &server_url, method, path, body)
}

pub(crate) fn signed_json_request_to_server(
    env: &CliEnvironment,
    server_url: &str,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, CliError> {
    let body = body.map(|body| serde_json::to_vec(&body)).transpose()?;
    let transport_url = absolute_server_url(server_url, path);
    let authorization_url = authorization_url_for_request(
        server_url,
        path,
        env::var(FINITE_BRAIN_PUBLIC_BASE_URL_ENV).ok().as_deref(),
    );
    validate_http_url(&authorization_url)?;
    let signer = load_signer(env)?;
    let authorization =
        signed_http_auth_header(&signer.keys, method, &authorization_url, body.as_deref())?;
    let response = http_request(
        method,
        &transport_url,
        Some(&authorization),
        body.as_deref(),
    )?;
    if !(200..300).contains(&response.status) {
        return Err(CliError::HttpStatus {
            status: response.status,
            body: response.body,
        });
    }
    if response.body.trim().is_empty() {
        return Ok(serde_json::json!({ "status": "ok" }));
    }
    serde_json::from_str(&response.body).map_err(CliError::from)
}

pub(crate) fn http_request(
    method: &str,
    url: &str,
    authorization: Option<&str>,
    body: Option<&[u8]>,
) -> Result<HttpResponse, CliError> {
    validate_http_url(url)?;
    let body = body.unwrap_or_default();
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .redirects(0)
        .build();
    let mut request = agent
        .request(method, url)
        .set("Accept", "application/json")
        .set("Connection", "close");
    if let Some(authorization) = authorization {
        request = request.set("Authorization", authorization);
    }

    let result = if body.is_empty() {
        request.call()
    } else {
        request
            .set("Content-Type", "application/json")
            .send_bytes(body)
    };
    let (status, response) = match result {
        Ok(response) => (response.status(), response),
        Err(ureq::Error::Status(status, response)) => (status, response),
        Err(error) => return Err(CliError::Http(error.to_string())),
    };
    let body = response
        .into_string()
        .map_err(|error| CliError::Http(error.to_string()))?;
    Ok(HttpResponse { status, body })
}

pub(crate) fn server_url_for_command(
    env: &CliEnvironment,
    args: &[String],
) -> Result<String, CliError> {
    server_url_for_optional_command(env, args).ok_or(CliError::MissingServer)
}

pub(crate) fn server_url_for_optional_command(
    env: &CliEnvironment,
    args: &[String],
) -> Option<String> {
    select_server_url(
        option_value(args, "--server"),
        saved_server_url(env),
        env::var(FINITE_BRAIN_SERVER_URL_ENV).ok(),
        env::var(FINITE_BRAIN_PUBLIC_BASE_URL_ENV).ok(),
    )
}

pub(crate) fn configured_server_url_for_open(args: &[String]) -> Option<String> {
    select_server_url(
        option_value(args, "--server"),
        None,
        env::var(FINITE_BRAIN_SERVER_URL_ENV).ok(),
        env::var(FINITE_BRAIN_PUBLIC_BASE_URL_ENV).ok(),
    )
}

pub(crate) fn select_server_url(
    explicit: Option<String>,
    saved: Option<String>,
    server_env: Option<String>,
    public_env: Option<String>,
) -> Option<String> {
    [explicit, saved, server_env, public_env]
        .into_iter()
        .flatten()
        .map(|url| url.trim().to_owned())
        .find(|url| !url.is_empty())
}

fn saved_server_url(env: &CliEnvironment) -> Option<String> {
    find_agent_state(&env.cwd)
        .ok()
        .flatten()
        .and_then(|root| read_agent_state(&root).ok())
        .and_then(|state| state.server_url)
}

pub(crate) fn validate_http_url(url: &str) -> Result<(), CliError> {
    let development_host = env::var(FINITE_BRAIN_DEVELOPMENT_HTTP_HOST_ENV).ok();
    validate_http_url_with_development_host(url, development_host.as_deref())
}

pub(crate) fn validate_http_url_with_development_host(
    url: &str,
    development_host: Option<&str>,
) -> Result<(), CliError> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if let Some(rest) = url.strip_prefix("http://") {
        let host = rest
            .split('/')
            .next()
            .and_then(http_host_without_port)
            .unwrap_or_default();
        if is_loopback_host(host) || development_host_matches(host, development_host) {
            return Ok(());
        }
    }
    Err(CliError::Unsupported(
        "fbrain HTTP transport requires https:// except for localhost or loopback http:// URLs"
            .to_owned(),
    ))
}

fn development_host_matches(host: &str, configured: Option<&str>) -> bool {
    let Some(configured) = configured.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    configured.len() <= 253
        && !configured.contains(['/', ':', '@', '[', ']'])
        && host.eq_ignore_ascii_case(configured)
}

fn http_host_without_port(host_port: &str) -> Option<&str> {
    let host_port = host_port.rsplit('@').next().unwrap_or(host_port);
    if let Some(rest) = host_port.strip_prefix('[') {
        let (host, suffix) = rest.split_once(']')?;
        if suffix.is_empty() {
            return Some(host);
        }
        let port = suffix.strip_prefix(':')?;
        if port.parse::<u16>().is_ok() {
            return Some(host);
        }
        return None;
    }
    let (host, port) = host_port
        .split_once(':')
        .map_or((host_port, None), |(host, port)| (host, Some(port)));
    if let Some(port) = port
        && port.parse::<u16>().is_err()
    {
        return None;
    }
    (!host.is_empty()).then_some(host)
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .map(|address| address.is_loopback())
            .unwrap_or(false)
}

pub(crate) fn absolute_server_url(server_url: &str, path: &str) -> String {
    format!(
        "{}{}",
        server_url.trim_end_matches('/'),
        if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("/{path}")
        }
    )
}

fn authorization_url_for_request(
    server_url: &str,
    path: &str,
    public_base_url: Option<&str>,
) -> String {
    let base_url = public_base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(server_url);
    absolute_server_url(base_url, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_http_validation_rejects_malformed_bracketed_hosts() {
        assert!(validate_http_url("http://[::1]:3015/health").is_ok());
        assert!(validate_http_url("http://[::1]junk:3015/health").is_err());
    }

    #[test]
    fn loopback_http_validation_rejects_malformed_ports() {
        assert!(validate_http_url("http://127.0.0.1:3015/health").is_ok());
        assert!(validate_http_url("http://127.0.0.1:bad/health").is_err());
    }

    #[test]
    fn development_http_validation_is_exact_and_fail_closed() {
        assert!(
            validate_http_url_with_development_host(
                "http://host.container.internal:18790/health",
                Some("host.container.internal"),
            )
            .is_ok()
        );
        assert!(
            validate_http_url_with_development_host(
                "http://192.168.64.1:18790/health",
                Some("192.168.64.1"),
            )
            .is_ok()
        );
        assert!(
            validate_http_url_with_development_host(
                "http://finite.computer/health",
                Some("host.container.internal"),
            )
            .is_err()
        );
        assert!(
            validate_http_url_with_development_host(
                "http://host.container.internal.attacker.test/health",
                Some("host.container.internal"),
            )
            .is_err()
        );
        assert!(
            validate_http_url_with_development_host(
                "http://host.container.internal:18790/health",
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn server_url_selection_trims_selected_candidate() {
        assert_eq!(
            select_server_url(
                Some("  ".to_owned()),
                None,
                Some("  http://127.0.0.1:3015  ".to_owned()),
                Some("https://example.test".to_owned()),
            )
            .as_deref(),
            Some("http://127.0.0.1:3015")
        );
    }

    #[test]
    fn signed_request_uses_public_origin_without_changing_transport() {
        assert_eq!(
            authorization_url_for_request(
                "http://192.168.67.1:18790",
                "/_admin/brains",
                Some(" http://127.0.0.1:13002 "),
            ),
            "http://127.0.0.1:13002/_admin/brains"
        );
        assert_eq!(
            authorization_url_for_request("http://192.168.67.1:18790", "/_admin/brains", None,),
            "http://192.168.67.1:18790/_admin/brains"
        );
    }
}
