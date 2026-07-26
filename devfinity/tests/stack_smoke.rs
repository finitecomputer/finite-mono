use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use finitechat_core::native_device_link::{NativeDeviceEnrollmentSession, NativeDeviceLinkSession};
use finitechat_core::{
    AppAction, AppProfileSummary, FiniteChatRuntime, OpenOptions, npub_from_account_id,
};
use serde_json::Value;
use tempfile::TempDir;

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
    let issued: Value = ureq::post(&format!(
        "{}/api/core/v1/admin/launch-code-batches",
        env.core_url
    ))
    .set(
        "authorization",
        &format!("Bearer {}", env.operator_access_token),
    )
    .send_json(serde_json::json!({
        "name": "Devfinity Rust smoke",
        "codeCount": 1,
        "expiresInHours": 24
    }))?
    .into_json()?;
    let launch_code = issued
        .get("codes")
        .and_then(Value::as_array)
        .and_then(|codes| codes.first())
        .and_then(|code| code.get("code"))
        .and_then(Value::as_str)
        .ok_or("Core did not return one Launch Code")?;

    let response = ureq::post(&format!("{}/agent-creation-requests", env.dashboard_url))
        .send_form(&[
            ("displayName", display_name.as_str()),
            ("access", "launch-code"),
            ("launchCode", launch_code),
            ("idempotencyKey", idempotency_key.as_str()),
        ])?;
    assert!(
        (200..400).contains(&response.status()),
        "dashboard create-agent returned HTTP {}",
        response.status()
    );

    let me: Value = ureq::get(&format!("{}/api/core/v1/me", env.core_url))
        .set(
            "authorization",
            &format!("Bearer {}", env.customer_access_token),
        )
        .set("content-type", "application/json")
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

#[test]
#[ignore = "requires a running devfinity stack; run `just dev smoke`"]
fn native_device_pairing_crosses_dashboard_hosted_and_authority()
-> Result<(), Box<dyn std::error::Error>> {
    let env = DevfinityEnv::from_env()?;
    assert_eq!(env.profile, "services-only");
    let dashboard_origin = env
        .dashboard_url
        .strip_suffix("/dashboard")
        .ok_or("FC_DASHBOARD_URL must end in /dashboard")?;

    // The shell smoke opens this same Hosted Device before invoking this test.
    // Pair twice so both an established account and the source state changed by
    // a previous Device admission must survive the complete boundary.
    let first_account = pair_native_device(&env, dashboard_origin, "first", true)?;
    let second_account = pair_native_device(&env, dashboard_origin, "second", false)?;
    assert_eq!(
        first_account, second_account,
        "successive clean Devices must join the same durable human account"
    );

    println!(
        "native Device pairing crossed dashboard, Hosted Device, Identity Authority, and chat server twice"
    );
    Ok(())
}

fn pair_native_device(
    env: &DevfinityEnv,
    dashboard_origin: &str,
    suffix: &str,
    seed_agent: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let run_id = smoke_run_id();
    // Pair the synthetic Agent on the established Hosted Device before the
    // companion begins. The enrollment snapshot must originate from the
    // source Device; creating this room on the target would bypass the exact
    // boundary this smoke is intended to protect.
    let _source_agent = seed_agent
        .then(|| seed_source_agent(env, &run_id))
        .transpose()?;
    let device_id = format!("devfinity-pair-{suffix}-{run_id}");
    let link = NativeDeviceLinkSession::create(
        env.finitechat_url.clone(),
        dashboard_origin.to_owned(),
        device_id.clone(),
    )?;

    // Fixture-mode devfinity uses its bounded server-side account context.
    // Production uses the same route after bearer verification has produced
    // that context.
    link.approve_authenticated_account(None)?;
    let account_secret = link.claim_account_secret()?;
    let enrollment_grant = link.enrollment_grant()?;
    if account_secret.len() != 64 || !account_secret.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("pairing did not return one 32-byte account secret".into());
    }

    let target_store = TempDir::new()?;
    let target = FiniteChatRuntime::open(OpenOptions {
        data_dir: target_store.path().display().to_string(),
        server_url: env.finitechat_url.clone(),
        device_id,
        account_secret_hex: Some(account_secret),
        now_unix_seconds: None,
    })?;
    let mut target_state = target.dispatch_and_wait(AppAction::StartRuntime)?;
    let account_id = target_state.identity.account_id.clone();
    if account_id.len() != 64 || !account_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("paired Device did not open the expected Native Principal".into());
    }

    link.acknowledge_stored()?;
    let enrollment =
        NativeDeviceEnrollmentSession::resume(dashboard_origin.to_owned(), enrollment_grant)?;
    let ready = enrollment.wait_until_ready()?;
    for _ in 0..100_000 {
        let exact_receipts_present = ready.manifests.iter().all(|expected| {
            target_state
                .device_link_bootstrap_receipts
                .iter()
                .any(|actual| {
                    actual.bootstrap_id == expected.bootstrap_id
                        && actual.room_id == expected.room_id
                        && actual.manifest_sha256 == expected.manifest_sha256
                })
        });
        if exact_receipts_present && target_state.paired_agent.is_some() {
            return Ok(account_id);
        }
        target_state = target.dispatch_and_wait(AppAction::StartRuntime)?;
    }
    Err("paired Device did not converge to exact complete-history receipts".into())
}

fn seed_source_agent(
    env: &DevfinityEnv,
    run_id: &str,
) -> Result<(TempDir, Arc<FiniteChatRuntime>), Box<dyn std::error::Error>> {
    let agent_store = TempDir::new()?;
    let agent = FiniteChatRuntime::open(OpenOptions {
        data_dir: agent_store.path().display().to_string(),
        server_url: env.finitechat_url.clone(),
        device_id: format!("devfinity-agent-{run_id}"),
        account_secret_hex: None,
        now_unix_seconds: None,
    })?;
    let agent_state = agent.dispatch_and_wait(AppAction::StartRuntime)?;
    let agent_account_id = agent_state.identity.account_id;
    let profile = AppProfileSummary {
        npub: npub_from_account_id(agent_account_id.clone())?,
        account_id: agent_account_id,
        display_name: "Devfinity Agent".to_owned(),
        about: None,
        picture: None,
        stale: false,
        is_agent: true,
    };

    hosted_action(env, serde_json::json!({ "StartRuntime": null }))?;
    let connected = hosted_action(
        env,
        serde_json::json!({
            "StartProfileChat": {
                "profile": profile,
                "display_name": "Devfinity Agent"
            }
        }),
    )?;
    let room_id = connected
        .get("selected_room_id")
        .and_then(Value::as_str)
        .ok_or("Hosted source did not create the synthetic direct-agent room")?
        .to_owned();
    agent.dispatch_and_wait(AppAction::StartRuntime)?;
    let paired = hosted_action(
        env,
        serde_json::json!({ "PairAgent": { "room_id": room_id } }),
    )?;
    if paired
        .get("paired_agent")
        .and_then(|value| value.get("canonical_room_id"))
        .and_then(Value::as_str)
        != Some(room_id.as_str())
    {
        return Err("Hosted source did not persist the canonical paired Agent".into());
    }

    Ok((agent_store, agent))
}

fn hosted_action(env: &DevfinityEnv, action: Value) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(
        ureq::post(&format!("{}/v1/app/actions", env.hosted_web_device_url))
            .set("authorization", &format!("Bearer {}", env.hosted_api_token))
            .set("x-finite-workos-user-id", &env.workos_user_id)
            .send_json(action)?
            .into_json()?,
    )
}

struct DevfinityEnv {
    core_url: String,
    dashboard_url: String,
    finitechat_url: String,
    hosted_web_device_url: String,
    finitesites_api_url: String,
    operator_access_token: String,
    customer_access_token: String,
    hosted_api_token: String,
    workos_user_id: String,
    profile: String,
}

impl DevfinityEnv {
    fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let fixture_dir = PathBuf::from(env::var("DEVFINITY_STATE_DIR")?).join("workos-fixture");
        Ok(Self {
            core_url: trim_trailing_slash(env::var("FC_CORE_URL")?),
            dashboard_url: trim_trailing_slash(env::var("FC_DASHBOARD_URL")?),
            finitechat_url: trim_trailing_slash(env::var("FINITECHAT_SERVER_URL")?),
            hosted_web_device_url: trim_trailing_slash(env::var("FC_HOSTED_WEB_DEVICE_URL")?),
            finitesites_api_url: trim_trailing_slash(env::var("FINITE_SITES_API")?),
            operator_access_token: read_nonempty_token(fixture_dir.join("operator.jwt"))?,
            customer_access_token: read_nonempty_token(fixture_dir.join("dashboard-customer.jwt"))?,
            hosted_api_token: env::var("FINITECHAT_HOSTED_API_TOKEN")?,
            workos_user_id: env::var("FC_DASHBOARD_DEV_WORKOS_USER_ID")?,
            profile: env::var("DEVFINITY_PROFILE")?,
        })
    }
}

fn read_nonempty_token(path: PathBuf) -> Result<String, Box<dyn std::error::Error>> {
    let token = fs::read_to_string(path)?.trim().to_string();
    if token.is_empty() {
        return Err("WorkOS fixture token was empty".into());
    }
    Ok(token)
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
