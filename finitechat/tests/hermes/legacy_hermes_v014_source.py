#!/usr/bin/env python3
"""Create and export a real Hermes v0.14.0 source bundle for compatibility tests."""

from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import importlib.util
import json
import shutil
import sys
import tarfile
from pathlib import Path
from typing import Any, cast


def _load_migration(path: Path):
    sys.path.insert(0, str(path.parent))
    spec = importlib.util.spec_from_file_location("legacy_v014_migration", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--migration-tool", type=Path, required=True)
    args = parser.parse_args()

    actual_version = importlib.metadata.version("hermes-agent")
    if actual_version != "0.14.0":
        raise SystemExit(f"legacy fixture requires Hermes 0.14.0, got {actual_version}")

    migration = _load_migration(args.migration_tool)
    from hermes_state import SessionDB
    from plugins.memory.holographic.store import MemoryStore

    root = args.root
    payload = root / "bundle/payload"
    payload.mkdir(parents=True)

    source_state = root / "source-state.db"
    sessions = SessionDB(db_path=source_state)
    try:
        sessions.create_session(
            "legacy-telegram-session",
            "telegram",
            user_id="legacy-user",
        )
        sessions.append_message(
            "legacy-telegram-session",
            "user",
            # v0.14 accepts structured JSON parts; v0.20 narrowed this annotation to str.
            cast(
                Any,
                [
                    {"type": "text", "text": "Remember the owner"},
                    {"type": "file", "path": "/home/node/uploads/photo.jpg"},
                    {
                        "type": "audio",
                        "path": "/home/node/.hermes/audio_cache/voice.ogg",
                    },
                ],
            ),
        )
        sessions.append_message(
            "legacy-telegram-session",
            "assistant",
            "Remembered",
        )
    finally:
        sessions.close()

    source_memory = root / "source-memory.db"
    memories = MemoryStore(db_path=source_memory)
    try:
        memories.add_fact(
            "The owner hosts a recurring technical meetup",
            category="community",
        )
    finally:
        memories.close()

    session_result = migration.export_source_sessions(payload / "sessions.jsonl", source_state)
    memory_result = migration.snapshot_source_memory(payload / "memory_store.db", source_memory)
    source_home = root / "source-home"
    (source_home / "workspace").mkdir(parents=True)
    (source_home / "dev/published-site").mkdir(parents=True)
    (source_home / "uploads").mkdir()
    (source_home / ".hermes/skills/legacy-finite").mkdir(parents=True)
    (source_home / "custom-data").mkdir()
    (source_home / ".finite").mkdir()
    (source_home / "workspace/README.md").write_text("legacy workspace\n", encoding="utf-8")
    (source_home / "custom-data/ledger.txt").write_text(
        "unknown durable source data\n", encoding="utf-8"
    )
    (source_home / "dev/published-site/index.html").write_text("legacy site\n", encoding="utf-8")
    (source_home / "uploads/photo.jpg").write_bytes(b"synthetic photo\n")
    (source_home / ".hermes/skills/legacy-finite/SKILL.md").write_text(
        "# Legacy Finite skill\n", encoding="utf-8"
    )
    (source_home / ".hermes/.env").write_text(
        "TELEGRAM_BOT_TOKEN=synthetic-mixed-version-token\n", encoding="utf-8"
    )
    (source_home / ".finite/device.key").write_text("legacy platform identity\n", encoding="utf-8")
    source_inventory = root / "bundle/source-volume-inventory.json"
    migration.inventory_source_volume(source_inventory, source_home)
    active_payload = root / "active-payload"
    migration.stage_source_active_payload(active_payload, source_home, source_inventory)
    shutil.copytree(active_payload, payload, dirs_exist_ok=True, symlinks=True)
    endpoints = root / "published-endpoints.json"
    endpoints.write_text(
        json.dumps(
            {
                "machineId": "legacy-bot",
                "endpoints": [
                    {
                        "hostname": "legacy-site.example.invalid",
                        "label": "Legacy Site",
                        "target_port": 3000,
                        "status": "published",
                        "run_command": "python3 -m http.server 3000",
                        "run_cwd": "/home/node/dev/published-site",
                        "desired_process_state": "running",
                        "auth": {"mode": "self"},
                        "created_at": "2026-07-01T00:00:00Z",
                        "updated_at": "2026-08-01T00:00:00Z",
                    }
                ],
            }
        ),
        encoding="utf-8",
    )
    sites = migration.inventory_source_sites(
        root / "bundle/sites.json",
        endpoints,
        source_inventory,
        expected_machine_id="legacy-bot",
    )
    integrations = migration.inventory_source_integrations(
        root / "bundle/integrations.json", source_home, source_inventory
    )
    with tarfile.open(payload / "source-home.tar", "w") as archive:
        archive.add(source_home, arcname=".", recursive=True)
    (payload / "source-home.tar").chmod(0o600)
    print(
        json.dumps(
            {
                "hermes_version": actual_version,
                "sessions": session_result,
                "memory": memory_result,
                "sites": sites,
                "integrations": integrations,
                "source_inventory_sha256": hashlib.sha256(
                    source_inventory.read_bytes()
                ).hexdigest(),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
