"""Synthetic box1 source home shaped like the 2026-11-13 metadata census.

The census (names, counts and config key names only) found: Telegram token and
home channel in `.env`, pairing stores in both `pairing/` and
`platforms/pairing/`, `jobs.json` with cron/interval/once schedules, scripts and
`/home/node` references, Hermes-bundled plus user skills, a box1-seeded
AGENTS.md, the same shared tool keys on every bot and loose files directly in
`/home/node`. Every value here is synthetic; no real token, id or content.
"""

from __future__ import annotations

import hashlib
import json
import shutil
import sqlite3
import tarfile
from collections.abc import Callable
from contextlib import closing
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

TOKEN = "7012345678:AAHsyntheticLegacyFixtureToken_0123456789"
OWNER_ID = 101  # approved in pairing/
SECOND_ID = 303  # approved only in platforms/pairing/
PENDING_ID = 404  # pending code in pairing/
PENDING_CODE = "KQ7MZ2PX"  # plaintext only in the fixture; stored hashed
PENDING_SALT = bytes(range(16))
SOUL_MARKER = "FRANKIE-PERSONA-7731"
# The active Telegram conversation when the bot moves (continuity proof).
ACTIVE_SESSION_ID = "legacy-telegram-active"
ACTIVE_USER_TEXT = "Please draft two poster versions for the meetup"
ACTIVE_ASSISTANT_TEXT = "Here they are: version A-BLUE-5521 and version B-GREEN-8834."
AGENTS_SEED = (
    "# Agent Computer Runtime Notes\n"
    "- Use `finitec publish` to publish Sites.\n"
    "- Traefik routes *.finite.computer to your container.\n"
)
AGENTS_OWNER_ADDITION = "## Owner preferences\n- Always answer in short paragraphs."
INTERVAL_MINUTES = 180
# A recurring job whose migrated script, skill and Telegram delivery the live
# test runs through the real gateway scheduler (it is never due on its own).
PROBE_JOB_ID = "cellar-probe"
PROBE_SCRIPT_MARKER = "CELLAR-PROBE-4471"
PROBE_PROMPT = "Report the cellar probe result to Owner."
MEMORY_MARKER = "MEMORY-MARKER-4410"
USER_SKILL = "leisure/wine-guidance"
CLASH_SKILL = "productivity/google-workspace-finite"
BUNDLED_SKILL = "apple/apple-notes"
TIMEZONE = "America/Chicago"
TOOL_KEYS = (
    "ELEVENLABS_API_KEY",
    "FAL_KEY",
    "FC_GITEA_HOST",
    "FC_GITEA_TOKEN",
    "FC_GITEA_USERNAME",
    "FC_MACHINE_API_TOKEN",
    "FINITE_PRIVATE_API_KEY",
    "FIRECRAWL_API_KEY",
    "FRED_API_KEY",
    "GOOGLE_PLACES_API_KEY",
    "OPENROUTER_API_KEY",
    "PERPLEXITY_API_KEY",
    "XAI_API_KEY",
    "X_API_BEARER_TOKEN",
)
SECRET_VALUES = (TOKEN, "synthetic-tool-secret", "synthetic-model-key")

SESSION_KEYS = (
    "source",
    "user_id",
    "model",
    "model_config",
    "system_prompt",
    "parent_session_id",
    "started_at",
    "ended_at",
    "end_reason",
    "message_count",
    "tool_call_count",
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "reasoning_tokens",
    "billing_provider",
    "billing_base_url",
    "billing_mode",
    "estimated_cost_usd",
    "actual_cost_usd",
    "cost_status",
    "cost_source",
    "pricing_version",
    "title",
    "api_call_count",
    "handoff_state",
    "handoff_platform",
    "handoff_error",
)
MESSAGE_KEYS = (
    "tool_call_id",
    "tool_calls",
    "tool_name",
    "token_count",
    "finish_reason",
    "reasoning",
    "reasoning_content",
    "reasoning_details",
    "codex_reasoning_items",
    "codex_message_items",
)


def _dir_md5(directory: Path) -> str:
    hasher = hashlib.md5()
    for path in sorted(directory.rglob("*")):
        if path.is_file():
            hasher.update(str(path.relative_to(directory)).encode("utf-8"))
            hasher.update(path.read_bytes())
    return hasher.hexdigest()


def _write(path: Path, text: str | bytes, mode: int | None = None) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if isinstance(text, bytes):
        path.write_bytes(text)
    else:
        path.write_text(text, encoding="utf-8")
    if mode is not None:
        path.chmod(mode)


def _config_yaml() -> str:
    # The census key set (census2.json "top"/"second"), synthetic values.
    return f"""model:
  default: glm-5-3-flash
  provider: custom
  base_url: https://finite-private.finite.containers.tinfoil.dev/v1
  api_mode: chat_completions
  context_length: 393216
  api_key: synthetic-model-key
fallback_providers: []
toolsets:
- hermes-cli
agent:
  max_turns: 90
  tool_use_enforcement: auto
  personality: kawaii
terminal:
  backend: local
  cwd: /home/node
  timeout: 180
browser:
  inactivity_timeout: 120
checkpoints:
  enabled: false
compression:
  enabled: true
  summary_model: glm-5-3-flash
smart_model_routing:
  enabled: false
auxiliary:
  vision:
    provider: auto
display:
  compact: false
  personality: kawaii
  streaming: true
  skin: default
  show_cost: false
  platforms:
    telegram:
      streaming: true
privacy:
  redact_pii: false
tts:
  provider: edge
  edge:
    voice: en-US-AriaNeural
  elevenlabs:
    voice_id: synthetic-voice
    api_key: synthetic-tool-secret
stt:
  enabled: true
  provider: local
voice:
  auto_tts: false
human_delay:
  mode: natural
  min_ms: 800
  max_ms: 2500
memory:
  provider: holographic
  memory_enabled: true
  user_profile_enabled: true
  memory_char_limit: 2200
  user_char_limit: 1375
plugins:
  enabled:
  - hermes-memory-store
gateway:
  platforms: {{}}
delegation:
  model: glm-5-3-flash
  api_key: synthetic-model-key
prefill_messages_file: /home/node/.hermes/prefill.json
finite_managed_skill_sources:
- name: finite
  url: https://github.com/finitecomputer/finite-skills.git
  ref: main
  auth: none
skills:
  external_dirs:
  - ~/.finite/managed-skills/finite/current
honcho: {{}}
timezone: {TIMEZONE}
discord:
  require_mention: true
whatsapp: {{}}
approvals:
  mode: off
command_allowlist: []
quick_commands: {{}}
personalities:
  kawaii: You are cheerful and use the occasional kaomoji.
security:
  redact_secrets: true
cron:
  wrap_response: true
_config_version: 10
session_reset:
  mode: idle
  idle_minutes: 1440
  at_hour: 4
onboarding:
  seen: true
"""


def _cron_expr(now: datetime, hours_ahead: int, minute: int) -> str:
    """A daily expression whose slot is `hours_ahead` hours from `now` in the bot's
    timezone, so no test run can cross a real slot (no wall-clock flake)."""
    from zoneinfo import ZoneInfo

    local = now.astimezone(ZoneInfo(TIMEZONE)) + timedelta(hours=hours_ahead)
    return f"{minute} {local.hour} * * *"


def _jobs(now: datetime) -> dict[str, Any]:
    origin = {"platform": "telegram", "chat_id": str(OWNER_ID), "chat_name": "Owner"}
    briefing = _cron_expr(now, 8, 0)
    pinned = _cron_expr(now, 10, 30)
    base = {
        "created_at": (now - timedelta(days=30)).isoformat(),
        "last_status": "ok",
        "last_error": None,
        "last_delivery_error": None,
        "repeat": {"times": None, "completed": 4},
        "skills": [],
        "skill": None,
        "model": None,
        "provider": None,
        "base_url": None,
        "script": None,
        "workdir": None,
        "no_agent": False,
        "enabled_toolsets": None,
        "context_from": None,
        "paused_at": None,
        "paused_reason": None,
        "state": "scheduled",
        "origin": origin,
    }
    stale = (now - timedelta(hours=3)).isoformat()
    return {
        "jobs": [
            {
                **base,
                "id": "daily-briefing",
                "name": "Daily briefing",
                "prompt": "Summarize /home/node/workspace/notes.md for Owner.",
                "schedule": {
                    "kind": "cron",
                    "expr": briefing,
                    "display": briefing,
                },
                "schedule_display": briefing,
                "deliver": "origin",
                "enabled": True,
                "last_run_at": (now - timedelta(days=1, hours=3)).isoformat(),
                "next_run_at": stale,
                "skills": ["leisure/wine-guidance"],
                "run_claim": {"owner": "box1-dead-process", "claimed_at": stale},
            },
            {
                **base,
                "id": "inbox-check",
                "name": "Inbox check",
                "prompt": "Run the check script and report changes.",
                "schedule": {
                    "kind": "interval",
                    "minutes": INTERVAL_MINUTES,
                    "display": f"every {INTERVAL_MINUTES}m",
                },
                "schedule_display": f"every {INTERVAL_MINUTES}m",
                "deliver": "telegram",
                "enabled": True,
                "script": "/home/node/.hermes/scripts/check_inbox.py",
                "workdir": "/home/node/dev/project",
                "last_run_at": stale,
                "next_run_at": stale,
            },
            {
                **base,
                "id": PROBE_JOB_ID,
                "name": "Cellar probe",
                "prompt": PROBE_PROMPT,
                "schedule": {
                    "kind": "interval",
                    "minutes": INTERVAL_MINUTES,
                    "display": f"every {INTERVAL_MINUTES}m",
                },
                "schedule_display": f"every {INTERVAL_MINUTES}m",
                "deliver": "telegram",
                "enabled": True,
                "skills": [USER_SKILL],
                "script": "/home/node/.hermes/scripts/cellar_probe.py",
                "workdir": "/home/node/dev/project",
                "last_run_at": stale,
                "next_run_at": stale,
            },
            {
                **base,
                "id": "future-reminder",
                "name": "Dentist reminder",
                "prompt": "Remind Owner about the dentist.",
                "schedule": {
                    "kind": "once",
                    "run_at": (now + timedelta(days=2)).isoformat(),
                    "display": "once",
                },
                "deliver": "origin",
                "enabled": True,
                "repeat": {"times": 1, "completed": 0},
                "last_run_at": None,
                "next_run_at": (now + timedelta(days=2)).isoformat(),
            },
            {
                **base,
                "id": "missed-reminder",
                "name": "Missed reminder",
                "prompt": "Remind Owner to call back.",
                "schedule": {"kind": "once", "run_at": stale, "display": "once"},
                "deliver": "origin",
                "enabled": True,
                "repeat": {"times": 1, "completed": 0},
                "last_run_at": None,
                "next_run_at": stale,
            },
            {
                **base,
                "id": "paused-digest",
                "name": "Paused digest",
                "prompt": "Weekly digest.",
                "schedule": {
                    "kind": "cron",
                    "expr": "0 9 * * 1",
                    "display": "0 9 * * 1",
                },
                "deliver": "local",
                "enabled": False,
                "state": "paused",
                "paused_at": stale,
                "paused_reason": "owner paused",
                "next_run_at": stale,
            },
            {
                **base,
                "id": "matrix-relay",
                "name": "Matrix relay",
                "prompt": "Post the status to the room.",
                "schedule": {"kind": "interval", "minutes": 60, "display": "every 60m"},
                "deliver": "origin",
                "enabled": True,
                "origin": {"platform": "matrix", "chat_id": "!room:matrix.invalid"},
                "next_run_at": stale,
            },
            {
                **base,
                "id": "openrouter-job",
                "name": "Pinned route",
                "prompt": "Draft a note in /home/node/Downloads.",
                "schedule": {"kind": "cron", "expr": pinned, "display": pinned},
                "deliver": "local",
                "enabled": True,
                "model": "anthropic/claude-sonnet-4.6",
                "provider": "openrouter",
                "base_url": "https://openrouter.ai/api/v1",
                "next_run_at": stale,
            },
            # Execution and recovery states the box1 scheduler left behind.
            {
                **base,
                "id": "errored-poll",
                "name": "Errored poll",
                "prompt": "Check the feed.",
                "schedule": {
                    "kind": "interval",
                    "minutes": 120,
                    "display": "every 120m",
                },
                "deliver": "origin",
                "enabled": True,
                "last_status": "error",
                "last_error": "synthetic old failure",
                "last_run_at": (now - timedelta(hours=6)).isoformat(),
                "next_run_at": (now - timedelta(hours=5, minutes=30)).isoformat(),
                "failure_streak": 3,
            },
            {
                **base,
                "id": "error-state-daily",
                "name": "Error state daily",
                "prompt": "Daily check.",
                "schedule": {"kind": "cron", "expr": briefing, "display": briefing},
                "deliver": "local",
                "enabled": True,
                "state": "error",
                "last_status": "error",
                "last_run_at": (now - timedelta(days=2)).isoformat(),
                "next_run_at": None,
            },
            {
                **base,
                "id": "exhausted-interval",
                "name": "Three times only",
                "prompt": "Ping three times.",
                "schedule": {"kind": "interval", "minutes": 60, "display": "every 60m"},
                "deliver": "local",
                "enabled": True,
                "repeat": {"times": 3, "completed": 3},
                "last_run_at": (now - timedelta(hours=4)).isoformat(),
                "next_run_at": stale,
            },
            {
                **base,
                "id": "completed-oneshot",
                "name": "Done reminder",
                "prompt": "Already reminded.",
                "schedule": {
                    "kind": "once",
                    "run_at": (now - timedelta(days=1)).isoformat(),
                    "display": "once",
                },
                "deliver": "origin",
                "enabled": False,
                "state": "completed",
                "repeat": {"times": 1, "completed": 1},
                "last_run_at": (now - timedelta(days=1)).isoformat(),
                "next_run_at": None,
            },
            {
                **base,
                "id": "claimed-oneshot",
                "name": "Claimed reminder",
                "prompt": "Remind Owner about the call.",
                "schedule": {
                    "kind": "once",
                    "run_at": (now + timedelta(hours=20)).isoformat(),
                    "display": "once",
                },
                "deliver": "origin",
                "enabled": True,
                "repeat": {"times": 1, "completed": 0},
                "last_run_at": None,
                "next_run_at": (now + timedelta(hours=20)).isoformat(),
                "run_claim": {"by": "box1-dead-process", "at": now.isoformat()},
                "fire_claim": {"by": "box1-dead-process", "at": now.isoformat()},
                "manual_run_at": (now - timedelta(minutes=5)).isoformat(),
            },
        ],
        "updated_at": stale,
    }


def _session(
    session_id: str,
    started: float,
    messages: list[tuple[str, Any]],
    stamps: list[float] | None = None,
    **extra,
) -> dict:
    session = {key: None for key in SESSION_KEYS}
    session.update(
        {
            "id": session_id,
            "source": "telegram",
            "user_id": str(OWNER_ID),
            "started_at": started,
            "message_count": len(messages),
            "tool_call_count": 0,
            "input_tokens": 0,
            "output_tokens": 0,
            "cache_read_tokens": 0,
            "cache_write_tokens": 0,
            "reasoning_tokens": 0,
            "api_call_count": 0,
        }
    )
    session.update(extra)
    rendered = []
    for index, (role, content) in enumerate(messages, start=1):
        message = {key: None for key in MESSAGE_KEYS}
        message.update(
            {
                "id": index,
                "session_id": session_id,
                "role": role,
                "content": content,
                "timestamp": stamps[index - 1] if stamps else started + index,
            }
        )
        rendered.append(message)
    session["messages"] = rendered
    return session


def sessions(now: datetime) -> list[dict[str, Any]]:
    started = (now - timedelta(days=3)).timestamp()
    # A long-running chat: started two days ago, last turn 20 minutes ago, so
    # only its real last activity keeps it inside the bot's idle window.
    active_started = (now - timedelta(days=2)).timestamp()
    return [
        _session(
            ACTIVE_SESSION_ID,
            active_started,
            [
                ("user", "Morning! Can you help with the meetup?"),
                ("assistant", "Of course."),
                ("user", ACTIVE_USER_TEXT),
                ("assistant", ACTIVE_ASSISTANT_TEXT),
            ],
            stamps=[
                active_started + 1,
                active_started + 2,
                (now - timedelta(minutes=22)).timestamp(),
                (now - timedelta(minutes=20)).timestamp(),
            ],
            cwd="/home/node",
        ),
        _session(
            "legacy-telegram-session",
            started,
            [
                (
                    "user",
                    [
                        {"type": "text", "text": "Please keep the poster"},
                        {"type": "file", "path": "/home/node/uploads/photo.txt"},
                        {
                            "type": "audio",
                            "path": "/home/node/.hermes/audio_cache/voice.ogg",
                        },
                    ],
                ),
                (
                    "assistant",
                    "Saved it to /home/node/poster_final.jpg and the memo to "
                    "/home/node/Downloads/memo.txt. MEDIA: /home/node/uploads/photo.txt",
                ),
            ],
            cwd="/home/node",
        ),
    ]


def routing_index(now: datetime) -> dict[str, Any]:
    """box1's gateway sessions.json, in the SessionEntry shape Hermes loads."""
    owner = {
        "platform": "telegram",
        "chat_id": str(OWNER_ID),
        "chat_name": "Owner",
        "chat_type": "dm",
        "user_id": str(OWNER_ID),
        "user_name": "Owner",
    }
    active = (now - timedelta(minutes=20)).isoformat()
    return {
        "_README": "routing index",
        f"agent:main:telegram:dm:{OWNER_ID}": {
            "session_key": f"agent:main:telegram:dm:{OWNER_ID}",
            "session_id": ACTIVE_SESSION_ID,
            "created_at": active,
            "updated_at": active,
            "origin": owner,
            "display_name": "Owner",
            "platform": "telegram",
            "chat_type": "dm",
            # Process claims of the frozen box1 gateway; never carried over.
            "active_turn_token": "box1-stale-turn",
            "active_turn_started_at": active,
            "resume_pending": True,
            "suspended": True,
            "model_override": {"model": "anthropic/claude-sonnet-4.6"},
        },
        "agent:main:matrix:group:!room:matrix.invalid": {
            "session_key": "agent:main:matrix:group:!room:matrix.invalid",
            "session_id": "legacy-telegram-session",
            "created_at": active,
            "updated_at": active,
            "origin": {
                "platform": "matrix",
                "chat_id": "!room:matrix.invalid",
                "chat_type": "group",
            },
            "platform": "matrix",
            "chat_type": "group",
        },
    }


def pending_entry(now: datetime) -> dict[str, Any]:
    """A pairing code issued just before the freeze, hashed as Hermes stores it."""
    return {
        "hash": hashlib.sha256(PENDING_SALT + PENDING_CODE.encode()).hexdigest(),
        "salt": PENDING_SALT.hex(),
        "user_id": str(PENDING_ID),
        "user_name": "Pending",
        "created_at": (now - timedelta(minutes=10)).timestamp(),
    }


def simple_memory_store(path: Path) -> int:
    with closing(sqlite3.connect(path)) as connection:
        connection.execute(
            "CREATE TABLE facts (fact_id INTEGER PRIMARY KEY, content TEXT)"
        )
        connection.execute(
            "INSERT INTO facts VALUES (1, 'Owner hosts a recurring AI meetup; "
            "the notes are at /home/node/workspace/notes.md')"
        )
        connection.commit()
    return 1


def build_source_home(root: Path, now: datetime) -> Path:
    home = root / "source-home"
    hermes = home / ".hermes"
    # Loose home files and directories (census home_top).
    _write(home / "AGENTS.md", AGENTS_SEED + "\n" + AGENTS_OWNER_ADDITION + "\n")
    _write(home / "poster_final.jpg", b"\xff\xd8\xff\xe0synthetic-jpeg\x00\x01")
    _write(
        home / "make_post.py",
        "OUT = '/home/node/workspace/post.png'\nprint(OUT)\n",
        0o755,
    )
    _write(home / "Downloads/memo.txt", "memo from Downloads\n")
    # Binary file that mentions the old home: kept as is, listed in the receipt.
    _write(home / "Downloads/cache.bin", b"\x00\x01/home/node/workspace/post.png\x00")
    # azadi has both Downloads/ and downloads/; a case-insensitive Mac test
    # filesystem cannot hold both, so the second loose dir is recipes/ (lavender).
    _write(home / "recipes/report.csv", "a,b\n1,2\n")
    _write(home / "lessons/week1.md", "lesson one\n")
    (home / "lessons/latest").symlink_to("week1.md")
    (home / "lessons/workspace-link").symlink_to("/home/node/workspace/notes.md")
    (home / "lessons/hermes-cache-link").symlink_to(
        "/home/node/.hermes/audio_cache/voice.ogg"
    )
    _write(home / "node_modules/left-pad/index.js", "module.exports = 1;\n")
    _write(home / "__pycache__/make_post.cpython-312.pyc", b"\x00pyc")
    _write(home / "finitebrain/notes.md", "legacy brain working tree\n")
    _write(home / "workspace/notes.md", "workspace notes\n")
    _write(home / "workspace/plan.md", "Final memo: /home/node/Downloads/memo.txt\n")
    _write(home / "dev/project/README.md", "project\n")
    (home / "dev/project/readme-link").symlink_to("README.md")
    _write(home / "uploads/photo.txt", "upload\n")
    _write(home / ".finite/device.key", "legacy platform identity\n")
    _write(home / ".codex/config.toml", "model = 'x'\n")
    _write(home / ".cache/pip/x", "cache\n")
    _write(home / ".npm/_cacache/x", "cache\n")
    # Hermes home.
    _write(
        hermes / "SOUL.md",
        f"You are Frankie, a warm and witty helper. {SOUL_MARKER}\n"
        "Keep drafts in /home/node/workspace and finished files in ~/Downloads.\n",
    )
    _write(hermes / "AGENTS.md", AGENTS_SEED)
    _write(hermes / "config.yaml", _config_yaml())
    env_lines = [f"{name}=synthetic-tool-secret" for name in TOOL_KEYS]
    env_lines += [f"TELEGRAM_BOT_TOKEN={TOKEN}", f"TELEGRAM_HOME_CHANNEL={OWNER_ID}"]
    _write(hermes / ".env", "\n".join(sorted(env_lines)) + "\n", 0o600)
    _write(
        hermes / "memories/MEMORY.md",
        f"Owner keeps travel plans in /home/node/workspace/trips.md. {MEMORY_MARKER}\n",
    )
    _write(hermes / "memories/MEMORY.md.lock", "")
    _write(hermes / "memories/USER.md", "Owner prefers short answers.\n")
    _write(hermes / "memories/USER.md.lock", "")
    skills = hermes / "skills"
    _write(
        skills / BUNDLED_SKILL / "SKILL.md",
        "---\nname: apple-notes\ndescription: Bundled\n---\n# Apple Notes\n",
    )
    _write(
        skills / USER_SKILL / "SKILL.md",
        "---\nname: wine-guidance\ndescription: Owner's wine notes\n---\n"
        "Read /home/node/workspace/cellar.md before recommending.\n",
    )
    _write(skills / USER_SKILL / "references/regions.md", "Rioja, Barolo\n")
    _write(
        skills / CLASH_SKILL / "SKILL.md",
        "---\nname: google-workspace-finite\ndescription: Local override\n---\nLocal copy\n",
    )
    overlay_text = (
        b"---\nname: overlay-helper\n---\nShipped by the box1 image overlay\n"
    )
    _write(skills / "devops/overlay-helper/SKILL.md", overlay_text)
    # A managed overlay skill the bot edited after the image shipped it.
    _write(
        skills / "devops/edited-helper/SKILL.md",
        "---\nname: edited-helper\n---\nShipped, then edited by the bot\n",
    )
    _write(
        skills / ".archive/old-skill/SKILL.md", "---\nname: old-skill\n---\narchived\n"
    )
    _write(
        skills / ".bundled_manifest",
        f"apple-notes:{_dir_md5(skills / BUNDLED_SKILL)}\n",
    )
    managed = hermes / ".runtime-managed/.hermes/skills/devops"
    _write(
        managed / "overlay-helper/SKILL.md.sha256",
        hashlib.sha256(overlay_text).hexdigest() + "\n",
    )
    _write(
        managed / "edited-helper/SKILL.md.sha256",
        hashlib.sha256(b"the version the image shipped").hexdigest() + "\n",
    )
    _write(hermes / ".runtime-managed/.hermes/config.yaml.sha256", "0" * 64 + "\n")
    _write(hermes / ".skills_prompt_snapshot.json", "{}\n")
    _write(
        hermes / "scripts/check_inbox.py",
        "import pathlib\nprint(pathlib.Path('/home/node/workspace/notes.md').exists())\n",
        0o755,
    )
    _write(
        hermes / "scripts/cellar_probe.py",
        "import pathlib\n"
        "pathlib.Path(__file__).with_name('cellar-probe.out').write_text('ran\\n')\n"
        f"print('{PROBE_SCRIPT_MARKER} cellar list: /home/node/workspace/cellar.md')\n",
        0o755,
    )
    _write(hermes / "cron/jobs.json", json.dumps(_jobs(now), indent=2))
    _write(hermes / "cron/output/daily-briefing/old.md", "old output\n")
    _write(
        hermes / "pairing/telegram-approved.json",
        json.dumps({str(OWNER_ID): {"user_name": "Owner", "approved_at": 1.0}}),
    )
    _write(
        hermes / "pairing/telegram-pending.json",
        json.dumps({"0123456789abcdef": pending_entry(now)}),
    )
    _write(
        hermes / "pairing/_rate_limits.json",
        json.dumps({f"telegram:{PENDING_ID}": 1.0}),
    )
    _write(
        hermes / "platforms/pairing/telegram-approved.json",
        json.dumps(
            {
                str(SECOND_ID): {"user_name": "Second", "approved_at": 2.0},
                str(OWNER_ID): {"user_name": "Owner (older)", "approved_at": 0.5},
            }
        ),
    )
    _write(hermes / "platforms/pairing/telegram-pending.json", "{}")
    _write(
        hermes / "pairing/signal-approved.json",
        json.dumps({"+15550000000": {"user_name": "S"}}),
    )
    _write(hermes / "google_token.json", '{"token":"synthetic-tool-secret"}\n', 0o600)
    _write(hermes / "auth.json", '{"providers":{}}\n', 0o600)
    _write(hermes / "channel_directory.json", "{}\n")
    _write(hermes / "gateway_state.json", "{}\n")
    _write(hermes / "kanban.db", b"SQLite format 3\x00synthetic")
    _write(hermes / "sessions/sessions.json", json.dumps(routing_index(now), indent=2))
    _write(hermes / "plugins/owner-plugin/plugin.yaml", "name: owner-plugin\n")
    _write(hermes / "hooks/on_start.sh", "#!/bin/sh\n", 0o755)
    _write(hermes / "bin/helper", "#!/bin/sh\n", 0o755)
    _write(hermes / "state.db", b"SQLite format 3\x00synthetic-state")
    _write(hermes / "memory_store.db", b"SQLite format 3\x00synthetic-memory")
    _write(hermes / "venv/bin/python", "#!/bin/sh\n", 0o755)
    _write(hermes / "audio_cache/voice.ogg", b"OggS-synthetic")
    _write(hermes / "logs/gateway.log", "log\n")
    return home


def build_bundle(
    root: Path,
    migration,
    *,
    now: datetime | None = None,
    memory_store_writer: Callable[[Path], int] = simple_memory_store,
    sessions_writer: Callable[[Path, datetime], None] | None = None,
    stage_agents_seed: bool = True,
) -> dict[str, Any]:
    """Build a bundle exactly as runbook step 5 stages it, then return evidence."""
    now = now or datetime.now(timezone.utc)
    home = build_source_home(root, now)
    bundle = root / "bundle"
    payload = bundle / "payload"
    (payload / "hermes").mkdir(parents=True)
    (payload / "home").mkdir(parents=True)
    inventory = bundle / "source-volume-inventory.json"
    migration.inventory_source_volume(inventory, home)
    endpoints = root / "published-endpoints.json"
    endpoints.write_text(
        json.dumps(
            {
                "machineId": "frankie",
                "endpoints": [
                    {
                        "hostname": "frankie-site.finite.computer",
                        "label": "Frankie Site",
                        "target_port": 3000,
                        "status": "published",
                        "run_command": "python3 -m http.server 3000",
                        "run_cwd": "/home/node/dev/project",
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
    migration.inventory_source_sites(
        bundle / "sites.json", endpoints, inventory, expected_machine_id="frankie"
    )
    migration.inventory_source_integrations(
        bundle / "integrations.json", home, inventory
    )
    with tarfile.open(payload / "source-home.tar", "w") as archive:
        archive.add(home, arcname=".", recursive=True)
    (payload / "source-home.tar").chmod(0o600)
    hermes = home / ".hermes"
    for name in ("memories", "skills", "scripts"):
        shutil.copytree(hermes / name, payload / "hermes" / name, symlinks=True)
    (payload / "hermes/cron").mkdir()
    shutil.copy2(hermes / "cron/jobs.json", payload / "hermes/cron/jobs.json")
    for name in ("workspace", "dev", "uploads"):
        shutil.copytree(home / name, payload / "home" / name, symlinks=True)
    if stage_agents_seed:
        # Runbook: the seed the box1 pod renders into /home/node/AGENTS.md.
        _write(bundle / "box1-seed/AGENTS.md", AGENTS_SEED)
    if sessions_writer is None:
        with (payload / "sessions.jsonl").open("w", encoding="utf-8") as handle:
            for session in sessions(now):
                handle.write(json.dumps(session, ensure_ascii=False) + "\n")
    else:
        sessions_writer(payload / "sessions.jsonl", now)
    facts = memory_store_writer(payload / "memory_store.db")
    return {
        "home": home,
        "bundle": bundle,
        "facts": facts,
        "now": now,
        "inventory_sha256": hashlib.sha256(inventory.read_bytes()).hexdigest(),
    }


def source_metadata(migration, inventory_sha256: str):
    return migration.SourceMetadata(
        host_id="box1",
        machine_id="frankie",
        owner_email="owner@example.invalid",
        hermes_version="0.14.0",
        image_reference="docker.io/library/fc-agent-runtime:main-015rdv4dc6hz",
        image_manifest_digest="sha256:" + "a" * 64,
        container_image_id="sha256:" + "b" * 64,
        source_inventory_sha256=inventory_sha256,
    )


DEFAULT_SOUL = "You are Hermes Agent, built by Nous Research.\n"
MANAGED_SKILLS = "/data/agent/managed-skills/finite/current"


def build_target(root: Path, config_text: str | None = None) -> Path:
    """A booted, fresh v2 target: identity, Chat store, seeded Hermes home."""
    target = root / "target"
    hermes = target / "agent/hermes-home"
    (target / "agent/identity").mkdir(parents=True)
    hermes.mkdir(parents=True)
    (target / "workspace").mkdir()
    (target / "agent/identity/identity.json").write_text('{"npub":"npub-target"}\n')
    (target / "agent/client.sqlite3").write_bytes(b"target-chat-store")
    if config_text is None:
        config_text = (
            "model:\n  default: glm-5-3-flash\n  provider: custom\n"
            "  base_url: https://finite-private.finite.containers.tinfoil.dev/v1\n"
            "  api_mode: chat_completions\n  api_key: ${FINITE_PRIVATE_API_KEY}\n"
            "terminal:\n  backend: local\n  cwd: /data/workspace\n"
            "approvals:\n  mode: 'off'\ndisplay:\n  streaming: false\n"
            "  platforms:\n    finitechat:\n      streaming: false\n"
            "plugins:\n  enabled:\n  - finitechat\n"
            "gateway:\n  platforms:\n    finitechat:\n      enabled: true\n"
            f"skills:\n  external_dirs:\n  - {MANAGED_SKILLS}\n"
            "_config_version: 10\n"
        )
    (hermes / "config.yaml").write_text(config_text)
    (hermes / "config.yaml").chmod(0o600)
    (hermes / "SOUL.md").write_text(DEFAULT_SOUL)
    managed = target / MANAGED_SKILLS.removeprefix("/data/")
    _write(
        managed / "productivity/google-workspace-finite/SKILL.md",
        "---\nname: google-workspace-finite\n---\nManaged baseline\n",
    )
    _write(
        managed / "software-development/finitebrain/SKILL.md",
        "---\nname: finitebrain\n---\n",
    )
    return target
