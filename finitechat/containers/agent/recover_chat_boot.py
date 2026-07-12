#!/usr/bin/env python3
"""Run the image-owned, one-shot Finite Chat recovery boot operation."""

from __future__ import annotations

import argparse
import contextlib
import fcntl
import hashlib
import json
import os
import re
import shutil
import sqlite3
import stat
import subprocess
import sys
import time
import urllib.parse
from pathlib import Path
from typing import Any

from reconcile_hermes_config import (
    ConfigError,
    _atomic_write,
    _dump,
    _load,
    reconcile_config,
)


class RecoveryRefusal(RuntimeError):
    """Recovery cannot continue without risking durable user state."""

    def __init__(self, code: str, remediation: str = "restore_or_escalate"):
        super().__init__(code)
        self.code = code
        self.remediation = remediation


class InjectedRecoveryCrash(RuntimeError):
    """A test-only interruption at a journal boundary."""


RECOVERY_INTENT_ENV = "FINITE_AGENT_BOOT_INTENT_JSON"
RECOVERY_TESTING_ENV = "FINITE_RECOVER_CHAT_TESTING"
RECOVERY_FAILPOINT_ENV = "FINITE_RECOVER_CHAT_TEST_FAILPOINT"
RECOVERY_INTENT_SCHEMA_VERSION = 1
STARTUP_REPORT_SCHEMA_VERSION = 1
RECOVERY_KIND = "recover_known_good"
CANONICAL_PLUGIN_NAME = "finitechat"
OPERATION_ID_PATTERN = re.compile(r"^[^\x00-\x1f\x7f]{1,200}$")
REQUIRED_CLIENT_TABLES = {
    "client_device_states",
    "client_app_messages",
    "client_app_events",
    "client_app_outbox",
    "client_app_rooms",
    "client_app_state",
    "client_app_profiles",
}
TRANSIENT_AGENT_FILES = {
    "hermes-service.json": "finitechat_service_ready",
    "hermes-bridge-status.json": "bridge_health_cache",
    "hermes-gateway.pid": "hermes_gateway_pid",
    "finitechat-hermes.pid": "finitechat_service_pid",
}
TRANSIENT_AGENTD_FILES = {
    "finitechat-ready.json": "agentd_finitechat_ready",
    "status.json": "agentd_health_cache",
    "finitechat.pid": "agentd_finitechat_pid",
    "health.pid": "agentd_health_pid",
    "hermes.pid": "agentd_hermes_pid",
}


def _atomic_json(path: Path, value: dict[str, Any]) -> None:
    _atomic_write(path, json.dumps(value, indent=2, sort_keys=True) + "\n")


def _operation_hash(operation_id: str) -> str:
    return "sha256:" + hashlib.sha256(operation_id.encode("utf-8")).hexdigest()


def _parse_recovery_intent() -> str:
    raw = os.environ.get(RECOVERY_INTENT_ENV, "")
    try:
        value = json.loads(raw)
    except (json.JSONDecodeError, UnicodeError) as exc:
        raise RecoveryRefusal("boot_intent_invalid", "fix_runtime_spec_boot_intent") from exc
    if not isinstance(value, dict) or set(value) != {
        "schema_version",
        "kind",
        "operation_id",
    }:
        raise RecoveryRefusal("boot_intent_invalid", "fix_runtime_spec_boot_intent")
    if value.get("schema_version") != RECOVERY_INTENT_SCHEMA_VERSION:
        raise RecoveryRefusal("boot_intent_version_unsupported", "upgrade_runtime_image")
    if value.get("kind") != RECOVERY_KIND:
        raise RecoveryRefusal("boot_intent_kind_unsupported", "fix_runtime_spec_boot_intent")
    operation_id = value.get("operation_id")
    if not isinstance(operation_id, str) or not OPERATION_ID_PATTERN.fullmatch(operation_id):
        raise RecoveryRefusal("boot_intent_operation_id_invalid", "fix_runtime_spec_boot_intent")
    return _operation_hash(operation_id)


def _lstat(path: Path, code: str) -> os.stat_result:
    try:
        return path.lstat()
    except OSError as exc:
        raise RecoveryRefusal(code) from exc


def _require_directory(path: Path, code: str) -> None:
    mode = _lstat(path, code).st_mode
    if stat.S_ISLNK(mode) or not stat.S_ISDIR(mode):
        raise RecoveryRefusal(code)


def _require_regular_file(path: Path, code: str) -> os.stat_result:
    info = _lstat(path, code)
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
        raise RecoveryRefusal(code)
    return info


def _read_json_object(path: Path, code: str) -> dict[str, Any]:
    _require_regular_file(path, code)
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise RecoveryRefusal(code) from exc
    if not isinstance(value, dict):
        raise RecoveryRefusal(code)
    return value


def _run_json_command(args: list[str], code: str) -> dict[str, Any]:
    child_env = dict(os.environ)
    child_env.pop(RECOVERY_INTENT_ENV, None)
    child_env.pop(RECOVERY_FAILPOINT_ENV, None)
    child_env.pop(RECOVERY_TESTING_ENV, None)
    try:
        result = subprocess.run(
            args,
            capture_output=True,
            check=False,
            env=child_env,
            text=True,
            timeout=30,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise RecoveryRefusal(code) from exc
    if result.returncode != 0:
        raise RecoveryRefusal(code)
    try:
        value = json.loads(result.stdout)
    except (json.JSONDecodeError, UnicodeError) as exc:
        raise RecoveryRefusal(code) from exc
    if not isinstance(value, dict):
        raise RecoveryRefusal(code)
    return value


def _validate_agent_config(agent_home: Path) -> dict[str, str]:
    value = _read_json_object(agent_home / "config.json", "agent_config_missing_or_corrupt")
    normalized: dict[str, str] = {}
    for key in ("server_url", "device_id", "account_id"):
        item = value.get(key)
        if not isinstance(item, str) or not item.strip():
            raise RecoveryRefusal("agent_config_missing_or_corrupt")
        normalized[key] = item.strip()
    account_id = normalized["account_id"]
    if len(account_id) != 64 or any(
        character not in "0123456789abcdef" for character in account_id
    ):
        raise RecoveryRefusal("agent_config_missing_or_corrupt")
    return normalized


def _validate_identity(
    finite_home: Path, finitechat_bin: Path, account_id: str
) -> tuple[str, tuple[int, int]]:
    identity_path = finite_home / "identity" / "identity.json"
    identity_info = _require_regular_file(identity_path, "identity_missing_or_corrupt")
    status = _run_json_command(
        [str(finitechat_bin), "auth", "status"], "identity_missing_or_corrupt"
    )
    if status.get("account_id") != account_id:
        raise RecoveryRefusal("identity_account_mismatch")
    npub = status.get("npub")
    if not isinstance(npub, str) or not npub.startswith("npub1"):
        raise RecoveryRefusal("identity_missing_or_corrupt")
    return npub, (identity_info.st_dev, identity_info.st_ino)


def _validate_client_store(path: Path) -> tuple[int, int]:
    info = _require_regular_file(path, "client_store_missing_or_corrupt")
    uri = "file:" + urllib.parse.quote(path.as_posix(), safe="/") + "?mode=ro"
    try:
        connection = sqlite3.connect(uri, uri=True, timeout=5)
        try:
            if connection.execute("PRAGMA quick_check").fetchone() != ("ok",):
                raise RecoveryRefusal("client_store_missing_or_corrupt")
            tables = {
                str(row[0])
                for row in connection.execute(
                    "SELECT name FROM sqlite_master WHERE type = 'table'"
                ).fetchall()
            }
            if not REQUIRED_CLIENT_TABLES.issubset(tables):
                raise RecoveryRefusal("client_store_missing_or_corrupt")
        finally:
            connection.close()
    except RecoveryRefusal:
        raise
    except (OSError, sqlite3.Error) as exc:
        raise RecoveryRefusal("client_store_missing_or_corrupt") from exc
    return info.st_dev, info.st_ino


def _validate_plugin_destination(hermes_home: Path) -> None:
    plugins = hermes_home / "plugins"
    if os.path.lexists(plugins):
        _require_directory(plugins, "plugin_destination_unsafe")
    canonical = plugins / CANONICAL_PLUGIN_NAME
    if not os.path.lexists(canonical):
        return
    canonical_mode = _lstat(canonical, "plugin_destination_unsafe").st_mode
    if stat.S_ISLNK(canonical_mode) or not (
        stat.S_ISDIR(canonical_mode) or stat.S_ISREG(canonical_mode)
    ):
        raise RecoveryRefusal("plugin_destination_unsafe")
    if stat.S_ISREG(canonical_mode):
        return
    for path in canonical.rglob("*"):
        mode = _lstat(path, "plugin_destination_unsafe").st_mode
        if stat.S_ISLNK(mode) or not (stat.S_ISDIR(mode) or stat.S_ISREG(mode)):
            raise RecoveryRefusal("plugin_destination_unsafe")


def _plugin_digest(path: Path) -> str | None:
    if not path.exists():
        return None
    digest = hashlib.sha256(b"finitechat-plugin-v1\0")
    if path.is_file():
        try:
            digest.update(b"f")
            with path.open("rb") as handle:
                while chunk := handle.read(1024 * 1024):
                    digest.update(chunk)
        except OSError as exc:
            raise RecoveryRefusal("plugin_destination_unsafe") from exc
        return "sha256:" + digest.hexdigest()
    for item in sorted(path.rglob("*")):
        relative = item.relative_to(path).as_posix().encode("utf-8")
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        if item.is_dir():
            digest.update(b"d")
            continue
        digest.update(b"f")
        try:
            with item.open("rb") as handle:
                while chunk := handle.read(1024 * 1024):
                    digest.update(chunk)
        except OSError as exc:
            raise RecoveryRefusal("plugin_destination_unsafe") from exc
    return "sha256:" + digest.hexdigest()


def _validate_reconcile_settings(
    settings: dict[str, str], agent_home: Path, finitechat_bin: Path
) -> None:
    required_nonempty = (
        "FINITE_CONFIG_PLUGIN_NAME",
        "FINITE_CONFIG_AGENT_HOME",
        "FINITE_CONFIG_FINITECHAT_BIN",
        "FINITE_CONFIG_SERVICE_ADDR",
        "FINITE_CONFIG_POLL_TIMEOUT_SECS",
        "FINITE_CONFIG_POLL_LIMIT",
    )
    if any(not settings.get(key, "").strip() for key in required_nonempty):
        raise RecoveryRefusal("generated_config_settings_invalid")
    if settings["FINITE_CONFIG_PLUGIN_NAME"] != CANONICAL_PLUGIN_NAME:
        raise RecoveryRefusal("noncanonical_plugin_name")
    if Path(settings["FINITE_CONFIG_AGENT_HOME"]) != agent_home:
        raise RecoveryRefusal("generated_config_settings_invalid")
    if Path(settings["FINITE_CONFIG_FINITECHAT_BIN"]) != finitechat_bin:
        raise RecoveryRefusal("generated_config_settings_invalid")
    try:
        timeout = int(settings["FINITE_CONFIG_POLL_TIMEOUT_SECS"])
        limit = int(settings["FINITE_CONFIG_POLL_LIMIT"])
    except ValueError as exc:
        raise RecoveryRefusal("generated_config_settings_invalid") from exc
    if timeout < 0 or limit < 1:
        raise RecoveryRefusal("generated_config_settings_invalid")
    managed_skills = settings.get("FINITE_CONFIG_MANAGED_SKILLS_DIR", "")
    if managed_skills:
        expected = agent_home / "managed-skills" / "finite" / "current"
        if Path(managed_skills) != expected or not expected.is_dir():
            raise RecoveryRefusal("managed_skills_path_invalid")


def _validate_transient_allowlist(agent_home: Path, hermes_home: Path) -> None:
    for path in (
        *(agent_home / name for name in TRANSIENT_AGENT_FILES),
        *(agent_home / "agentd" / name for name in TRANSIENT_AGENTD_FILES),
        *hermes_home.glob(".config.yaml.*"),
    ):
        if not os.path.lexists(path):
            continue
        mode = _lstat(path, "transient_path_unsafe").st_mode
        if not (stat.S_ISREG(mode) or stat.S_ISLNK(mode)):
            raise RecoveryRefusal("transient_path_unsafe")
    cache = hermes_home / "plugins" / CANONICAL_PLUGIN_NAME / "__pycache__"
    if os.path.lexists(cache):
        mode = _lstat(cache, "transient_path_unsafe").st_mode
        if not (stat.S_ISREG(mode) or stat.S_ISLNK(mode) or stat.S_ISDIR(mode)):
            raise RecoveryRefusal("transient_path_unsafe")


def _unlink_transient(path: Path, label: str, removed: list[str]) -> None:
    if not os.path.lexists(path):
        return
    try:
        path.unlink()
    except OSError as exc:
        raise RecoveryRefusal("transient_cleanup_failed") from exc
    removed.append(label)


def _clear_transient_allowlist(agent_home: Path, hermes_home: Path) -> list[str]:
    removed: list[str] = []
    for name, label in TRANSIENT_AGENT_FILES.items():
        _unlink_transient(agent_home / name, label, removed)
    for name, label in TRANSIENT_AGENTD_FILES.items():
        _unlink_transient(agent_home / "agentd" / name, label, removed)
    for path in hermes_home.glob(".config.yaml.*"):
        _unlink_transient(path, "incomplete_generated_config", removed)

    cache = hermes_home / "plugins" / CANONICAL_PLUGIN_NAME / "__pycache__"
    if os.path.lexists(cache):
        mode = _lstat(cache, "transient_path_unsafe").st_mode
        try:
            if stat.S_ISDIR(mode) and not stat.S_ISLNK(mode):
                shutil.rmtree(cache)
            else:
                cache.unlink()
        except OSError as exc:
            raise RecoveryRefusal("transient_cleanup_failed") from exc
        removed.append("canonical_plugin_python_cache")
    return sorted(removed)


def _config_home_channel(
    config: dict[str, Any],
) -> tuple[tuple[str, None] | None, bool]:
    gateway = config.get("gateway")
    if not isinstance(gateway, dict):
        return None, False
    platforms = gateway.get("platforms")
    if not isinstance(platforms, dict):
        return None, False
    finitechat = platforms.get("finitechat")
    if not isinstance(finitechat, dict):
        return None, False
    if "home_channel" not in finitechat:
        return None, False
    home_channel = finitechat.get("home_channel")
    if not isinstance(home_channel, dict):
        return None, True
    room_id = home_channel.get("chat_id")
    if not isinstance(room_id, str) or not room_id.strip():
        return None, True
    return (room_id.strip(), None), False


def _metadata_home_channel(
    agent_home: Path,
) -> tuple[tuple[str, str | None] | None, bool]:
    path = agent_home / "hermes-home-channel.json"
    if not os.path.lexists(path):
        return None, False
    info = _lstat(path, "home_channel_metadata_unsafe")
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
        raise RecoveryRefusal("home_channel_metadata_unsafe")
    try:
        value = _read_json_object(path, "home_channel_metadata_corrupt")
    except RecoveryRefusal:
        return None, True
    room_id = value.get("room_id")
    conversation_id = value.get("conversation_id")
    if not isinstance(room_id, str) or not room_id.strip():
        return None, True
    if conversation_id is not None and (
        not isinstance(conversation_id, str) or not conversation_id.strip()
    ):
        return None, True
    return (
        room_id.strip(),
        conversation_id.strip() if isinstance(conversation_id, str) else None,
    ), False


def _home_channel_plan(
    agent_home: Path, config: dict[str, Any], settings: dict[str, str]
) -> tuple[tuple[str, str | None] | None, bool, bool]:
    metadata, metadata_corrupt = _metadata_home_channel(agent_home)
    configured, config_corrupt = _config_home_channel(config)
    env_room = settings.get("FINITE_CONFIG_HOME_CHANNEL", "").strip()
    if env_room:
        conversation_id = metadata[1] if metadata and metadata[0] == env_room else None
        desired = (env_room, conversation_id)
    elif metadata is not None:
        desired = metadata
    else:
        desired = configured
    if desired is None and (metadata_corrupt or config_corrupt):
        raise RecoveryRefusal("home_channel_metadata_corrupt")
    if desired is not None:
        settings["FINITE_CONFIG_HOME_CHANNEL"] = desired[0]
    return desired, metadata != desired, metadata_corrupt or config_corrupt


def _apply_home_channel_plan(
    agent_home: Path,
    finitechat_bin: Path,
    desired: tuple[str, str | None] | None,
    metadata_changed: bool,
    corrupt: bool,
) -> dict[str, Any]:
    if desired is None:
        return {"action": "home_channel_reconcile", "status": "not_configured"}
    if metadata_changed or corrupt:
        room_id, conversation_id = desired
        command = [
            str(finitechat_bin),
            "hermes",
            "--home",
            str(agent_home),
            "home-channel",
            "set",
            "--room-id",
            room_id,
        ]
        if conversation_id:
            command.extend(["--conversation-id", conversation_id])
        _run_json_command(command, "home_channel_reconcile_failed")
        return {"action": "home_channel_reconcile", "status": "changed"}
    return {"action": "home_channel_reconcile", "status": "preserved"}


def _interrupted_turn_count(path: Path) -> int:
    if not os.path.lexists(path):
        return 0
    value = _read_json_object(path, "interrupted_turn_state_corrupt")
    messages = value.get("messages", [])
    if not isinstance(messages, list) or any(not isinstance(message, dict) for message in messages):
        raise RecoveryRefusal("interrupted_turn_state_corrupt")
    return len(messages)


def _recover_interrupted_turns(
    agent_home: Path, finitechat_bin: Path, expected_count: int
) -> dict[str, Any]:
    if expected_count == 0:
        return {
            "action": "interrupted_turn_recovery",
            "status": "not_needed",
            "count": 0,
        }
    value = _run_json_command(
        [
            str(finitechat_bin),
            "hermes",
            "--home",
            str(agent_home),
            "recover",
            "--json",
        ],
        "interrupted_turn_recovery_failed",
    )
    recovered = value.get("recovered")
    if not isinstance(recovered, int) or isinstance(recovered, bool) or recovered != expected_count:
        raise RecoveryRefusal("interrupted_turn_recovery_failed")
    remaining = _interrupted_turn_count(agent_home / "hermes-running.json")
    if remaining != 0:
        raise RecoveryRefusal("interrupted_turn_recovery_failed")
    return {
        "action": "interrupted_turn_recovery",
        "status": "completed",
        "count": recovered,
    }


def _failpoint(name: str) -> None:
    configured = os.environ.get(RECOVERY_FAILPOINT_ENV, "")
    if configured and os.environ.get(RECOVERY_TESTING_ENV) != "1":
        raise RecoveryRefusal("test_override_forbidden")
    if configured == name:
        raise InjectedRecoveryCrash(name)


def _startup_report(
    operation_hash: str | None,
    *,
    status: str,
    phase: str,
    resumed: bool = False,
) -> dict[str, Any]:
    return {
        "schema_version": STARTUP_REPORT_SCHEMA_VERSION,
        "report_kind": "finite_agent_startup",
        "boot_mode": RECOVERY_KIND,
        "status": status,
        "phase": phase,
        "generated_at_unix": int(time.time()),
        "operation_id_hash": operation_hash,
        "idempotency": {
            "same_operation_replay": "no_op_after_terminal_state",
            "resumed_after_interruption": resumed,
        },
        "actions": [],
        "refusals": [],
        "warnings": [],
        "preservation": {
            "identity": "preserve",
            "client_store": "reuse_in_place",
            "hermes_memory": "preserve",
            "workspace": "preserve",
            "user_platforms_model": "preserve",
            "user_tools_connections_skills": "preserve",
            "mutation_allowlist": [
                "canonical_finitechat_plugin",
                "generated_finitechat_config",
                "home_channel_metadata",
                "interrupted_hermes_turn_completion",
                "explicit_transient_runtime_files",
                "recovery_journal_and_startup_report",
            ],
        },
        "acceptance_scope": {
            "runtime_spec_delivery": "not_proven",
            "provider_conformance": "not_proven",
            "phala_acceptance": "not_proven",
        },
    }


def _root_facts(
    state_root: Path, agent_home: Path, hermes_home: Path, workspace: Path
) -> dict[str, Any]:
    return {
        label: {
            "present": path.is_dir(),
            "writable": path.is_dir() and os.access(path, os.W_OK),
        }
        for label, path in (
            ("/data", state_root),
            ("/data/agent", agent_home),
            ("/data/agent/hermes-home", hermes_home),
            ("/data/workspace", workspace),
        )
    }


def _write_progress(
    startup_path: Path, operation_path: Path | None, report: dict[str, Any]
) -> None:
    _atomic_json(startup_path, report)
    if operation_path is not None:
        _atomic_json(operation_path, report)


def _load_operation(path: Path, operation_hash: str) -> dict[str, Any] | None:
    if not os.path.lexists(path):
        return None
    value = _read_json_object(path, "operation_journal_corrupt")
    if (
        value.get("schema_version") != STARTUP_REPORT_SCHEMA_VERSION
        or value.get("report_kind") != "finite_agent_startup"
        or value.get("boot_mode") != RECOVERY_KIND
        or value.get("operation_id_hash") != operation_hash
        or value.get("status") not in {"running", "completed", "refused"}
    ):
        raise RecoveryRefusal("operation_journal_corrupt")
    return value


def _validate_completed_projection(startup_path: Path, completed_operation: dict[str, Any]) -> None:
    startup = _read_json_object(startup_path, "startup_report_missing_or_corrupt")
    if startup != completed_operation:
        raise RecoveryRefusal("startup_report_terminal_mismatch")


def _open_operation_lock(path: Path) -> int:
    if os.path.lexists(path):
        _require_regular_file(path, "operation_journal_unsafe")
    flags = os.O_CREAT | os.O_RDWR
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        return os.open(path, flags, 0o600)
    except OSError as exc:
        raise RecoveryRefusal("operation_journal_unsafe") from exc


def _write_refusal_report(
    agent_home: Path,
    operation_hash: str | None,
    refusal: RecoveryRefusal,
    *,
    phase: str,
) -> dict[str, Any]:
    report = _startup_report(operation_hash, status="refused", phase=phase)
    report["error_code"] = refusal.code
    report["refusals"] = [{"code": refusal.code, "remediation": refusal.remediation}]
    try:
        agent_home_mode = agent_home.lstat().st_mode
    except OSError:
        return report
    if stat.S_ISLNK(agent_home_mode) or not stat.S_ISDIR(agent_home_mode):
        return report
    with contextlib.suppress(OSError, RecoveryRefusal):
        _atomic_json(agent_home / "startup-report.json", report)
    return report


def recover_known_good_boot(config_path: Path) -> int:
    agent_home = Path(os.environ.get("FINITECHAT_HOME", "/data/agent"))
    hermes_home = Path(os.environ.get("HERMES_HOME", str(agent_home / "hermes-home")))
    finite_home = Path(os.environ.get("FINITE_HOME", str(agent_home)))
    workspace = Path(os.environ.get("FINITECHAT_WORKSPACE", "/data/workspace"))
    state_root = Path(os.environ.get("FINITE_AGENT_STATE_ROOT", "/data"))
    finitechat_bin = Path(os.environ.get("FINITECHAT_BIN", "/usr/local/bin/finitechat"))
    startup_path = agent_home / "startup-report.json"

    try:
        operation_hash = _parse_recovery_intent()
    except RecoveryRefusal as refusal:
        _write_refusal_report(agent_home, None, refusal, phase="intent_validation")
        print(f"FINITE_RECOVER_CHAT_REFUSED code={refusal.code}", file=sys.stderr)
        return 65

    try:
        _require_directory(agent_home, "agent_home_missing_or_unsafe")
        operations = agent_home / "recover-chat-operations"
        if os.path.lexists(operations):
            _require_directory(operations, "operation_journal_unsafe")
        else:
            operations.mkdir(mode=0o700)
        lock_descriptor = _open_operation_lock(agent_home / ".recover-chat.lock")
    except (OSError, RecoveryRefusal) as exc:
        code = exc.code if isinstance(exc, RecoveryRefusal) else "operation_journal_unsafe"
        _write_refusal_report(
            agent_home,
            operation_hash,
            RecoveryRefusal(code),
            phase="blocked",
        )
        print(f"FINITE_RECOVER_CHAT_REFUSED code={code}", file=sys.stderr)
        return 65

    operation_path = operations / f"{operation_hash.removeprefix('sha256:')}.json"
    with os.fdopen(lock_descriptor, "a", encoding="utf-8") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        try:
            prior = _load_operation(operation_path, operation_hash)
        except RecoveryRefusal as refusal:
            _write_refusal_report(agent_home, operation_hash, refusal, phase="blocked")
            print(f"FINITE_RECOVER_CHAT_REFUSED code={refusal.code}", file=sys.stderr)
            return 65
        if prior and prior.get("status") == "completed":
            try:
                _validate_completed_projection(startup_path, prior)
            except RecoveryRefusal as refusal:
                print(
                    f"FINITE_RECOVER_CHAT_REFUSED code={refusal.code}",
                    file=sys.stderr,
                )
                return 65
            print(f"FINITE_RECOVER_CHAT_NOOP operation={operation_hash}")
            return 0
        if prior and prior.get("status") == "refused":
            code = prior.get("error_code")
            safe_code = code if isinstance(code, str) else "prior_recovery_refused"
            print(f"FINITE_RECOVER_CHAT_REFUSED code={safe_code}", file=sys.stderr)
            return 65

        resumed = bool(prior and prior.get("status") == "running")
        report = _startup_report(
            operation_hash, status="running", phase="preflight", resumed=resumed
        )
        try:
            _write_progress(startup_path, operation_path, report)
        except OSError:
            refusal = RecoveryRefusal("startup_report_unwritable")
            report = _write_refusal_report(agent_home, operation_hash, refusal, phase="blocked")
            with contextlib.suppress(OSError):
                _atomic_json(operation_path, report)
            print(f"FINITE_RECOVER_CHAT_REFUSED code={refusal.code}", file=sys.stderr)
            return 65
        try:
            _failpoint("after_started_report")
            for path, code in (
                (state_root, "state_root_missing_or_unsafe"),
                (agent_home, "agent_home_missing_or_unsafe"),
                (hermes_home, "hermes_home_missing_or_unsafe"),
                (workspace, "workspace_missing_or_unsafe"),
            ):
                _require_directory(path, code)
            try:
                agent_home.resolve().relative_to(state_root.resolve())
                hermes_home.resolve().relative_to(agent_home.resolve())
                workspace.resolve().relative_to(state_root.resolve())
            except (OSError, ValueError) as exc:
                raise RecoveryRefusal("state_root_contract_mismatch") from exc
            if finite_home != agent_home:
                raise RecoveryRefusal("identity_root_contract_mismatch")
            if config_path != hermes_home / "config.yaml":
                raise RecoveryRefusal("generated_config_path_mismatch")
            _require_regular_file(finitechat_bin, "finitechat_binary_missing_or_unsafe")
            _require_regular_file(config_path, "generated_config_missing_or_corrupt")

            agent_config = _validate_agent_config(agent_home)
            agent_config_before = dict(agent_config)
            npub, identity_file_key = _validate_identity(
                finite_home, finitechat_bin, agent_config["account_id"]
            )
            client_store = agent_home / "client.sqlite3"
            client_store_key = _validate_client_store(client_store)
            try:
                existing_config = _load(config_path)
            except (ConfigError, OSError, UnicodeError) as exc:
                raise RecoveryRefusal("generated_config_missing_or_corrupt") from exc
            settings = dict(os.environ)
            _validate_reconcile_settings(settings, agent_home, finitechat_bin)
            desired_home_channel, home_channel_changed, home_channel_corrupt = _home_channel_plan(
                agent_home, existing_config, settings
            )
            try:
                reconcile_config(existing_config, settings, recover_known_good=True)
            except (ConfigError, KeyError, ValueError) as exc:
                raise RecoveryRefusal("generated_config_missing_or_corrupt") from exc
            interrupted_turn_count = _interrupted_turn_count(agent_home / "hermes-running.json")
            _validate_plugin_destination(hermes_home)
            _validate_transient_allowlist(agent_home, hermes_home)

            report["identity"] = {"present": True, "npub": npub}
            report["state_roots"] = _root_facts(state_root, agent_home, hermes_home, workspace)
            report["phase"] = "repair"
            _write_progress(startup_path, operation_path, report)

            removed = _clear_transient_allowlist(agent_home, hermes_home)
            report["actions"].append(
                {
                    "action": "transient_cleanup",
                    "status": "completed",
                    "count": len(removed),
                    "items": removed,
                }
            )
            _write_progress(startup_path, operation_path, report)
            _failpoint("after_transient_cleanup")

            plugin_path = hermes_home / "plugins" / CANONICAL_PLUGIN_NAME
            plugin_before = _plugin_digest(plugin_path)
            if plugin_path.exists():
                try:
                    if plugin_path.is_dir():
                        shutil.rmtree(plugin_path)
                    else:
                        plugin_path.unlink()
                except OSError as exc:
                    raise RecoveryRefusal("plugin_reinstall_failed") from exc
            try:
                plugin_path.parent.mkdir(mode=0o700, exist_ok=True)
            except OSError as exc:
                raise RecoveryRefusal("plugin_reinstall_failed") from exc
            installed = _run_json_command(
                [
                    str(finitechat_bin),
                    "hermes",
                    "--home",
                    str(agent_home),
                    "install",
                    "--plugins-dir",
                    str(plugin_path.parent),
                    "--plugin-name",
                    CANONICAL_PLUGIN_NAME,
                    "--finitechat-bin",
                    str(finitechat_bin),
                    "--force",
                    "--json",
                ],
                "plugin_reinstall_failed",
            )
            if installed.get("plugin_name") != CANONICAL_PLUGIN_NAME:
                raise RecoveryRefusal("plugin_reinstall_failed")
            _validate_plugin_destination(hermes_home)
            plugin_after = _plugin_digest(plugin_path)
            if plugin_after is None:
                raise RecoveryRefusal("plugin_reinstall_failed")
            report["actions"].append(
                {
                    "action": "canonical_plugin_reinstall",
                    "status": "changed" if plugin_before != plugin_after else "verified",
                    "digest_before": plugin_before,
                    "digest_after": plugin_after,
                }
            )
            _write_progress(startup_path, operation_path, report)
            _failpoint("after_plugin_reinstall")

            report["actions"].append(
                _apply_home_channel_plan(
                    agent_home,
                    finitechat_bin,
                    desired_home_channel,
                    home_channel_changed,
                    home_channel_corrupt,
                )
            )
            reconciled = reconcile_config(existing_config, settings, recover_known_good=True)
            config_changed = reconciled != existing_config
            if config_changed:
                _atomic_write(config_path, _dump(reconciled))
            report["actions"].append(
                {
                    "action": "generated_finitechat_config_reconcile",
                    "status": "changed" if config_changed else "verified",
                }
            )
            _write_progress(startup_path, operation_path, report)
            _failpoint("after_config_reconcile")

            report["actions"].append(
                _recover_interrupted_turns(agent_home, finitechat_bin, interrupted_turn_count)
            )
            _write_progress(startup_path, operation_path, report)
            _failpoint("after_turn_recovery")

            if _validate_client_store(client_store) != client_store_key:
                raise RecoveryRefusal("client_store_replaced_during_recovery")
            final_config = _validate_agent_config(agent_home)
            if final_config != agent_config_before:
                raise RecoveryRefusal("agent_config_changed_during_recovery")
            final_npub, final_identity_file_key = _validate_identity(
                finite_home, finitechat_bin, agent_config["account_id"]
            )
            if final_npub != npub or final_identity_file_key != identity_file_key:
                raise RecoveryRefusal("identity_changed_during_recovery")
            report["preservation"].update(
                {
                    "agent_config_preserved": True,
                    "identity_reused_in_place": True,
                    "client_store_reused_in_place": True,
                }
            )
            report["status"] = "completed"
            report["phase"] = "complete"
            report["completed_at_unix"] = int(time.time())
            _atomic_json(startup_path, report)
            _failpoint("before_complete_marker")
            _atomic_json(operation_path, report)
            print(f"FINITE_RECOVER_CHAT_COMPLETE operation={operation_hash}")
            return 0
        except InjectedRecoveryCrash as crash:
            print(f"FINITE_RECOVER_CHAT_TEST_CRASH phase={crash}", file=sys.stderr)
            return 75
        except Exception as exc:
            refusal = (
                exc
                if isinstance(exc, RecoveryRefusal)
                else RecoveryRefusal("internal_recovery_error")
            )
            report["status"] = "refused"
            report["phase"] = "blocked"
            report["error_code"] = refusal.code
            report["refusals"] = [{"code": refusal.code, "remediation": refusal.remediation}]
            with contextlib.suppress(OSError):
                _write_progress(startup_path, operation_path, report)
            print(f"FINITE_RECOVER_CHAT_REFUSED code={refusal.code}", file=sys.stderr)
            return 65


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", type=Path, required=True)
    args = parser.parse_args()
    raise SystemExit(recover_known_good_boot(args.config))


if __name__ == "__main__":
    main()
