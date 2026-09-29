"""Finite Chat slash-command policy for the Hermes adapter.

``slash_policy.json`` tiers every gateway command in the pinned Hermes
registry. The adapter refuses ``restricted`` commands before Hermes sees them,
and fails closed on a gateway command the file does not tier, so a Hermes pin
bump cannot quietly expose a new command. Anything that is not a gateway
command (skills, plugin commands, confirm replies such as ``/always``) passes
through unchanged.

Parsing reuses Hermes itself: plaintext coercion ("restart hermes"),
``MessageEvent.get_command`` and ``resolve_command``, so case, aliases, and
``@bot`` suffixes resolve exactly as the gateway resolves them. If a helper is
missing on some Hermes version the check degrades to allowing and logs once;
it never raises into message dispatch.
"""

from __future__ import annotations

import copy
import functools
import json
import logging
from pathlib import Path
from typing import Any, NamedTuple

logger = logging.getLogger(__name__)

POLICY_PATH = Path(__file__).with_name("slash_policy.json")
RESTRICTED_TIER = "restricted"
GENERIC_REASON = "This command isn't available in Finite chat."
# A /loop prompt that starts with "/" is re-dispatched as an internal command
# that never passes this check, so /loop is refused whatever its tier says.
ALWAYS_REFUSED = frozenset({"loop"})

_warned: set[str] = set()


class Refusal(NamedTuple):
    command: str
    text: str


def evaluate(event: Any) -> Refusal | None:
    """Return a refusal for a restricted gateway command, otherwise None."""
    try:
        return _evaluate(event)
    except Exception:
        _warn_once("evaluate", "slash policy check failed; allowing the message", exc_info=True)
        return None


def _evaluate(event: Any) -> Refusal | None:
    candidate = _coerced(event)
    command = candidate.get_command()
    if not command:
        return None
    registry = _registry()
    if registry is None:
        return None
    resolve_command, gateway_commands = registry
    definition = resolve_command(command)
    if definition is None or definition.name not in gateway_commands:
        return None
    name = definition.name
    if name == "pause" and candidate.get_command_args().strip().lower() == "off":
        return None
    policy = _policy()
    if policy is None:
        return None
    entry = policy.get(name)
    if name not in ALWAYS_REFUSED and entry is not None and entry["tier"] != RESTRICTED_TIER:
        return None
    reason = (entry or {}).get("reason") or GENERIC_REASON
    return Refusal(name, _refusal_text(name, reason))


def _refusal_text(name: str, reason: str) -> str:
    header = f"/{name} isn't available in Finite chat."
    return header if reason == GENERIC_REASON else f"{header} {reason}"


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
        if not isinstance(data, dict) or not all(
            isinstance(entry, dict) and isinstance(entry.get("tier"), str)
            for entry in data.values()
        ):
            raise ValueError("expected a command name -> {tier, reason?} object")
    except (OSError, ValueError) as exc:
        logger.error(
            "[finitechat] slash policy %s is unreadable; allowing commands: %s", POLICY_PATH, exc
        )
        return None
    return data


def _warn_once(key: str, message: str, *, exc_info: bool = False) -> None:
    if key in _warned:
        return
    _warned.add(key)
    logger.warning("[finitechat] %s", message, exc_info=exc_info)
