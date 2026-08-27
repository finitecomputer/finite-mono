use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use crate::{
    AgentSyncStatus, CliEnvironment, CliError, HealthCheck, HttpResponse, SyncOnceReport,
    find_agent_state, load_signer, mutate_agent_state, option_value,
    pending_working_tree_change_paths, read_agent_state, reconcile_local_search_paths,
    reconcile_search_changes, run_working_tree_sync, signed_http_auth_header,
};

pub(crate) const FINITE_BRAIN_DEVELOPMENT_HTTP_HOST_ENV: &str =
    "FINITE_BRAIN_DEVELOPMENT_HTTP_HOST";
pub(crate) const FINITE_BRAIN_EXPORT_RESPONSE_LIMIT_BYTES_ENV: &str =
    "FINITE_BRAIN_EXPORT_RESPONSE_LIMIT_BYTES";
const DEFAULT_JSON_RESPONSE_LIMIT_BYTES: usize = 10 * 1024 * 1024;
pub(crate) const SYNC_BOOTSTRAP_RESPONSE_LIMIT_BYTES: usize = 128 * 1024 * 1024;
const DEFAULT_EXPORT_RESPONSE_LIMIT_BYTES: usize = SYNC_BOOTSTRAP_RESPONSE_LIMIT_BYTES;

/// Response cap for full encrypted Brain exports. Routine sync never fetches
/// the export, so this bound only guards first-open/repair/re-import fetches;
/// it tracks the bootstrap cap and can be raised via environment for
/// exceptionally large Brains.
pub(crate) fn encrypted_export_response_limit_bytes() -> usize {
    parse_response_limit_bytes_env(std::env::var(FINITE_BRAIN_EXPORT_RESPONSE_LIMIT_BYTES_ENV).ok())
}

fn parse_response_limit_bytes_env(value: Option<String>) -> usize {
    value
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|limit| *limit > 0)
        .unwrap_or(DEFAULT_EXPORT_RESPONSE_LIMIT_BYTES)
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrainUpdateNotification {
    pub brain_id: String,
    pub latest_sequence: u64,
    pub reason: String,
    #[serde(skip)]
    pub transport_epoch: u64,
}

pub(crate) fn read_brain_update_stream(
    env: &CliEnvironment,
    sender: &std::sync::mpsc::Sender<Result<BrainUpdateNotification, String>>,
    connected: &mut bool,
) -> Result<(), CliError> {
    let server_url = server_url_for_command(env, &[])?;
    let path = "/v1/brain-updates";
    let transport_url = absolute_server_url(&server_url, path);
    let authorization_url = authorization_url_for_request(env, &server_url, path);
    let signer = load_signer(env)?;
    let authorization = signed_http_auth_header(&signer.keys, "GET", &authorization_url, None)?;
    let response = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .build()
        .get(&transport_url)
        .set("Accept", "text/event-stream")
        .set("Authorization", &authorization)
        .call()
        .map_err(|error| match error {
            ureq::Error::Status(404 | 405, _) => CliError::Unsupported(
                "Brain Update Notifications are not supported by this server".to_owned(),
            ),
            error => CliError::Http(error.to_string()),
        })?;
    *connected = true;
    let _ = sender.send(Ok(BrainUpdateNotification {
        brain_id: String::new(),
        latest_sequence: 0,
        reason: "stream_catch_up".to_owned(),
        transport_epoch: 0,
    }));
    let mut event = String::new();
    let mut data = String::new();
    for line in BufReader::new(response.into_reader()).lines() {
        let line = line.map_err(|error| CliError::Http(error.to_string()))?;
        if line.is_empty() {
            if event == "brain_update" && !data.is_empty() {
                match serde_json::from_str(&data) {
                    Ok(notification) => {
                        if sender.send(Ok(notification)).is_err() {
                            return Ok(());
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error.to_string()));
                    }
                }
            }
            event.clear();
            data.clear();
        } else if let Some(value) = line.strip_prefix("event:") {
            event = value.trim().to_owned();
        } else if let Some(value) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.trim_start());
        }
    }
    Ok(())
}

pub(crate) fn check_signed_brain_access(env: &CliEnvironment, server_url: &str) -> HealthCheck {
    match signed_json_request_to_server(env, server_url, "GET", "/v1/brains", None) {
        Ok(_) => HealthCheck::ok(format!(
            "signed /v1/brains request succeeded at {server_url}"
        )),
        Err(error) => HealthCheck::warn(format!("signed /v1/brains request failed: {error}")),
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
    // The notification supervisor and an explicit `fbrain sync now` are
    // separate processes. Serialize every writer for a Brain through the
    // stable Runtime config directory so replacing a Working Tree root cannot
    // create a second lock domain around the same authoritative Brain.
    let _sync_lock = root
        .as_deref()
        .map(|root| {
            let brain_id = read_agent_state(root)?.brain_id;
            acquire_brain_sync_lock(env, &brain_id)
        })
        .transpose()?;
    sync_once_with_local_paths_holding_lock(env, args, activity_kind, discovered_local_paths, root)
}

pub(crate) fn sync_once_holding_brain_lock(
    env: &CliEnvironment,
    args: &[String],
    activity_kind: &str,
) -> Result<SyncOnceReport, CliError> {
    let root = find_agent_state(&env.cwd).ok().flatten();
    sync_once_with_local_paths_holding_lock(env, args, activity_kind, None, root)
}

fn sync_once_with_local_paths_holding_lock(
    env: &CliEnvironment,
    args: &[String],
    activity_kind: &str,
    discovered_local_paths: Option<Vec<String>>,
    root: Option<PathBuf>,
) -> Result<SyncOnceReport, CliError> {
    let local_paths = discovered_local_paths.map_or_else(
        || {
            root.as_deref()
                .map(pending_working_tree_change_paths)
                .transpose()
        },
        |paths| Ok(Some(paths)),
    );
    let report = run_working_tree_sync(env, args, activity_kind);
    if report.as_ref().is_err_and(is_brain_access_loss) {
        let _ = mutate_agent_state(env, |state, now| {
            state.sync.status = AgentSyncStatus::PausedAccessRevoked;
            state.daemon.last_error =
                Some("authoritative Brain access is no longer available".to_owned());
            state.add_activity(
                now,
                "daemon.access_paused",
                "Brain sync paused after authoritative access loss; local files and unsynced edits were preserved",
            );
            Ok(())
        });
    }
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

pub(crate) fn acquire_brain_sync_lock(
    env: &CliEnvironment,
    brain_id: &str,
) -> Result<std::fs::File, CliError> {
    let lock_directory = env.config_dir.join("sync-locks");
    fs::create_dir_all(&lock_directory)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_directory.join(format!("{brain_id}.lock")))?;
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive)
        .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))?;
    Ok(lock)
}

pub(crate) fn is_brain_access_loss(error: &CliError) -> bool {
    if let CliError::SyncStage { source, .. } = error {
        return is_brain_access_loss(source);
    }
    matches!(
        error,
        CliError::HttpStatus { status: 403, body }
            if {
                let canonical = body.to_ascii_lowercase();
                canonical.contains("brain access required")
                    || canonical.contains("brain_access_required")
            }
    )
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

pub(crate) fn signed_json_request_with_response_limit(
    env: &CliEnvironment,
    args: &[String],
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
    response_limit_bytes: usize,
) -> Result<serde_json::Value, CliError> {
    let server_url = server_url_for_command(env, args)?;
    signed_json_request_to_server_with_response_limit(
        env,
        &server_url,
        method,
        path,
        body,
        response_limit_bytes,
    )
}

pub(crate) fn signed_json_request_to_server(
    env: &CliEnvironment,
    server_url: &str,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, CliError> {
    signed_json_request_to_server_with_response_limit(
        env,
        server_url,
        method,
        path,
        body,
        DEFAULT_JSON_RESPONSE_LIMIT_BYTES,
    )
}

pub(crate) fn signed_json_request_to_server_with_response_limit(
    env: &CliEnvironment,
    server_url: &str,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
    response_limit_bytes: usize,
) -> Result<serde_json::Value, CliError> {
    let body = body.map(|body| serde_json::to_vec(&body)).transpose()?;
    let transport_url = absolute_server_url(server_url, path);
    let authorization_url = authorization_url_for_request(env, server_url, path);
    validate_http_url(&authorization_url)?;
    let signer = load_signer(env)?;
    let authorization =
        signed_http_auth_header(&signer.keys, method, &authorization_url, body.as_deref())?;
    let response = http_request_with_response_limit(
        method,
        &transport_url,
        Some(&authorization),
        body.as_deref(),
        response_limit_bytes,
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
    parse_success_json(&response.body)
}

fn parse_success_json(body: &str) -> Result<serde_json::Value, CliError> {
    serde_json::from_str(body).map_err(|error| CliError::HttpResponseDecode(error.to_string()))
}

fn http_request_with_response_limit(
    method: &str,
    url: &str,
    authorization: Option<&str>,
    body: Option<&[u8]>,
    response_limit_bytes: usize,
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
    if response
        .header("Content-Length")
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > response_limit_bytes)
    {
        return Err(body_read_error(
            status,
            format!("response body exceeds the configured {response_limit_bytes}-byte limit"),
        ));
    }
    let body = read_response_body(response.into_reader(), status, response_limit_bytes)?;
    Ok(HttpResponse { status, body })
}

fn read_response_body(
    reader: impl Read,
    status: u16,
    response_limit_bytes: usize,
) -> Result<String, CliError> {
    let mut bytes = Vec::new();
    reader
        .take(response_limit_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| body_read_error(status, error.to_string()))?;
    if bytes.len() > response_limit_bytes {
        return Err(body_read_error(
            status,
            format!("response body exceeds the configured {response_limit_bytes}-byte limit"),
        ));
    }
    String::from_utf8(bytes).map_err(|error| body_read_error(status, error.to_string()))
}

fn body_read_error(status: u16, error: String) -> CliError {
    // Headers are authoritative even when the body stream is broken.
    // Preserve the status so callers cannot misclassify a server rejection as
    // transport uncertainty.
    if !(200..300).contains(&status) {
        CliError::HttpStatus {
            status,
            body: format!("response body could not be read: {error}"),
        }
    } else {
        CliError::Http(error)
    }
}

pub(crate) fn server_url_for_command(
    env: &CliEnvironment,
    args: &[String],
) -> Result<String, CliError> {
    server_url_for_optional_command(env, args)?.ok_or(CliError::MissingServer)
}

pub(crate) fn server_url_for_optional_command(
    env: &CliEnvironment,
    args: &[String],
) -> Result<Option<String>, CliError> {
    let explicit = option_value(args, "--server");
    if explicit
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty())
    {
        return Ok(select_server_url(explicit, None, None, None));
    }
    Ok(select_server_url(
        None,
        saved_server_url(env)?,
        env.server_url.clone(),
        env.public_base_url.clone(),
    ))
}

pub(crate) fn configured_server_url_for_open(
    env: &CliEnvironment,
    args: &[String],
) -> Option<String> {
    select_server_url(
        option_value(args, "--server"),
        None,
        env.server_url.clone(),
        env.public_base_url.clone(),
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

fn saved_server_url(env: &CliEnvironment) -> Result<Option<String>, CliError> {
    let Some(root) = find_agent_state(&env.cwd)? else {
        return Ok(None);
    };
    Ok(read_agent_state(&root)?.server_url)
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

fn authorization_url_for_request(env: &CliEnvironment, server_url: &str, path: &str) -> String {
    let uses_configured_transport = env
        .server_url
        .as_deref()
        .is_some_and(|configured| same_origin_text(configured, server_url));
    let base_url = if uses_configured_transport {
        env.public_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(server_url)
    } else {
        server_url
    };
    absolute_server_url(base_url, path)
}

fn same_origin_text(left: &str, right: &str) -> bool {
    left.trim().trim_end_matches('/') == right.trim().trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_response_limit_env_override_parses_or_falls_back() {
        assert_eq!(
            parse_response_limit_bytes_env(Some(" 2048 ".to_owned())),
            2048
        );
        assert_eq!(
            parse_response_limit_bytes_env(Some("not-a-number".to_owned())),
            DEFAULT_EXPORT_RESPONSE_LIMIT_BYTES
        );
        assert_eq!(
            parse_response_limit_bytes_env(Some("0".to_owned())),
            DEFAULT_EXPORT_RESPONSE_LIMIT_BYTES
        );
        assert_eq!(
            parse_response_limit_bytes_env(None),
            DEFAULT_EXPORT_RESPONSE_LIMIT_BYTES
        );
    }

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
        let env = CliEnvironment {
            server_url: Some("http://192.168.67.1:18790".to_owned()),
            public_base_url: Some("http://127.0.0.1:13002".to_owned()),
            ..test_environment()
        };
        assert_eq!(
            authorization_url_for_request(&env, "http://192.168.67.1:18790", "/v1/brains",),
            "http://127.0.0.1:13002/v1/brains"
        );
    }

    #[test]
    fn explicit_or_saved_transport_override_is_signed_for_its_exact_origin() {
        let env = CliEnvironment {
            server_url: Some("https://brain.finite.computer".to_owned()),
            public_base_url: Some("https://brain.finite.computer".to_owned()),
            ..test_environment()
        };
        assert_eq!(
            authorization_url_for_request(&env, "http://127.0.0.1:18790", "/v1/brains"),
            "http://127.0.0.1:18790/v1/brains"
        );
        assert_eq!(
            authorization_url_for_request(
                &env,
                "https://brain.smoke.finite.computer",
                "/v1/brains"
            ),
            "https://brain.smoke.finite.computer/v1/brains"
        );
    }

    fn test_environment() -> CliEnvironment {
        CliEnvironment {
            cwd: std::path::PathBuf::from("."),
            config_dir: std::path::PathBuf::from(".fbrain"),
            server_url: None,
            public_base_url: None,
            working_tree_root: None,
            now: None,
            identity_authority_url: None,
            finite_home: None,
            embedding_provider: None,
        }
    }

    #[test]
    fn transport_without_public_origin_signs_itself() {
        assert_eq!(
            authorization_url_for_request(
                &test_environment(),
                "http://192.168.67.1:18790",
                "/v1/brains",
            ),
            "http://192.168.67.1:18790/v1/brains"
        );
    }

    #[test]
    fn malformed_success_body_is_typed_as_response_decode_uncertainty() {
        let error = parse_success_json("{not-json").unwrap_err();
        assert!(matches!(error, CliError::HttpResponseDecode(_)));
    }

    #[test]
    fn body_read_failure_preserves_authoritative_non_success_status() {
        let error = body_read_error(409, "connection reset".to_owned());
        assert!(matches!(error, CliError::HttpStatus { status: 409, .. }));
    }

    #[test]
    fn bounded_response_reader_accepts_exact_limit() {
        let body = read_response_body(std::io::Cursor::new(b"1234"), 200, 4).unwrap();
        assert_eq!(body, "1234");
    }

    #[test]
    fn bounded_response_reader_rejects_one_byte_over_limit_before_json_decode() {
        let error = read_response_body(std::io::Cursor::new(b"12345"), 200, 4).unwrap_err();
        assert!(matches!(error, CliError::Http(_)));
        assert!(error.to_string().contains("configured 4-byte limit"));
        assert!(!matches!(error, CliError::HttpResponseDecode(_)));
    }

    #[test]
    fn only_authoritative_brain_access_rejection_pauses_sync() {
        assert!(is_brain_access_loss(&CliError::HttpStatus {
            status: 403,
            body: r#"{"error":"brain access required"}"#.to_owned(),
        }));
        assert!(!is_brain_access_loss(&CliError::HttpStatus {
            status: 403,
            body: "folder access required".to_owned(),
        }));
        assert!(!is_brain_access_loss(&CliError::Http("offline".to_owned())));
        assert!(is_brain_access_loss(&CliError::SyncStage {
            stage: "fetch incremental remote sync".to_owned(),
            root: std::path::PathBuf::from("/tmp/brain"),
            source: Box::new(CliError::HttpStatus {
                status: 403,
                body: "brain_access_required".to_owned(),
            }),
        }));
    }
}
