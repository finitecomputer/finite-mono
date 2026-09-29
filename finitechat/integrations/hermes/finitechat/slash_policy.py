"""Finite Chat slash-command policy for the Hermes adapter.

``slash_policy.json`` tiers every gateway command in the pinned Hermes
registry. The adapter refuses ``restricted`` commands before Hermes sees them,
and fails closed on a gateway command the file does not tier, so a Hermes pin
bump cannot quietly expose a new command. Anything that is not a gateway
command (skills, plugin commands, confirm replies such as ``/always``) passes
through unchanged.

Parsing reuses Hermes itself: plaintext coercion ("restart hermes"),
``MessageEvent.get_command``, the owner's ``quick_commands`` aliases, and
``resolve_command``, so case, aliases, and ``@bot`` suffixes resolve exactly
as the gateway resolves them. If a helper is missing on some Hermes version,
or the policy file does not load, the check degrades to allowing and logs
once; it never raises into message dispatch. ``self_check`` makes that state
visible where it must not ship (the runtime image validation).
"""

from __future__ import annotations

import copy
import functools
import json
import logging
import sys
from pathlib import Path
from typing import Any, NamedTuple

logger = logging.getLogger(__name__)

POLICY_PATH = Path(__file__).with_name("slash_policy.json")
TIERS = frozenset({"suggested", "available", "not_recommended", "unlisted", "restricted"})
RESTRICTED_TIER = "restricted"
GENERIC_REASON = "This command isn't available in Finite chat."
# Interrupts and approval replies must always reach Hermes. A policy file that
# would refuse them is treated as unreadable rather than trusted.
ESCAPE_HATCHES = frozenset({"stop", "new", "approve", "deny"})
# Hermes lifts a pause on any of these (GatewayRunner._handle_pause_command).
PAUSE_RESUME_ARGS = frozenset({"off", "resume", "stop", "disengage"})
# /loop creation is always refused: a loop prompt that starts with "/" is
# re-dispatched internally. These dispatch_loop_command verbs create and
# resume nothing, so a loop made before this policy can still be turned off.
LOOP_CONTROL_ARGS = frozenset({"", "status", "pause", "stop", "clear", "cancel"})
# Hermes expands a quick-command alias before built-in dispatch and again in
# its quick-command block, so a chain can resolve twice; follow a few hops.
MAX_ALIAS_HOPS = 4

_warned: set[str] = set()


class Refusal(NamedTuple):
    command: str
    text: str


def evaluate(event: Any, quick_commands: dict[str, Any] | None = None) -> Refusal | None:
    """Return a refusal for a restricted gateway command, otherwise None.

    ``quick_commands`` defaults to the live gateway runner's configuration,
    the same mapping Hermes expands aliases from.
    """
    try:
        return _evaluate(event, quick_commands)
    except Exception:
        _warn_once("evaluate", "slash policy check failed; allowing the message", exc_info=True)
        return None


def _evaluate(event: Any, quick_commands: dict[str, Any] | None) -> Refusal | None:
    candidate = _coerced(event)
    if not candidate.get_command():
        return None
    registry = _registry()
    if registry is None:
        return None
    resolve_command, gateway_commands = registry
    candidate = _expand_quick_alias(candidate, resolve_command, quick_commands)
    command = candidate.get_command()
    definition = resolve_command(command) if command else None
    if definition is None or definition.name not in gateway_commands:
        return None
    name = definition.name
    args = candidate.get_command_args().strip().lower()
    if name == "pause" and args in PAUSE_RESUME_ARGS:
        return None
    if name == "loop":
        if args in LOOP_CONTROL_ARGS:
            return None
        return _refusal(name, _policy())
    policy = _policy()
    if policy is None:
        return None
    entry = policy.get(name)
    if entry is not None and entry["tier"] != RESTRICTED_TIER:
        return None
    return _refusal(name, policy)


def _refusal(name: str, policy: dict[str, dict[str, str]] | None) -> Refusal:
    reason = ((policy or {}).get(name) or {}).get("reason") or GENERIC_REASON
    header = f"/{name} isn't available in Finite chat."
    return Refusal(name, header if reason == GENERIC_REASON else f"{header} {reason}")


def _coerced(event: Any) -> Any:
    """Apply Hermes's DM plaintext rewrite to a copy, leaving the event as sent."""
    if not getattr(event, "allow_gateway_control", True):
        return event
    try:
        from gateway.platforms.base import coerce_plaintext_gateway_command
    except ImportError:
        coerce_plaintext_gateway_command = None
    if not callable(coerce_plaintext_gateway_command):
        _warn_once("coerce", "Hermes plaintext command coercion is unavailable")
        return event
    candidate = copy.copy(event)
    coerce_plaintext_gateway_command(candidate)
    return candidate


def _expand_quick_alias(
    candidate: Any, resolve_command: Any, quick_commands: dict[str, Any] | None
) -> Any:
    """Rewrite an alias to its target the way GatewayRunner._handle_message does.

    Registry commands always win over a quick command of the same name, and
    only ``type: alias`` entries rewrite; exec entries and unknown names pass.
    """
    aliases = _runner_quick_commands() if quick_commands is None else quick_commands
    for _ in range(MAX_ALIAS_HOPS):
        command = candidate.get_command()
        if not command or resolve_command(command) is not None:
            return candidate
        entry = aliases.get(command) if isinstance(aliases, dict) else None
        if not isinstance(entry, dict) or entry.get("type") != "alias":
            return candidate
        target = str(entry.get("target") or "").strip()
        if not target:
            return candidate
        # Hermes strips every leading slash when dispatching alias targets,
        # unlike MessageEvent.get_command(), which removes only one.
        target = f"/{target.lstrip('/')}"
        user_args = candidate.get_command_args().strip()
        candidate = copy.copy(candidate)
        candidate.text = f"{target} {user_args}".strip()
    return candidate


def _runner_quick_commands() -> dict[str, Any]:
    # Only a running gateway has aliases; never import gateway.run for them.
    run_module = sys.modules.get("gateway.run")
    runner_ref = getattr(run_module, "_gateway_runner_ref", None)
    runner = runner_ref() if callable(runner_ref) else None
    config = getattr(runner, "config", None)
    if isinstance(config, dict):
        quick_commands = config.get("quick_commands")
    else:
        quick_commands = getattr(config, "quick_commands", None)
    return quick_commands if isinstance(quick_commands, dict) else {}


def _registry() -> tuple[Any, frozenset[str]] | None:
    try:
        from hermes_cli.commands import GATEWAY_KNOWN_COMMANDS, resolve_command
    except ImportError:
        resolve_command = None
        GATEWAY_KNOWN_COMMANDS = frozenset()
    if not callable(resolve_command):
        _warn_once("registry", "Hermes command registry is unavailable; allowing commands")
        return None
    return resolve_command, frozenset(GATEWAY_KNOWN_COMMANDS)


@functools.cache
def _policy() -> dict[str, dict[str, str]] | None:
    try:
        data = json.loads(POLICY_PATH.read_text(encoding="utf-8"))
        _validate(data)
    except (OSError, ValueError) as exc:
        logger.error(
            "[finitechat] slash policy %s did not load; allowing commands: %s", POLICY_PATH, exc
        )
        return None
    return data


def _validate(data: Any) -> None:
    if not isinstance(data, dict) or not data:
        raise ValueError("expected a non-empty command name -> {tier, reason?} object")
    for name, entry in data.items():
        if (
            not isinstance(entry, dict)
            or not set(entry) <= {"tier", "reason"}
            or entry.get("tier") not in TIERS
            or not isinstance(entry.get("reason", ""), str)
        ):
            raise ValueError(f"malformed entry for /{name}")
    for name in sorted(ESCAPE_HATCHES):
        if data.get(name, {}).get("tier") in (None, RESTRICTED_TIER):
            raise ValueError(f"/{name} must be tiered and never restricted")


def self_check() -> str:
    """Raise unless the policy is loaded and refuses what it must.

    Run by the runtime image validation so an image cannot ship with the
    policy silently off. Returns a one-line summary.
    """
    policy = _policy()
    if policy is None:
        raise RuntimeError(f"slash policy {POLICY_PATH} did not load")
    registry = _registry()
    if registry is None:
        raise RuntimeError("Hermes command registry is unavailable")
    resolve_command, gateway_commands = registry
    tiered = {getattr(resolve_command(name), "name", name) for name in gateway_commands}
    untiered = sorted(tiered - set(policy))
    if untiered:
        raise RuntimeError(f"untiered gateway commands: {untiered}")

    from gateway.platforms.base import MessageEvent, Platform
    from gateway.session import SessionSource

    source = SessionSource(platform=Platform.LOCAL, chat_id="self-check", chat_type="dm")
    for text, refused in (
        ("/update", True),
        ("restart hermes", True),
        ("/loop 5m /status", True),
        ("/stop", False),
        ("/loop stop", False),
        ("hello", False),
    ):
        refusal = evaluate(MessageEvent(text=text, source=source), quick_commands={})
        if (refusal is not None) != refused:
            raise RuntimeError(f"slash policy {'allowed' if refused else 'refused'} {text!r}")
    restricted = sum(entry["tier"] == RESTRICTED_TIER for entry in policy.values())
    return f"slash policy loaded: {len(policy)} commands, {restricted} restricted"


def _warn_once(key: str, message: str, *, exc_info: bool = False) -> None:
    if key in _warned:
        return
    _warned.add(key)
    logger.warning("[finitechat] %s", message, exc_info=exc_info)
