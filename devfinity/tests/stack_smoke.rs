use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

#[test]
#[ignore = "requires a running devfinity stack; run `just dev rust-smoke`"]
fn dashboard_create_agent_flow_persists_request_in_core() -> Result<(), Box<dyn std::error::Error>>
{
    let env = DevfinityEnv::from_env()?;
    assert_eq!(env.profile, "services-only");
    assert_http_contains("core", &format!("{}/healthz", env.core_url), "\"ok\":true")?;
    assert_http_contains(
        "finitechat",
        &format!("{}/health", env.finitechat_url),
        "\"status\":\"ok\"",
    )?;
    assert_http_contains(
        "hosted-web-device",
        &format!("{}/healthz", env.hosted_web_device_url),
        "\"status\":\"ok\"",
    )?;
    assert_http_contains(
        "finitesites",
        &format!("{}/api/v1/healthz", env.finitesites_api_url),
        "\"ok\":true",
    )?;
    assert_http_contains("dashboard", &env.dashboard_url, "<html")?;

    let run_id = smoke_run_id();
    let display_name = format!("Devfinity Rust Smoke Agent {run_id}");
    let idempotency_key = format!("devfinity-rust-smoke-{run_id}");
    let launch_code = env::var("DEVFINITY_SMOKE_LAUNCH_CODE").unwrap_or_else(|_| "off2026".into());

    let response = ureq::post(&format!("{}/agent-creation-requests", env.dashboard_url))
        .send_form(&[
            ("displayName", display_name.as_str()),
            ("launchCode", launch_code.as_str()),
            ("idempotencyKey", idempotency_key.as_str()),
        ])?;
    assert!(
        (200..400).contains(&response.status()),
        "dashboard create-agent returned HTTP {}",
        response.status()
    );

    let me: Value = ureq::get(&format!("{}/api/core/v1/me", env.core_url))
        .set("authorization", &format!("Bearer {}", env.core_api_token))
        .set("content-type", "application/json")
        .set("x-finite-workos-user-id", &env.dashboard_dev_workos_user_id)
        .set("x-finite-workos-email", &env.dashboard_dev_email)
        .set("x-finite-workos-email-verified", "true")
        .call()?
        .into_json()?;

    let requests = me
        .get("agent_creation_requests")
        .and_then(Value::as_array)
        .ok_or("Core /me response did not include agent_creation_requests")?;
    let request = requests
        .iter()
        .find(|candidate| {
            candidate.get("display_name").and_then(Value::as_str) == Some(display_name.as_str())
                && matches!(
                    candidate.get("status").and_then(Value::as_str),
                    Some("requested" | "launching")
                )
        })
        .ok_or_else(|| {
            format!(
                "Core did not report a pending dashboard-created agent request for {display_name}: {me}"
            )
        })?;
    let project_id = request
        .get("project_id")
        .and_then(Value::as_str)
        .ok_or("agent creation request did not include project_id")?;

    let projects = me
        .get("projects")
        .and_then(Value::as_array)
        .ok_or("Core /me response did not include projects")?;
    let project_exists = projects.iter().any(|candidate| {
        candidate
            .get("project")
            .and_then(|project| project.get("id"))
            .and_then(Value::as_str)
            == Some(project_id)
    });
    assert!(
        project_exists,
        "Core reported request {} without project {project_id}",
        request
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>")
    );

    println!(
        "dashboard->core rust smoke ok: request {} created project {project_id}",
        request
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>")
    );
    Ok(())
}

struct DevfinityEnv {
    core_url: String,
    dashboard_url: String,
    finitechat_url: String,
    hosted_web_device_url: String,
    finitesites_api_url: String,
    core_api_token: String,
    dashboard_dev_email: String,
    dashboard_dev_workos_user_id: String,
    profile: String,
}

impl DevfinityEnv {
    fn from_env() -> Result<Self, env::VarError> {
        Ok(Self {
            core_url: trim_trailing_slash(env::var("FC_CORE_URL")?),
            dashboard_url: trim_trailing_slash(env::var("FC_DASHBOARD_URL")?),
            finitechat_url: trim_trailing_slash(env::var("FINITECHAT_SERVER_URL")?),
            hosted_web_device_url: trim_trailing_slash(env::var("FC_HOSTED_WEB_DEVICE_URL")?),
            finitesites_api_url: trim_trailing_slash(env::var("FINITE_SITES_API")?),
            core_api_token: env::var("FC_CORE_API_TOKEN")?,
            dashboard_dev_email: env::var("FC_DASHBOARD_DEV_EMAIL")?,
            dashboard_dev_workos_user_id: env::var("FC_DASHBOARD_DEV_WORKOS_USER_ID")?,
            profile: env::var("DEVFINITY_PROFILE")?,
        })
    }
}

fn assert_http_contains(
    name: &str,
    url: &str,
    expected: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let body = ureq::get(url).call()?.into_string()?;
    assert!(
        body.contains(expected),
        "{name} response from {url} did not contain {expected:?}: {body}"
    );
    Ok(())
}

fn trim_trailing_slash(value: String) -> String {
    value.trim_end_matches('/').to_string()
}

fn smoke_run_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("{millis}-{}", std::process::id())
}
