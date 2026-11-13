"""Bounded, load-only hosted identity census for exact operator-supplied keys.

Never emits identity-file contents, secret fields or message bodies. SQLite
queries run only on private scratch copies supplied by finite-status. Filesystem
observations are bounded evidence, not an atomic snapshot or a control proof.
"""
from __future__ import annotations

import json
import os
import re
import shutil
import stat
import tempfile
import time
from pathlib import Path
from typing import Any, Callable

MAX_ROOTS = 2000
MAX_KEYS = 512
MAX_IDENTITY_BYTES = 64 * 1024
MAX_STORE_BYTES = 1 << 30
MAX_TOTAL_BYTES = 16 << 30
BUDGET_SECONDS = 600
HEX = re.compile(r"^[0-9a-f]{64}$")


class CensusError(Exception):
    pass


def fingerprint(info: os.stat_result) -> tuple[int, ...]:
    return info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns


def public_selector(path: Path) -> tuple[str, tuple[int, ...]]:
    """Parse a bounded file locally, returning only its public selector/stat.

    The JSON may contain secrets; never include parse exceptions or file bytes
    in output. No cryptographic derivation, identity loading or creation occurs.
    """
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd, "rb") as stream:
        before = os.fstat(stream.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_size > MAX_IDENTITY_BYTES:
            raise CensusError("identity file exceeds bound or is not regular")
        data = stream.read(MAX_IDENTITY_BYTES + 1)
        if len(data) > MAX_IDENTITY_BYTES or fingerprint(before) != fingerprint(os.fstat(stream.fileno())):
            raise CensusError("identity file changed or exceeds bound")
    try:
        value = json.loads(data)
        key = value.get("public_key_hex") if isinstance(value, dict) else None
    except (ValueError, UnicodeError):
        raise CensusError("identity JSON invalid") from None
    if not isinstance(key, str) or not HEX.fullmatch(key):
        raise CensusError("public selector invalid")
    if fingerprint(path.lstat()) != fingerprint(before):
        raise CensusError("identity file replaced during observation")
    return key, fingerprint(before)


def collect(
    root: Path,
    target_keys: list[str],
    accounts: list[dict[str, Any]],
    scratch_copy: Callable[[Path], Any],
    query: Callable[[Path, str], list[dict[str, Any]]],
    clock: Callable[[], float] = time.monotonic,
    free_bytes: Callable[[], int] | None = None,
) -> dict[str, Any]:
    if (not target_keys or len(target_keys) > MAX_KEYS or len(set(target_keys)) != len(target_keys)
            or any(not isinstance(k, str) or not HEX.fullmatch(k) for k in target_keys)):
        raise CensusError("requires 1-512 distinct canonical public keys")
    if len(accounts) > MAX_ROOTS:
        raise CensusError("Core accounts exceeded bound")
    by_storage: dict[str, list[dict[str, Any]]] = {}
    for account in accounts:
        storage = account.get("storage")
        if not isinstance(storage, str) or not HEX.fullmatch(storage):
            raise CensusError("invalid Core storage id")
        by_storage.setdefault(storage, []).append(account)
    users = root / "users"
    if users.is_symlink() or root.is_symlink():
        raise CensusError("hosted root must not be a symlink")

    def inventory() -> dict[str, tuple[int, int]]:
        entries = {}
        with os.scandir(users) as listing:
            for entry in listing:
                if len(entries) >= MAX_ROOTS:
                    raise CensusError("hosted root census exceeded bound")
                info = entry.stat(follow_symlinks=False)
                if not HEX.fullmatch(entry.name) or not stat.S_ISDIR(info.st_mode):
                    raise CensusError("unexpected or symlinked hosted root")
                entries[entry.name] = (info.st_dev, info.st_ino)
        return entries

    initial = inventory()
    targets = set(target_keys)
    started, charged = clock(), 0
    if free_bytes is None:
        free_bytes = lambda: shutil.disk_usage(tempfile.gettempdir()).free
    results = []
    sql = ("SELECT account_id, device_id FROM client_device_states"
           " WHERE device_id = 'hosted-web' ORDER BY account_id LIMIT 513;")
    for storage in sorted(initial):
        entry: dict[str, Any] = {"storage": storage, "state": "unavailable", "matches": []}
        results.append(entry)
        if clock() - started > BUDGET_SECONDS:
            entry["reason"] = "census time budget exhausted"
            continue
        home = users / storage
        identity = home / "finite-home/identity/identity.json"
        store = home / "chat/client.sqlite3"
        try:
            for parent in (home / "finite-home", identity.parent, store.parent):
                if parent.is_symlink():
                    raise CensusError("symlinked identity/store parent")
            present = identity.exists(), store.exists()
            if identity.is_symlink() or store.is_symlink():
                raise CensusError("symlinked identity/store")
            if present == (False, False):
                entry["state"] = "emptyRoot"
                continue
            if present != (True, True):
                raise CensusError("identity/store counterpart missing")
            before_key, before_stat = public_selector(identity)
            size = 0
            for suffix in ("", "-wal", "-shm"):
                part = Path(str(store) + suffix)
                if not part.exists():
                    continue
                info = part.lstat()
                if not stat.S_ISREG(info.st_mode):
                    raise CensusError("store or sidecar is not regular")
                size += info.st_size
            if size > MAX_STORE_BYTES or charged + size > MAX_TOTAL_BYTES:
                raise CensusError("store/census byte budget exceeded")
            if free_bytes() < size + (64 << 20):
                raise CensusError("insufficient scratch space")
            charged += size
            with scratch_copy(store) as scratch:
                rows = query(scratch, sql)
                scratch_parent = scratch.parent
            if scratch_parent.exists():
                raise CensusError("scratch cleanup incomplete")
            after_key, after_stat = public_selector(identity)
            if before_key != after_key or before_stat != after_stat:
                raise CensusError("identity changed across store observation")
            if (len(rows) > MAX_KEYS or any(
                    not isinstance(r.get("account_id"), str) or not HEX.fullmatch(r["account_id"])
                    or r.get("device_id") != "hosted-web" for r in rows)):
                raise CensusError("hosted device-state keys invalid or over bound")
            stored_keys = {r["account_id"] for r in rows}
            owners = by_storage.get(storage, [])
            unique_owner = owners[0] if len(owners) == 1 else None
            consistent = stored_keys == {before_key}
            entry.update(state="checked", sourceBytes=size, hostedKeyCount=len(stored_keys),
                         selectorMatchesStore=consistent, coreAccountCount=len(owners))
            for key in sorted(targets & (stored_keys | {before_key})):
                entry["matches"].append({
                    "publicKeyHex": key, "currentSelector": key == before_key,
                    "hostedStore": key in stored_keys,
                    "account": None if unique_owner is None else {
                        field: unique_owner.get(field) for field in ("account", "email", "link_status")},
                    "currentAccountCandidate": bool(consistent and key == before_key
                        and unique_owner and unique_owner.get("link_status") == "linked"),
                })
        except Exception as error:  # noqa: BLE001 - retain per-root coverage, redact external errors
            # Never export exception text from parsers, OS calls or SQLite.
            entry["reason"] = str(error) if isinstance(error, CensusError) else type(error).__name__
    unchanged = inventory() == initial
    claims: dict[str, list[str]] = {}
    for entry in results:
        for match in entry["matches"]:
            claims.setdefault(match["publicKeyHex"], []).append(entry["storage"])
    complete = unchanged and all(r["state"] in ("checked", "emptyRoot") for r in results)
    return {
        "schema_version": "finite.hosted-identity-census.v1", "exit_code": 0 if complete else 2,
        "coverage": {"complete": complete, "rootSetUnchanged": unchanged,
                     "rootCount": len(initial), "targetKeyCount": len(targets),
                     "checkedRoots": sum(r["state"] == "checked" for r in results),
                     "chargedSourceBytes": charged},
        "roots": results,
        "duplicateClaims": {k: v for k, v in claims.items() if len(v) > 1},
        "limitations": [
            "One configured hosted root; per-file observations are not an atomic filesystem/Core snapshot.",
            "Stored public selector only; no secret derivation or cryptographic control proof.",
            "SQLite contents are copied privately; only hosted-web public key columns are queried.",
            "Byte/time scheduling guards apply before copies; an in-progress copy can exceed them.",
            "A currentAccountCandidate is usable only with complete coverage and no duplicate key claim.",
        ],
    }
