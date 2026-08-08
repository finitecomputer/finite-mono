//! `/healthz` + `/contact` (+ `/invite` alias), replicating
//! `finitechat/containers/agent/health_server.py`'s response contract exactly
//! for existing fields — the runner and smoke parse them — plus the shell's
//! new generation-state fields.
//!
//! Differences from the Python server, by design:
//! - the npub comes from the `/data` identity file
//!   (`<agent home>/identity/identity.json`, the shared Finite identity),
//!   not from shelling out to `finitechat auth status`;
//! - the shell adds `shell_version`, `payload_version`, `channel`,
//!   `payload_generations`, `last_flip`, `last_poll`, and
//!   `agentd_supervision`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::state::ShellState;
use crate::supervise::AgentdSupervisionStatus;
use crate::{DataLayout, SHELL_VERSION, ShellSettings, generations};

// The vocabularies health_server.py sanitizes the startup report against,
// byte for byte.
static RECOVERY_ACTIONS: LazyLock<BTreeSet<&'static str>> = LazyLock::new(|| {
    BTreeSet::from([
        "canonical_plugin_reinstall",
        "generated_finitechat_config_reconcile",
        "home_channel_reconcile",
        "interrupted_turn_recovery",
        "transient_cleanup",
    ])
});
static RECOVERY_ACTION_STATUSES: LazyLock<BTreeSet<&'static str>> = LazyLock::new(|| {
    BTreeSet::from([
        "changed",
        "completed",
        "not_configured",
        "not_needed",
        "preserved",
        "verified",
    ])
});
static RECOVERY_ERROR_CODES: LazyLock<BTreeSet<&'static str>> = LazyLock::new(|| {
    BTreeSet::from([
        "agent_config_changed_during_recovery",
        "agent_config_missing_or_corrupt",
        "agent_home_missing_or_unsafe",
        "boot_intent_invalid",
        "boot_intent_kind_unsupported",
        "boot_intent_operation_id_invalid",
        "boot_intent_version_unsupported",
        "client_store_missing_or_corrupt",
        "client_store_replaced_during_recovery",
        "generated_config_missing_or_corrupt",
        "generated_config_path_mismatch",
        "generated_config_settings_invalid",
        "hermes_home_missing_or_unsafe",
        "home_channel_metadata_corrupt",
        "home_channel_metadata_unsafe",
        "home_channel_reconcile_failed",
        "identity_account_mismatch",
        "identity_changed_during_recovery",
        "identity_missing_or_corrupt",
        "identity_root_contract_mismatch",
        "internal_recovery_error",
        "interrupted_turn_recovery_failed",
        "interrupted_turn_state_corrupt",
        "managed_skills_path_invalid",
        "noncanonical_plugin_name",
        "operation_journal_corrupt",
        "operation_journal_unsafe",
        "plugin_destination_unsafe",
        "plugin_reinstall_failed",
        "finitechat_binary_missing_or_unsafe",
        "state_root_contract_mismatch",
        "state_root_missing_or_unsafe",
        "startup_report_unwritable",
        "startup_report_missing_or_corrupt",
        "startup_report_missing_for_recovery_intent",
        "startup_report_terminal_mismatch",
        "test_override_forbidden",
        "transient_cleanup_failed",
        "transient_path_unsafe",
        "workspace_missing_or_unsafe",
    ])
});
static RECOVERY_REMEDIATIONS: LazyLock<BTreeSet<&'static str>> = LazyLock::new(|| {
    BTreeSet::from([
        "fix_runtime_spec_boot_intent",
        "restore_or_escalate",
        "upgrade_runtime_image",
    ])
});

/// `identity()` from health_server.py, with the npub read from the shared
/// `/data` identity file instead of shelling into the payload.
fn identity(layout: &DataLayout) -> Value {
    if !layout.agent_config_file().exists() {
        return json!({ "ready": false, "error": "agent home is not initialized" });
    }
    match read_identity(layout) {
        Ok((npub, account_id)) => json!({
            "ready": true,
            "npub": npub,
            "account_id": account_id,
        }),
        Err(error) => json!({ "ready": false, "error": error }),
    }
}

fn read_identity(layout: &DataLayout) -> Result<(String, Value), String> {
    let config: Value = fs::read(layout.agent_config_file())
        .map_err(|error| error.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|error| error.to_string()))?;
    let account_id = config.get("account_id").cloned().unwrap_or(Value::Null);
    let identity: Value = fs::read(layout.identity_file())
        .map_err(|error| format!("identity is unavailable: {error}"))
        .and_then(|bytes| {
            serde_json::from_slice(&bytes)
                .map_err(|error| format!("identity is unavailable: {error}"))
        })?;
    let public_key_hex = identity
        .get("public_key_hex")
        .and_then(Value::as_str)
        .ok_or_else(|| "identity is unavailable: no public_key_hex".to_owned())?;
    let bytes: [u8; 32] = hex::decode(public_key_hex.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| "identity is unavailable: public_key_hex is not 32 bytes".to_owned())?;
    Ok((encode_npub(&bytes), account_id))
}

/// NIP-19 npub encoding, byte-for-byte the same as
/// `finite_identity::npub::encode`.
pub fn encode_npub(public_key: &[u8; 32]) -> String {
    let hrp = bech32::Hrp::parse("npub").expect("static hrp is valid");
    bech32::encode::<bech32::Bech32>(hrp, public_key)
        .expect("32-byte payload is within bech32 limits")
}

/// `bridge_status()` from health_server.py.
fn bridge_status(layout: &DataLayout) -> Value {
    match fs::read_to_string(layout.bridge_status_file()) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            json!({ "status": "starting", "ok": false })
        }
        Err(error) => json!({ "status": "unavailable", "ok": false, "error": error.to_string() }),
        Ok(contents) => match serde_json::from_str::<Value>(&contents) {
            Ok(Value::Object(payload)) => Value::Object(payload),
            Ok(_) => json!({
                "status": "unavailable",
                "ok": false,
                "error": "bridge status is not an object",
            }),
            Err(error) => {
                json!({ "status": "unavailable", "ok": false, "error": error.to_string() })
            }
        },
    }
}

/// `agentd_status()` from health_server.py (`FINITE_AGENTD_REQUIRED`
/// semantics included).
fn agentd_status(layout: &DataLayout, agentd_required: bool) -> Value {
    let payload: Value = match fs::read_to_string(layout.agentd_status_file()) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return json!({ "status": "starting", "ok": !agentd_required });
        }
        Err(error) => {
            return json!({ "status": "unavailable", "ok": false, "error": error.to_string() });
        }
        Ok(contents) => match serde_json::from_str(&contents) {
            Ok(value) => value,
            Err(error) => {
                return json!({
                    "status": "unavailable",
                    "ok": false,
                    "error": error.to_string(),
                });
            }
        },
    };
    let processes = payload
        .get("processes")
        .and_then(|value| value.get("processes"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let required = ["finitechat", "health", "hermes"];
    let ok = required.iter().all(|name| {
        processes
            .get(name)
            .and_then(|process| process.get("state"))
            .and_then(Value::as_str)
            == Some("running")
    });
    let mut process_states = Map::new();
    for name in required {
        let state = processes
            .get(name)
            .and_then(|process| process.get("state"))
            .cloned()
            .unwrap_or_else(|| Value::from("starting"));
        process_states.insert(name.to_owned(), state);
    }
    json!({
        "status": if ok { "running" } else { "starting" },
        "ok": ok,
        "version": payload.get("version").cloned().unwrap_or(Value::Null),
        "processes": Value::Object(process_states),
    })
}

fn invalid_startup_report(error_code: &str) -> Value {
    json!({
        "schema_version": 1,
        "status": "unavailable",
        "phase": "blocked",
        "ok": false,
        "error_code": error_code,
    })
}

/// `startup_report()` from health_server.py: the same field-by-field
/// sanitization against the recovery vocabularies.
fn startup_report(layout: &DataLayout, recovery_boot_intent_active: bool) -> Option<Value> {
    let payload: Value = match fs::read_to_string(layout.startup_report_file()) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if recovery_boot_intent_active {
                return Some(invalid_startup_report(
                    "startup_report_missing_for_recovery_intent",
                ));
            }
            return None;
        }
        Err(_) => return Some(invalid_startup_report("startup_report_invalid")),
        Ok(contents) => match serde_json::from_str(&contents) {
            Ok(value) => value,
            Err(_) => return Some(invalid_startup_report("startup_report_invalid")),
        },
    };
    let string_of = |value: &Value, key: &str| -> Option<String> {
        value.get(key).and_then(Value::as_str).map(str::to_owned)
    };
    let status = string_of(&payload, "status");
    let phase = string_of(&payload, "phase");
    let valid = payload.is_object()
        && payload.get("schema_version").and_then(Value::as_u64) == Some(1)
        && string_of(&payload, "report_kind").as_deref() == Some("finite_agent_startup")
        && string_of(&payload, "boot_mode").as_deref() == Some("recover_known_good")
        && matches!(status.as_deref(), Some("running" | "completed" | "refused"))
        && matches!(
            phase.as_deref(),
            Some("intent_validation" | "preflight" | "repair" | "complete" | "blocked")
        );
    if !valid {
        return Some(invalid_startup_report("startup_report_invalid"));
    }
    let status = status.expect("validated above");

    let mut safe_actions = Vec::new();
    if let Some(actions) = payload.get("actions").and_then(Value::as_array) {
        for action in actions {
            let (Some(action_name), Some(action_status)) = (
                action.get("action").and_then(Value::as_str),
                action.get("status").and_then(Value::as_str),
            ) else {
                continue;
            };
            if !RECOVERY_ACTIONS.contains(action_name)
                || !RECOVERY_ACTION_STATUSES.contains(action_status)
            {
                continue;
            }
            let mut safe_action = Map::new();
            safe_action.insert("action".to_owned(), Value::from(action_name));
            safe_action.insert("status".to_owned(), Value::from(action_status));
            if let Some(count) = action.get("count").and_then(Value::as_i64)
                && !action.get("count").is_some_and(Value::is_boolean)
                && count >= 0
            {
                safe_action.insert("count".to_owned(), Value::from(count));
            }
            safe_actions.push(Value::Object(safe_action));
        }
    }
    let mut safe_refusals = Vec::new();
    if let Some(refusals) = payload.get("refusals").and_then(Value::as_array) {
        for refusal in refusals {
            let (Some(code), Some(remediation)) = (
                refusal.get("code").and_then(Value::as_str),
                refusal.get("remediation").and_then(Value::as_str),
            ) else {
                continue;
            };
            if !RECOVERY_ERROR_CODES.contains(code) || !RECOVERY_REMEDIATIONS.contains(remediation)
            {
                continue;
            }
            safe_refusals.push(json!({ "code": code, "remediation": remediation }));
        }
    }
    let operation_id_hash = string_of(&payload, "operation_id_hash")
        .filter(|hash| {
            hash.len() == "sha256:".len() + 64
                && hash.starts_with("sha256:")
                && hash["sha256:".len()..]
                    .chars()
                    .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
        })
        .map(Value::from)
        .unwrap_or(Value::Null);
    let idempotency = payload
        .get("idempotency")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut safe_idempotency = Map::new();
    if idempotency
        .get("same_operation_replay")
        .and_then(Value::as_str)
        == Some("no_op_after_terminal_state")
    {
        safe_idempotency.insert(
            "same_operation_replay".to_owned(),
            Value::from("no_op_after_terminal_state"),
        );
    }
    if let Some(resumed) = idempotency
        .get("resumed_after_interruption")
        .and_then(Value::as_bool)
    {
        safe_idempotency.insert(
            "resumed_after_interruption".to_owned(),
            Value::from(resumed),
        );
    }
    let state_roots = payload
        .get("state_roots")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut safe_state_roots = Map::new();
    for label in [
        "/data",
        "/data/agent",
        "/data/agent/hermes-home",
        "/data/workspace",
    ] {
        let Some(value) = state_roots.get(label).and_then(Value::as_object) else {
            continue;
        };
        safe_state_roots.insert(
            label.to_owned(),
            json!({
                "present": value.get("present") == Some(&Value::Bool(true)),
                "writable": value.get("writable") == Some(&Value::Bool(true)),
            }),
        );
    }
    let acceptance_scope = payload
        .get("acceptance_scope")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut safe_acceptance_scope = Map::new();
    for key in [
        "runtime_spec_delivery",
        "provider_conformance",
        "phala_acceptance",
    ] {
        if acceptance_scope.get(key).and_then(Value::as_str) == Some("not_proven") {
            safe_acceptance_scope.insert(key.to_owned(), Value::from("not_proven"));
        }
    }
    let public_identity = payload
        .get("identity")
        .and_then(|identity| identity.get("npub"))
        .and_then(Value::as_str)
        .filter(|npub| {
            npub.starts_with("npub1")
                && npub.len() <= 100
                && npub
                    .chars()
                    .all(|character| character.is_ascii() && character.is_ascii_alphanumeric())
        })
        .map(Value::from)
        .unwrap_or(Value::Null);
    let error_code = string_of(&payload, "error_code")
        .filter(|code| RECOVERY_ERROR_CODES.contains(code.as_str()))
        .map(Value::from)
        .unwrap_or(Value::Null);

    Some(json!({
        "schema_version": 1,
        "report_kind": "finite_agent_startup",
        "boot_mode": "recover_known_good",
        "status": status,
        "phase": phase,
        "ok": status == "completed",
        "error_code": error_code,
        "operation_id_hash": operation_id_hash,
        "idempotency": Value::Object(safe_idempotency),
        "actions": safe_actions,
        "refusals": safe_refusals,
        "identity": { "npub": public_identity },
        "state_roots": Value::Object(safe_state_roots),
        "acceptance_scope": Value::Object(safe_acceptance_scope),
    }))
}

/// `runtime_health()` + the shell's new fields.
///
/// Beyond the file-derived compat fields, readiness requires the LIVE
/// supervision truth: files written by dead processes keep looking fresh,
/// but the shell knows in memory whether agentd is actually running. A dead
/// or crash-looping agentd forces `ready: false` with a distinct
/// `ready_reason`, as does a flip whose restoration failed (`failed_open`).
pub fn runtime_health(
    settings: &ShellSettings,
    layout: &DataLayout,
    shell_state: &ShellState,
    agentd_supervision: Option<&AgentdSupervisionStatus>,
) -> Value {
    let mut payload = identity(layout);
    // `npub` remains the generic identity-health field. `agent_npub` is the
    // stable Finite Chat contact coordinate (same comment as
    // health_server.py — the runner parses it from /contact).
    payload["agent_npub"] = payload.get("npub").cloned().unwrap_or(Value::Null);
    payload["bridge"] = bridge_status(layout);
    payload["agentd"] = agentd_status(layout, settings.agentd_required);
    if let Some(startup) = startup_report(layout, settings.recovery_boot_intent_active) {
        payload["startup"] = startup;
    }

    // New shell-owned fields.
    let current = generations::read_link_version(&layout.current_link());
    let previous = generations::read_link_version(&layout.previous_link());
    payload["shell_version"] = Value::from(SHELL_VERSION);
    payload["payload_version"] = current.clone().map(Value::from).unwrap_or(Value::Null);
    payload["channel"] = Value::from(layout.channel_name());
    payload["payload_generations"] = json!({
        "current": current,
        "previous": previous,
        "staged": shell_state
            .staged
            .as_ref()
            .map(|staged| staged.version_label.clone()),
        "bad": shell_state
            .bad
            .iter()
            .map(|bad| bad.version_label.clone())
            .collect::<Vec<_>>(),
    });
    payload["last_flip"] = shell_state
        .last_flip
        .as_ref()
        .and_then(|flip| serde_json::to_value(flip).ok())
        .unwrap_or(Value::Null);
    payload["last_poll"] = shell_state
        .last_poll
        .as_ref()
        .and_then(|poll| serde_json::to_value(poll).ok())
        .unwrap_or(Value::Null);
    payload["agentd_supervision"] = agentd_supervision
        .and_then(|status| serde_json::to_value(status).ok())
        .unwrap_or(Value::Null);

    // Live-supervision readiness gate: the file-derived fields above stay
    // untouched (they are the compat contract), but `ready` must not be
    // claimable by fresh-looking files a dead process left behind.
    let mut deny_ready = |reason: &str| {
        payload["ready"] = Value::from(false);
        if payload.get("ready_reason").is_none() {
            payload["ready_reason"] = Value::from(reason.to_owned());
        }
    };
    // The most alarming condition wins the reason field: a flip that failed
    // open means the machine may be running an unverified candidate.
    if shell_state
        .last_flip
        .as_ref()
        .is_some_and(|flip| flip.outcome == crate::state::FlipOutcome::FailedOpen)
    {
        deny_ready("flip_failed_open");
    }
    match agentd_supervision {
        None => deny_ready("agentd_not_running"),
        Some(supervision) => {
            if supervision.crash_looping {
                deny_ready("agentd_crash_looping");
            } else if supervision.state != "running" {
                deny_ready("agentd_not_running");
            }
        }
    }
    payload
}

/// `runtime_ready()` from health_server.py: `x.get("ok") is not False`
/// semantics — a missing or non-boolean `ok` passes.
pub fn runtime_ready(payload: &Value) -> bool {
    let ok_not_false = |key: &str| {
        payload.get(key).and_then(|section| section.get("ok")) != Some(&Value::Bool(false))
    };
    payload.get("ready") == Some(&Value::Bool(true))
        && ok_not_false("bridge")
        && ok_not_false("agentd")
        && ok_not_false("startup")
}

/// One HTTP response, replicating the Python handler's routing: `/healthz`,
/// `/contact`, and the `/invite` compatibility alias serve the health body
/// (200 when ready, 503 otherwise); everything else is a 404 with
/// `{"ready": false, "error": "not found"}`.
pub fn respond(path: &str, body_for_health: impl FnOnce() -> Value) -> (u16, Value) {
    match path {
        "/healthz" | "/contact" | "/invite" => {
            let payload = body_for_health();
            let status = if runtime_ready(&payload) { 200 } else { 503 };
            (status, payload)
        }
        _ => (404, json!({ "ready": false, "error": "not found" })),
    }
}

/// Serve HTTP on `listener` until the process exits. `health_body` produces
/// the current health body per request — the booted runtime's full body, or
/// a degraded bootstrapping/bootstrap_failed shape before boot succeeds.
pub async fn serve_http(
    listener: TcpListener,
    health_body: impl Fn() -> Value + Send + Sync + 'static,
) {
    let health_body = std::sync::Arc::new(health_body);
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let health_body = std::sync::Arc::clone(&health_body);
        tokio::spawn(async move {
            let mut buffer = vec![0_u8; 8192];
            let read = stream.read(&mut buffer).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
            let path = request
                .split_whitespace()
                .nth(1)
                .unwrap_or("/")
                .split('?')
                .next()
                .unwrap_or("/")
                .to_owned();
            let (status, payload) = respond(&path, || health_body());
            // serde_json's default map is sorted, matching Python's
            // json.dumps(..., sort_keys=True).
            let body = payload.to_string();
            let reason = match status {
                200 => "OK",
                503 => "Service Unavailable",
                _ => "Not Found",
            };
            let head = format!(
                "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(body.as_bytes()).await;
            let _ = stream.shutdown().await;
        });
    }
}

/// True when `path` (relative to the agent home) was freshly written at or
/// after `not_before` — the flip health gate's freshness check for the
/// agentd status file.
pub fn file_written_since(path: &Path, not_before: std::time::SystemTime) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| modified >= not_before)
}
