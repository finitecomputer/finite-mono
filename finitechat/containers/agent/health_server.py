#!/usr/bin/env python3
"""Narrow health/contact endpoint for the Finite Agent Runtime."""

from __future__ import annotations

import json
import os
import subprocess
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

AGENT_HOME = Path(os.environ.get("FINITECHAT_HOME", "/data/agent"))
FINITECHAT_BIN = os.environ.get("FINITECHAT_BIN", "/usr/local/bin/finitechat")
HOST = os.environ.get("FINITE_AGENT_HTTP_HOST", "0.0.0.0")
PORT = int(os.environ.get("FINITE_AGENT_HTTP_PORT", "8080"))
BRIDGE_STATUS_FILE = AGENT_HOME / "hermes-bridge-status.json"
AGENTD_STATUS_FILE = AGENT_HOME / "agentd" / "status.json"
STARTUP_REPORT_FILE = AGENT_HOME / "startup-report.json"
RECOVERY_BOOT_INTENT_ACTIVE = bool(os.environ.get("FINITE_AGENT_BOOT_INTENT_JSON"))
AGENTD_REQUIRED = os.environ.get("FINITE_AGENTD_REQUIRED", "").lower() in {
    "1",
    "true",
    "yes",
    "on",
}
RECOVERY_ACTIONS = {
    "canonical_plugin_reinstall",
    "generated_finitechat_config_reconcile",
    "home_channel_reconcile",
    "interrupted_turn_recovery",
    "transient_cleanup",
}
RECOVERY_ACTION_STATUSES = {
    "changed",
    "completed",
    "not_configured",
    "not_needed",
    "preserved",
    "verified",
}
RECOVERY_ERROR_CODES = {
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
}
RECOVERY_REMEDIATIONS = {
    "fix_runtime_spec_boot_intent",
    "restore_or_escalate",
    "upgrade_runtime_image",
}


def identity() -> dict[str, Any]:
    config_path = AGENT_HOME / "config.json"
    if not config_path.exists():
        return {"ready": False, "error": "agent home is not initialized"}
    try:
        proc = subprocess.run(
            [FINITECHAT_BIN, "auth", "status"],
            capture_output=True,
            check=True,
            text=True,
            timeout=5,
        )
        value = json.loads(proc.stdout)
    except Exception as exc:
        return {"ready": False, "error": str(exc)}
    return {
        "ready": True,
        "npub": value.get("npub"),
        "account_id": value.get("account_id"),
    }


def bridge_status() -> dict[str, Any]:
    try:
        payload = json.loads(BRIDGE_STATUS_FILE.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return {"status": "starting", "ok": False}
    except Exception as exc:
        return {"status": "unavailable", "ok": False, "error": str(exc)}
    if not isinstance(payload, dict):
        return {
            "status": "unavailable",
            "ok": False,
            "error": "bridge status is not an object",
        }
    return payload


def agentd_status() -> dict[str, Any]:
    try:
        payload = json.loads(AGENTD_STATUS_FILE.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return {"status": "starting", "ok": not AGENTD_REQUIRED}
    except Exception as exc:
        return {"status": "unavailable", "ok": False, "error": str(exc)}
    processes = payload.get("processes", {}).get("processes", {})
    required = ("finitechat", "health", "hermes")
    ok = all(processes.get(name, {}).get("state") == "running" for name in required)
    return {
        "status": "running" if ok else "starting",
        "ok": ok,
        "version": payload.get("version"),
        "processes": {name: processes.get(name, {}).get("state", "starting") for name in required},
    }


def _invalid_startup_report(
    error_code: str = "startup_report_invalid",
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "unavailable",
        "phase": "blocked",
        "ok": False,
        "error_code": error_code,
    }


def startup_report() -> dict[str, Any] | None:
    try:
        payload = json.loads(STARTUP_REPORT_FILE.read_text(encoding="utf-8"))
    except FileNotFoundError:
        if RECOVERY_BOOT_INTENT_ACTIVE:
            return _invalid_startup_report("startup_report_missing_for_recovery_intent")
        return None
    except Exception:
        return _invalid_startup_report()
    if (
        not isinstance(payload, dict)
        or payload.get("schema_version") != 1
        or payload.get("report_kind") != "finite_agent_startup"
        or payload.get("boot_mode") != "recover_known_good"
        or payload.get("status") not in {"running", "completed", "refused"}
        or payload.get("phase")
        not in {"intent_validation", "preflight", "repair", "complete", "blocked"}
    ):
        return _invalid_startup_report()

    status = payload.get("status")
    actions = payload.get("actions")
    refusals = payload.get("refusals")
    identity = payload.get("identity")
    safe_actions = []
    if isinstance(actions, list):
        for action in actions:
            if not isinstance(action, dict):
                continue
            action_name = action.get("action")
            action_status = action.get("status")
            if action_name not in RECOVERY_ACTIONS or action_status not in RECOVERY_ACTION_STATUSES:
                continue
            safe_action = {"action": action_name, "status": action_status}
            count = action.get("count")
            if isinstance(count, int) and not isinstance(count, bool) and count >= 0:
                safe_action["count"] = count
            safe_actions.append(safe_action)
    safe_refusals = []
    if isinstance(refusals, list):
        for refusal in refusals:
            if not isinstance(refusal, dict):
                continue
            code = refusal.get("code")
            remediation = refusal.get("remediation")
            if code not in RECOVERY_ERROR_CODES or remediation not in RECOVERY_REMEDIATIONS:
                continue
            safe_refusals.append({"code": code, "remediation": remediation})
    operation_id_hash = payload.get("operation_id_hash")
    if not (
        isinstance(operation_id_hash, str)
        and len(operation_id_hash) == len("sha256:") + 64
        and operation_id_hash.startswith("sha256:")
        and all(character in "0123456789abcdef" for character in operation_id_hash[7:])
    ):
        operation_id_hash = None
    idempotency = payload.get("idempotency")
    if not isinstance(idempotency, dict):
        idempotency = {}
    safe_idempotency = {}
    if idempotency.get("same_operation_replay") == "no_op_after_terminal_state":
        safe_idempotency["same_operation_replay"] = "no_op_after_terminal_state"
    resumed = idempotency.get("resumed_after_interruption")
    if isinstance(resumed, bool):
        safe_idempotency["resumed_after_interruption"] = resumed
    state_roots = payload.get("state_roots")
    if not isinstance(state_roots, dict):
        state_roots = {}
    safe_state_roots = {}
    for label in ("/data", "/data/agent", "/data/agent/hermes-home", "/data/workspace"):
        value = state_roots.get(label)
        if not isinstance(value, dict):
            continue
        safe_state_roots[label] = {
            "present": value.get("present") is True,
            "writable": value.get("writable") is True,
        }
    acceptance_scope = payload.get("acceptance_scope")
    if not isinstance(acceptance_scope, dict):
        acceptance_scope = {}
    safe_acceptance_scope = {
        key: acceptance_scope.get(key)
        for key in ("runtime_spec_delivery", "provider_conformance", "phala_acceptance")
        if acceptance_scope.get(key) == "not_proven"
    }
    public_identity = identity.get("npub") if isinstance(identity, dict) else None
    if not (
        isinstance(public_identity, str)
        and public_identity.startswith("npub1")
        and len(public_identity) <= 100
        and all(character.isascii() and character.isalnum() for character in public_identity)
    ):
        public_identity = None
    error_code = payload.get("error_code")
    if error_code not in RECOVERY_ERROR_CODES:
        error_code = None
    safe = {
        "schema_version": 1,
        "report_kind": "finite_agent_startup",
        "boot_mode": "recover_known_good",
        "status": status,
        "phase": payload.get("phase"),
        "ok": status == "completed",
        "error_code": error_code,
        "operation_id_hash": operation_id_hash,
        "idempotency": safe_idempotency,
        "actions": safe_actions,
        "refusals": safe_refusals,
        "identity": {"npub": public_identity},
        "state_roots": safe_state_roots,
        "acceptance_scope": safe_acceptance_scope,
    }
    return safe


def runtime_health() -> dict[str, Any]:
    payload = identity()
    # `npub` remains the generic identity-health field. `agent_npub` is the
    # stable Finite Chat contact coordinate used by Hosted Web, Electron, and
    # native Devices to perform MLS Add + Welcome admission.
    payload["agent_npub"] = payload.get("npub")
    payload["bridge"] = bridge_status()
    payload["agentd"] = agentd_status()
    startup = startup_report()
    if startup is not None:
        payload["startup"] = startup
    return payload


def runtime_ready(payload: dict[str, Any]) -> bool:
    return (
        bool(payload.get("ready"))
        and payload.get("bridge", {}).get("ok") is not False
        and payload.get("agentd", {}).get("ok") is not False
        and payload.get("startup", {}).get("ok") is not False
    )


class Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        if self.path == "/healthz":
            payload = runtime_health()
            self._write(200 if runtime_ready(payload) else 503, payload)
            return
        if self.path in {"/contact", "/invite"}:
            # `/invite` is a temporary URL-compatibility alias for already
            # recorded Runtime facts. It serves contact metadata only and does
            # not recreate the removed invite-session admission protocol.
            payload = runtime_health()
            self._write(200 if runtime_ready(payload) else 503, payload)
            return
        self._write(404, {"ready": False, "error": "not found"})

    def log_message(self, fmt: str, *args: object) -> None:
        return

    def _write(self, status: int, payload: dict[str, Any]) -> None:
        body = json.dumps(payload, sort_keys=True).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main() -> None:
    server = ThreadingHTTPServer((HOST, PORT), Handler)
    server.serve_forever()


if __name__ == "__main__":
    main()
