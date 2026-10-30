#!/usr/bin/env python3
"""Export an agent on the old Finite into one zip for the new Finite.

    python3 export.py survey
    python3 export.py build [--brain ID]... [--site SLUG]... [--leave-out-site SLUG]...
                            [--no-workspace] [--max-file-mb N] [--out DIR]

`survey` changes nothing: it prints, as JSON, what there is to export and
what to ask the person. `build` writes the zip and prints, as JSON, where
it is and whether it fits in one chat attachment. export.md, served
beside this file, is the format and the rules.

Each command tells the move site it ran (`report`, below): the agent's
ids and name, its owners' account ids and counts, never contents.
`--no-report` sends nothing.

Standard library only, so it runs on the old runtime's own Python. It
never copies a credential: the new agent signs in to everything again.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import mimetypes
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import urllib.parse
import urllib.request
import uuid
import zipfile
from dataclasses import dataclass
from pathlib import Path

FORMAT = "finite-agent-export/1"
CHAT_FORMAT = "finite-chat-archive/1"
MiB = 1024 * 1024
# The new chat takes attachments of at most 25 MiB; leave room.
LIMIT_BYTES = int(os.environ.get("FINITE_EXPORT_LIMIT_BYTES", str(24 * MiB)))
DEFAULT_MAX_FILE_MB = 5
DEFAULT_PICTURE = "https://avatars.githubusercontent.com/u/274919006?v=4"

TOOL_TEXT_MAX = 2000
REASONING_MAX = 2000
ARGUMENTS_MAX = 1000
CRON_OUTPUT_MAX = 4000

# Where each run reports that it ran: the move site's own `report`
# operation (fragment.json beside its pages). Empty sends nothing.
GUIDE_URL = "https://move.finite.place"
REPORT_URL = f"{GUIDE_URL}/__op/report"
REPORT_TIMEOUT_S = 8
RUN_ID = uuid.uuid4().hex[:16]

# Directories no export wants: version control, dependencies, caches.
SKIP_DIRS = {
    ".git",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    ".next",
    ".finitebrain",
    ".mypy_cache",
    ".pytest_cache",
    ".tox",
    ".gradle",
    "bower_components",
    "site-packages",
    "Pods",
    "DerivedData",
    "elm-stuff",
}
# What a tool builds beside a project file, made again from the source
# around it. Left out of the workspace only beside such a file: a folder
# of notes may be called "build" too.
BUILD_DIRS = {"dist", "build", "out", "target", "_build", "obj", "deps", "vendor", "coverage"}
PROJECT_FILES = {
    "package.json",
    "deno.json",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "setup.py",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "Makefile",
    "CMakeLists.txt",
    "mix.exs",
    "Gemfile",
    "composer.json",
    "pubspec.yaml",
    "Package.swift",
    "build.zig",
    "stack.yaml",
}
PROJECT_SUFFIXES = (".csproj", ".fsproj", ".sln", ".cabal")
# Kinds of file the workspace leaves out at any size: video and audio,
# archives and installers, model weights and compiled code. They are most
# of a big workspace's bytes and can't ride in one attachment; they stay
# on the old Finite, counted in the manifest by kind.
MEDIA_AND_BINARIES = {
    ".mp4",
    ".mov",
    ".m4v",
    ".avi",
    ".mkv",
    ".webm",
    ".wmv",
    ".flv",
    ".mpg",
    ".mpeg",
    ".3gp",
    ".mp3",
    ".wav",
    ".flac",
    ".aac",
    ".m4a",
    ".ogg",
    ".oga",
    ".opus",
    ".aif",
    ".aiff",
    ".wma",
    ".zip",
    ".tar",
    ".gz",
    ".tgz",
    ".bz2",
    ".xz",
    ".zst",
    ".7z",
    ".rar",
    ".dmg",
    ".iso",
    ".img",
    ".pkg",
    ".deb",
    ".rpm",
    ".apk",
    ".ipa",
    ".msi",
    ".exe",
    ".jar",
    ".war",
    ".whl",
    ".safetensors",
    ".ckpt",
    ".pt",
    ".pth",
    ".onnx",
    ".gguf",
    ".ggml",
    ".bin",
    ".h5",
    ".npy",
    ".npz",
    ".pkl",
    ".tflite",
    ".o",
    ".so",
    ".dylib",
    ".dll",
    ".a",
    ".lib",
    ".class",
    ".pyc",
    ".wasm",
    ".map",
}
# How many of a kind's files (its largest) or folders the manifest names.
KIND_EXAMPLES = 10
# A workspace bigger than this, after what is left out by kind, can't
# share one attachment with the rest: export.md asks about it.
WORKSPACE_ASK_BYTES = int(os.environ.get("FINITE_EXPORT_WORKSPACE_ASK_BYTES", str(20 * MiB)))
# Files that hold, or may hold, a credential, by name. Never exported.
SECRET_NAMES = {
    ".env",
    ".envrc",
    "auth.json",
    "google_token.json",
    "google_client_secret.json",
    "google_oauth_pending.json",
    "identity.json",
    "git-credentials",
    ".git-credentials",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".pgpass",
    "id_rsa",
    "id_ed25519",
    "id_ecdsa",
    "credentials.json",
    "client_secret.json",
    "token.json",
    "secrets.yaml",
    "secrets.yml",
    "secrets.json",
}
SECRET_SUFFIXES = (".pem", ".key", ".p12", ".pfx", ".keystore", ".env")
SECRET_NAME_SHAPES = re.compile(r"^(service[-_]?account.*|client_secret.*)\.json$", re.IGNORECASE)
# Files a brain makes for itself in each folder (fbrain's sync_engine).
BRAIN_GENERATED = {"AGENTS.md", "_index.md", ".keep"}

# Text shaped like a credential. A file holding one is left out (memories
# and the SOUL have it replaced instead); chats are the person's own and
# are exported as they are. A token starts at a word's start (so the
# "sk-" inside "coindesk-dl..." is none), and a placeholder ("sk-xxxx",
# "AKIA...EXAMPLE", "https://USER:TOKEN@", "${TOKEN}") is not a secret.
TOKEN_SHAPES = re.compile(
    r"(?<![A-Za-z0-9_-])(?P<token>sk-[A-Za-z0-9_-]{16,}|[sr]k_live_[A-Za-z0-9]{16,}|ghp_[A-Za-z0-9]{20,}"
    r"|github_pat_[A-Za-z0-9_]{20,}|glpat-[A-Za-z0-9_-]{20,}|xox[abpr]-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z_-]{30,}"
    r"|AKIA[0-9A-Z]{16}|ntn_[A-Za-z0-9]{20,}|hf_[A-Za-z0-9]{30,}|\d{6,}:[A-Za-z0-9_-]{30,}|nsec1[0-9a-z]{20,}"
    r"|eyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{10,})"
    r"|(?P<key>-----BEGIN [A-Z ]*PRIVATE KEY-----)"
    r"|(?P<hook>discord(?:app)?\.com/api/webhooks/\d+/[\w-]+)"
    r"|(?P<url>https?://[^\s/:@]+:(?P<password>[^\s/@]+)@)"
)
PLACEHOLDER = re.compile(
    r"x{4,}|\*{4,}|\.{3}|[<>${}]|your|example|placeholder|redacted|changeme", re.IGNORECASE
)
SCAN_MAX_BYTES = 2 * MiB


@dataclass(frozen=True)
class Paths:
    finite_home: Path
    hermes_home: Path
    workspace: Path
    brain_root: Path
    managed_skills: Path
    image_skills: Path
    legacy: Path


def paths_from_env(env) -> Paths:
    finite_home = Path(env.get("FINITE_HOME", "/data/agent"))
    workspace = Path(env.get("FINITECHAT_WORKSPACE", "/data/workspace"))
    return Paths(
        finite_home=finite_home,
        hermes_home=Path(env.get("HERMES_HOME", str(finite_home / "hermes-home"))),
        workspace=workspace,
        brain_root=Path(env.get("FBRAIN_WORKING_TREE_ROOT", str(workspace / "finitebrain"))),
        managed_skills=Path(
            env.get(
                "FINITE_EXPORT_MANAGED_SKILLS", str(finite_home / "managed-skills/finite/current")
            )
        ),
        image_skills=Path(env.get("FINITE_EXPORT_IMAGE_SKILLS", "/runtime/finite-skills")),
        legacy=Path(env.get("FINITE_EXPORT_LEGACY", "/data/migration")),
    )


# --- Small helpers -----------------------------------------------------------


def run(args, cwd=None, timeout=600):
    """Run a command; (stdout, "") on success, (None, why) on failure."""
    env = {**os.environ, "GIT_TERMINAL_PROMPT": "0"}
    try:
        done = subprocess.run(
            args, cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        return None, f"{args[0]}: {error}"
    if done.returncode != 0:
        detail = (done.stderr or done.stdout).strip()[-400:]
        return None, f"{' '.join(args[:3])} exited {done.returncode}: {detail}"
    return done.stdout, ""


def run_json(args, cwd=None, timeout=600):
    out, error = run(args, cwd, timeout)
    if out is None:
        return None, error
    try:
        return json.loads(out), ""
    except ValueError:
        return None, f"{' '.join(args[:3])} printed no JSON"


def iso(seconds):
    if seconds is None:
        return None
    try:
        return (
            datetime.datetime.fromtimestamp(float(seconds), datetime.UTC)
            .isoformat()
            .replace("+00:00", "Z")
        )
    except (TypeError, ValueError, OverflowError, OSError):
        return None


def now_iso():
    return (
        datetime.datetime.now(datetime.UTC)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
    )


def read_text(path, default=""):
    try:
        return Path(path).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return default


def read_json(path, default=None):
    try:
        return json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return default


def clip(text, limit):
    if len(text) <= limit:
        return text
    return f"{text[:limit]}… [{len(text) - limit} more characters not exported]"


def slug(text):
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-") or "agent"


def safe_name(text):
    return re.sub(r"[^A-Za-z0-9_.-]+", "_", text)[:120] or "_"


def frontmatter(text):
    """The simple `key: value` lines of a leading `---` block."""
    if not text.startswith("---"):
        return {}
    end = text.find("\n---", 3)
    if end < 0:
        return {}
    pairs = (
        line.split(":", 1)
        for line in text[3:end].splitlines()
        if ":" in line and not line.startswith((" ", "\t"))
    )
    return {key.strip(): value.strip().strip("\"'") for key, value in pairs}


def secret_reason(name):
    if (
        name in SECRET_NAMES
        or name.startswith(".env.")
        or name.endswith(SECRET_SUFFIXES)
        or SECRET_NAME_SHAPES.match(name)
    ):
        return "may hold a secret"
    return None


def is_secret(match):
    """A credential-shaped match that is no placeholder."""
    password = match.group("password")
    if password is not None:
        return (
            len(password) >= 6
            and not PLACEHOLDER.search(password)
            and not re.fullmatch(r"[A-Z0-9_]+", password)
        )
    token = match.group("token")
    if token is None:
        return True
    # A key is random: a digit somewhere, or long. "sk-is-a-slug" is neither.
    return not PLACEHOLDER.search(token) and (any(c.isdigit() for c in token) or len(token) >= 32)


def secret_in_string(text):
    return any(is_secret(match) for match in TOKEN_SHAPES.finditer(text))


def secret_in_text(path):
    """Whether a text file holds something shaped like a credential."""
    try:
        if path.stat().st_size > SCAN_MAX_BYTES:
            return False
        data = path.read_bytes()
    except OSError:
        return False
    if b"\x00" in data[:8192]:
        return False
    return secret_in_string(data.decode("utf-8", "ignore"))


def redact_text(text):
    return TOKEN_SHAPES.sub(
        lambda match: "[REDACTED]" if is_secret(match) else match.group(0), text
    )


def leave_out_reason(path, max_bytes):
    """Why a file stays out of the zip, or None."""
    return (
        secret_reason(path.name)
        or too_large(path, max_bytes)
        or ("looks like it holds a secret" if secret_in_text(path) else None)
    )


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(MiB), b""):
            digest.update(block)
    return digest.hexdigest()


def walk_files(base, skip_dir):
    """Each regular file under `base` as (path, relative path), pruning the
    directories `skip_dir(relative, absolute)` rejects. Symlinks are left."""
    base = Path(base)
    for dirpath, dirnames, filenames in os.walk(base):
        here = Path(dirpath)
        dirnames[:] = sorted(
            d for d in dirnames if not skip_dir((here / d).relative_to(base), here / d)
        )
        for name in sorted(filenames):
            path = here / name
            if not path.is_symlink() and path.is_file():
                yield path, path.relative_to(base)


def skip_common(relative, _absolute):
    return relative.name in SKIP_DIRS


def too_large(path, max_bytes):
    size = path.stat().st_size
    if size <= max_bytes:
        return None
    return f"too large ({size / MiB:.1f} MiB; the limit is {max_bytes / MiB:.0f} MiB)"


# --- The staging folder -----------------------------------------------------


class Stage:
    """The folder the zip is made from, and what was left out of it."""

    def __init__(self, root):
        self.root = Path(root)
        self.skipped = []
        self.problems = []

    def write_text(self, relative, text):
        target = self.root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")

    def write_json(self, relative, value, compact=False):
        separators = (",", ":") if compact else None
        text = json.dumps(
            value, ensure_ascii=False, indent=None if compact else 2, separators=separators
        )
        self.write_text(relative, text + "\n")

    def copy(self, source, relative):
        target = self.root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)

    def skip(self, relative, reason, size=None):
        entry = {"path": str(relative), "reason": reason}
        self.skipped.append({**entry, "bytes": size} if size is not None else entry)

    def skip_kind(self, relative, kind, reason, facts):
        """Many files or folders left out for one reason, as one entry."""
        self.skipped.append({"path": str(relative), "kind": kind, "reason": reason, **facts})

    def problem(self, text):
        self.problems.append(text)

    def copy_tree(
        self,
        source,
        prefix,
        skip_dir,
        max_bytes,
        silent=lambda name: False,
        set_aside=lambda path, target: False,
    ):
        """Copy `source` under `prefix`, leaving out secrets, large files and
        unreadable ones (each noted), whatever `silent` names (not noted),
        and whatever `set_aside` takes (it notes them itself)."""
        count, total = 0, 0
        for path, relative in walk_files(source, skip_dir):
            target = f"{prefix}/{relative.as_posix()}"
            if silent(path.name):
                continue
            try:
                if set_aside(path, target):
                    continue
                reason = leave_out_reason(path, max_bytes)
                if reason:
                    self.skip(target, reason, path.stat().st_size)
                    continue
                self.copy(path, target)
            except OSError as error:
                self.skip(target, f"could not be read: {error.strerror or error}")
                continue
            count, total = count + 1, total + path.stat().st_size
        return {"files": count, "bytes": total}


# --- The agent itself --------------------------------------------------------


def agent_profile(paths, env):
    picture = env.get("FINITECHAT_HERMES_AGENT_PICTURE_URL") or ""
    version, _ = run(["hermes", "--version"], timeout=60)
    return {
        "name": env.get("FINITE_AGENT_NAME")
        or env.get("FINITECHAT_HERMES_AGENT_NAME")
        or "Finite Agent",
        "picture_url": picture or None,
        "picture_is_default": picture in ("", DEFAULT_PICTURE),
        "hermes_version": version.strip().splitlines()[0] if version and version.strip() else None,
        "has_soul": (paths.hermes_home / "SOUL.md").is_file(),
    }


def scalar(value):
    value = value.split(" #", 1)[0].strip().strip("\"'")
    return None if value in ("", "|", ">", "null", "~") or secret_in_string(value) else value


def config_settings(text):
    """The few settings of config.yaml an import uses, read by name; the
    rest of it (keys, tokens, servers, headers) never leaves. Its time
    zone (top-level `timezone`), personality (`agent.personality`) and
    default model (`model.default`, or a scalar `model`)."""
    found, section = {}, None
    for line in text.splitlines():
        top = re.match(r"^([A-Za-z_][\w-]*):\s*(.*)$", line)
        if top:
            section, value = top.group(1), top.group(2)
            if section in ("timezone", "model", "personality") and scalar(value):
                found = {**found, section: scalar(value)}
            continue
        nested = re.match(r"^\s{2}([A-Za-z_][\w-]*):\s*(.*)$", line)
        if (
            nested
            and (section, nested.group(1)) in (("agent", "personality"), ("model", "default"))
            and scalar(nested.group(2))
        ):
            found = {
                **found,
                ("personality" if section == "agent" else "model"): scalar(nested.group(2)),
            }
    return found


def fetch_picture(url, stage):
    try:
        with urllib.request.urlopen(url, timeout=20) as response:
            data = response.read(5 * MiB + 1)
            kind = response.headers.get_content_type()
    except (OSError, ValueError) as error:
        stage.problem(f"agent picture: could not download {url}: {error}")
        return None
    if len(data) > 5 * MiB or not kind.startswith("image/"):
        stage.problem(f"agent picture: {url} is not an image of at most 5 MiB")
        return None
    name = f"agent/avatar{mimetypes.guess_extension(kind) or '.img'}"
    stage.write_text(name, "")
    (stage.root / name).write_bytes(data)
    return name


def export_agent(paths, env, stage):
    profile = agent_profile(paths, env)
    soul = paths.hermes_home / "SOUL.md"
    if soul.is_file():
        stage.write_text("agent/SOUL.md", redact_text(read_text(soul)))
    stage.write_json(
        "agent/settings.json", config_settings(read_text(paths.hermes_home / "config.yaml"))
    )
    picture = None
    if profile["picture_url"] and not profile["picture_is_default"]:
        picture = fetch_picture(profile["picture_url"], stage)
    stage.write_json("agent/profile.json", {**profile, "picture_file": picture})
    return profile


def memory_files(paths, max_bytes):
    """Hermes' memory files, less its locks and hidden files."""
    memories = paths.hermes_home / "memories"
    if not memories.is_dir():
        return []
    return [
        path
        for path in sorted(memories.iterdir())
        if path.is_file()
        and not path.name.startswith(".")
        and not path.name.endswith(".lock")
        and not too_large(path, max_bytes)
    ]


def export_memories(paths, stage, max_bytes):
    copied = []
    for path in memory_files(paths, max_bytes):
        stage.write_text(f"memories/{path.name}", redact_text(read_text(path)))
        copied.append(path.name)
    store = paths.hermes_home / "memory_store.db"
    if store.is_file():
        snapshot_sqlite(store, stage.root / "memories" / "memory_store.db")
        copied.append("memory_store.db")
    return {"files": copied}


def integrations(paths):
    config = read_text(paths.hermes_home / "config.yaml")
    google = read_json(paths.finite_home / "agentd" / "google-workspace.json", {}) or {}
    email = google.get("email") if isinstance(google, dict) else None
    return {
        "telegram": bool(re.search(r"^\s+telegram:\s*$", config, re.MULTILINE)),
        "google": {
            "connected": bool(email) or (paths.hermes_home / "google_token.json").is_file(),
            "email": email,
        },
        "simplex": (paths.finite_home / "simplex").is_dir(),
    }


# --- Chats ------------------------------------------------------------------


def snapshot_sqlite(source, target):
    """A consistent copy of a SQLite database another process may be writing."""
    target.parent.mkdir(parents=True, exist_ok=True)
    try:
        src = sqlite3.connect(f"file:{source}?mode=ro", uri=True)
        try:
            dst = sqlite3.connect(str(target))
            src.backup(dst)
            dst.close()
        finally:
            src.close()
    except sqlite3.Error:
        for suffix in ("", "-wal", "-shm"):
            if Path(f"{source}{suffix}").exists():
                shutil.copyfile(f"{source}{suffix}", f"{target}{suffix}")
    return target


def open_sessions(paths, scratch):
    db = paths.hermes_home / "state.db"
    if not db.is_file():
        return None
    conn = sqlite3.connect(str(snapshot_sqlite(db, Path(scratch) / "state.db")))
    conn.row_factory = sqlite3.Row
    return conn


def load_sessions(conn):
    return [dict(row) for row in conn.execute("SELECT * FROM sessions ORDER BY started_at, id")]


# Hermes' end reasons after which a child is a new conversation the person
# sees (hermes_state_common.py `_RESET_END_REASONS`).
RESET_END_REASONS = {
    "session_reset",
    "session_switch",
    "idle",
    "daily",
    "suspended",
    "resume_pending_expired",
}


def marker(session, key):
    try:
        config = json.loads(session.get("model_config") or "{}")
    except (TypeError, ValueError):
        return None
    return config.get(key) if isinstance(config, dict) else None


def is_branch(session, parent):
    return bool(marker(session, "_branched_from")) or (
        parent.get("end_reason") == "branched"
        and (session.get("started_at") or 0) >= (parent.get("ended_at") or 0)
    )


def is_reset(session, parent):
    key = session.get("session_key")
    return bool(marker(session, "_reset_from")) or (
        parent.get("end_reason") in RESET_END_REASONS
        and bool(key)
        and key == parent.get("session_key")
    )


def is_continuation(session, parent):
    """Hermes' compression continuation (hermes_state.py, the compression
    tip): a child of a session that ended 'compression', never a branch,
    a delegate or a tool's run."""
    return (
        parent.get("end_reason") == "compression"
        and not marker(session, "_branched_from")
        and not marker(session, "_delegate_from")
        and session.get("source") != "tool"
    )


def continuation_rank(session):
    reason = session.get("end_reason")
    order = 0 if reason == "compression" else 1 if session.get("ended_at") is None else 2
    return (order, session.get("started_at") or 0)


def chains(sessions):
    """Conversations: each session with the ones Hermes continued it in
    when it compressed its context, in order."""
    by_id = {s["id"]: s for s in sessions}
    children = {}
    for session in sessions:
        parent = by_id.get(session.get("parent_session_id"))
        if parent and is_continuation(session, parent):
            children.setdefault(parent["id"], []).append(session)
    next_of = {pid: min(kids, key=continuation_rank)["id"] for pid, kids in children.items()}
    continued = set(next_of.values())
    result, covered = [], set()
    for session in sessions:
        if session["id"] in continued:
            continue
        chain, seen = [session], {session["id"]}
        while next_of.get(chain[-1]["id"]) and next_of[chain[-1]["id"]] not in seen:
            chain.append(by_id[next_of[chain[-1]["id"]]])
            seen.add(chain[-1]["id"])
        result.append(chain)
        covered.update(s["id"] for s in chain)
    # A loop of continuations (never written by Hermes) still shows.
    return result + [[s] for s in sessions if s["id"] not in covered]


def conversation_kind(chain, by_id):
    """`cron` for a scheduled job's run; `subagent` for a side task (a
    child Hermes hides from its pickers); else `chat`: a root, a branch,
    or the new conversation a reset starts."""
    root = chain[0]
    if root.get("source") == "cron":
        return "cron"
    parent = by_id.get(root.get("parent_session_id"))
    if parent is None or is_branch(root, parent) or is_reset(root, parent):
        return "chat"
    return "subagent"


def part_text(part):
    kind = str(part.get("type", ""))
    if kind in ("text", "input_text", "output_text"):
        return part.get("text") or ""
    for word in ("image", "audio", "video", "file"):
        if word in kind:
            return f"[{word}]"
    return ""


def message_text(content):
    if not content:
        return ""
    if isinstance(content, bytes):
        content = content.decode("utf-8", "replace")
    if content.lstrip().startswith("["):
        try:
            parts = json.loads(content)
        except ValueError:
            return content
        if (
            isinstance(parts, list)
            and parts
            and all(isinstance(p, dict) and "type" in p for p in parts)
        ):
            return "\n".join(t for t in (part_text(p) for p in parts) if t)
    return content


def tool_calls(raw):
    try:
        calls = json.loads(raw) if raw else []
    except ValueError:
        return []
    if not isinstance(calls, list):
        return []
    named = []
    for call in calls:
        function = call.get("function", {}) if isinstance(call, dict) else {}
        arguments = function.get("arguments") if isinstance(function, dict) else None
        named.append(
            {
                "name": (function.get("name") if isinstance(function, dict) else None) or "tool",
                "arguments": clip(
                    arguments if isinstance(arguments, str) else json.dumps(arguments),
                    ARGUMENTS_MAX,
                ),
            }
        )
    return named


def normalize_message(row):
    role = row.get("role") or "unknown"
    text = message_text(row.get("content"))
    message = {"role": role, "at": iso(row.get("timestamp"))}
    if text:
        message["text"] = clip(text, TOOL_TEXT_MAX) if role == "tool" else text
    if row.get("tool_name"):
        message["tool"] = row["tool_name"]
    calls = tool_calls(row.get("tool_calls"))
    if calls:
        message["tool_calls"] = calls
    reasoning = row.get("reasoning") or row.get("reasoning_content")
    if reasoning:
        message["reasoning"] = clip(str(reasoning), REASONING_MAX)
    if row.get("_compressed_summary"):
        message["summary"] = True
    return message


def superseded(row):
    """A row Hermes took back: undone, or the original of a tail it carried
    forward when it compacted in place (`active=0, compacted=0`). Rows it
    compacted away (`compacted=1`) are history, and stay."""
    return row.get("active") == 0 and not row.get("compacted")


def is_said(message):
    """A message a person wrote or the agent said to them."""
    return (
        message["role"] in ("user", "assistant")
        and message.get("text")
        and not message.get("summary")
        and not message.get("tool_calls")
    )


def conversation(chain, by_id, conn):
    messages = [
        normalize_message(dict(row))
        for session in chain
        for row in conn.execute(
            "SELECT * FROM messages WHERE session_id = ? ORDER BY timestamp, id", (session["id"],)
        )
        if not superseded(dict(row))
    ]
    said = [m for m in messages if is_said(m)]
    first_user = next((m["text"] for m in said if m["role"] == "user"), "")
    titled = next((s.get("title") for s in chain if s.get("title")), None)
    last = chain[-1]
    root = chain[0]
    kind = conversation_kind(chain, by_id)
    record = {
        "id": root["id"],
        "file": conversation_file(root, kind),
        "title": titled or (first_user.splitlines()[0][:80] if first_user else "Untitled"),
        "kind": kind,
        "source": root.get("source"),
        "with": root.get("display_name"),
        "model": last.get("model") or root.get("model"),
        "started_at": iso(root.get("started_at")),
        "ended_at": iso(last.get("ended_at") or last.get("last_activity_at"))
        or (messages[-1]["at"] if messages else None),
        "messages": len(said),
        "preview": first_user[:160],
        "sessions": [s["id"] for s in chain],
    }
    return record, messages


def conversation_file(root, kind):
    """A chat has a file of its own. Scheduled jobs' runs and side tasks,
    which can number thousands, share one file a month and kind."""
    if kind == "chat":
        return f"chats/sessions/{safe_name(root['id'])}.json"
    month = (iso(root.get("started_at")) or "unknown")[:7]
    return f"chats/runs/{kind}-{month}.json"


def write_conversations(stage, written):
    """Each file of `written` ({file: [(record, messages)]}): a chat's
    alone, or a month's runs keyed by their ids."""
    for file, entries in written.items():
        if file.startswith("chats/sessions/"):
            record, messages = entries[0]
            stage.write_json(
                file, {"format": CHAT_FORMAT, **record, "messages": messages}, compact=True
            )
        else:
            bundle = {
                record["id"]: {**record, "messages": messages} for record, messages in entries
            }
            stage.write_json(file, {"format": CHAT_FORMAT, "conversations": bundle}, compact=True)


def survey_chats(paths, scratch):
    conn = open_sessions(paths, scratch)
    if conn is None:
        return {"conversations": 0, "by_kind": {}, "messages": 0}
    try:
        sessions = load_sessions(conn)
        by_id = {s["id"]: s for s in sessions}
        kinds = [conversation_kind(c, by_id) for c in chains(sessions)]
        messages = conn.execute("SELECT COUNT(*) FROM messages").fetchone()[0]
    finally:
        conn.close()
    return {
        "conversations": len(kinds),
        "by_kind": {k: kinds.count(k) for k in sorted(set(kinds))},
        "messages": messages,
    }


def export_chats(paths, stage, scratch, agent_name):
    conn = open_sessions(paths, scratch)
    if conn is None:
        stage.problem("chats: no state.db in the Hermes home")
        return {"conversations": 0}
    try:
        sessions = load_sessions(conn)
        by_id = {s["id"]: s for s in sessions}
        records = []
        # One file at a time, so only one file's messages are in memory.
        by_file = {}
        for chain in chains(sessions):
            by_file.setdefault(
                conversation_file(chain[0], conversation_kind(chain, by_id)), []
            ).append(chain)
        for file, file_chains in sorted(by_file.items()):
            entries = [conversation(chain, by_id, conn) for chain in file_chains]
            write_conversations(stage, {file: entries})
            records.extend(record for record, _ in entries)
    finally:
        conn.close()
    newest_first = sorted(
        records, key=lambda r: r["ended_at"] or r["started_at"] or "", reverse=True
    )
    stage.write_json(
        "chats/index.json",
        {
            "format": CHAT_FORMAT,
            "agent": agent_name,
            "exported_at": now_iso(),
            "conversations": newest_first,
        },
    )
    kinds = [r["kind"] for r in records]
    return {
        "conversations": len(records),
        "by_kind": {k: kinds.count(k) for k in sorted(set(kinds))},
    }


# --- Skills -----------------------------------------------------------------


def dir_hash(directory, without_caches=False):
    """Hermes' own change hash of a skill (tools/skills_sync.py `_dir_hash`).
    `without_caches` leaves out `__pycache__`, as finite-skills' own
    parity check does, for comparing two copies of a managed skill."""
    hasher = hashlib.md5()
    try:
        for path in sorted(Path(directory).rglob("*")):
            if without_caches and "__pycache__" in path.parts:
                continue
            if path.is_file():
                hasher.update(str(path.relative_to(directory)).encode("utf-8"))
                hasher.update(path.read_bytes())
    except OSError:
        pass
    return hasher.hexdigest()


def skill_dirs(root):
    """Each skill under `root`: a directory with a SKILL.md that is not
    inside another skill (whose references/ may hold SKILL-like files)."""
    root = Path(root)
    found = []
    if not root.is_dir():
        return found
    for dirpath, dirnames, filenames in os.walk(root):
        here = Path(dirpath)
        dirnames[:] = sorted(d for d in dirnames if d not in SKIP_DIRS and not d.startswith("."))
        if "SKILL.md" in filenames and here != root:
            found.append(here.relative_to(root))
            dirnames[:] = []
    return found


def bundled_manifest(path):
    pairs = (line.split(":", 1) for line in read_text(path).splitlines() if ":" in line)
    return {name.strip(): digest.strip() for name, digest in pairs}


def hub_skills(path):
    lock = read_json(path, {}) or {}
    installed = lock.get("installed", {}) if isinstance(lock, dict) else {}
    return installed if isinstance(installed, dict) else {}


def agent_created(path):
    usage = read_json(path, {}) or {}
    if not isinstance(usage, dict):
        return set()
    return {
        name
        for name, record in usage.items()
        if isinstance(record, dict)
        and (record.get("created_by") == "agent" or record.get("agent_created") is True)
    }


# Hermes' own skills name their authors so; one an older Hermes shipped,
# and no longer does, is known by this alone (it is in no manifest). Its
# community skills say just "community" (nano-pdf, on Clanky).
UPSTREAM_AUTHORS = re.compile(
    r"Hermes Agent|Teknium|Nous Research|Orchestra Research|^\s*community\s*$", re.IGNORECASE
)


def upstream_skills(env):
    """Hermes' bundled and optional skills on this runtime, by name: from
    the variables its Nix wrapper sets, or beside its binary."""
    roots = [env.get("HERMES_BUNDLED_SKILLS"), env.get("HERMES_OPTIONAL_SKILLS")]
    hermes = shutil.which("hermes", path=env.get("PATH"))
    if hermes:
        share = Path(os.path.realpath(hermes)).parent.parent / "share" / "hermes-agent"
        roots += [str(share / "skills"), str(share / "optional-skills")]
    found = {}
    for root in dict.fromkeys(r for r in roots if r):
        for relative in skill_dirs(root):
            found.setdefault(relative.name, Path(root) / relative)
    return found


# The skills that come over: the ones the person or their agent made,
# which it would have made on the new Finite too. Every skill a platform
# shipped (Finite's managed ones, Hermes' own, its hub's, and edited copies
# of any of them) has the new Finite's own in its place: listed, no files.
OWN_SKILLS = {"custom", "agent-created"}

# What an agent's own text says about the old platform, which an import
# adapts; a skill that says none of it comes over as it is.
OLD_PLATFORM = re.compile(
    r"fsite|fbrain|finitebrain|finite\.site|/data/agent|/data/workspace|FINITE_[A-Z]|FINITECHAT"
    r"|hermes cron|cronjob|google_token|gws auth|managed-skills",
    re.IGNORECASE,
)


def count_matches(directory, pattern, skip_dir=skip_common):
    """How often `pattern` appears in the text files under `directory`."""
    total = 0
    for path, _ in walk_files(directory, skip_dir):
        try:
            if path.stat().st_size > SCAN_MAX_BYTES:
                continue
            data = path.read_bytes()
        except OSError:
            continue
        if b"\x00" not in data[:8192]:
            total += len(pattern.findall(data.decode("utf-8", "ignore")))
    return total


def skill_entry(base, relative, provenance, export_path, extra=None):
    meta = frontmatter(read_text(Path(base) / relative / "SKILL.md"))
    return {
        "name": relative.name,
        "path": export_path,
        "provenance": provenance,
        "comes_over": provenance in OWN_SKILLS,
        "description": meta.get("description"),
        "source": str(Path(base) / relative),
        **(extra or {}),
    }


def skill_provenance(name, skill_dir, known):
    if name in known["bundled"]:
        return None if dir_hash(skill_dir) == known["bundled"][name] else "modified-bundled"
    if name in known["managed"]:
        return "shadows-managed"
    if name in known["hub"]:
        return "hub"
    if name in known["made"]:
        return "agent-created"
    author = frontmatter(read_text(skill_dir / "SKILL.md")).get("author", "")
    if name in known["upstream"] or UPSTREAM_AUTHORS.search(author):
        return "upstream"
    return "custom"


def find_skills(paths, env=None):
    """The skills this agent's person or the agent added or changed. An
    untouched skill Hermes or Finite shipped is left out; one of Hermes'
    own (`upstream`) is listed, with its files only when they differ
    from this runtime's copy."""
    root = paths.hermes_home / "skills"
    known = {
        "bundled": bundled_manifest(root / ".bundled_manifest"),
        "managed": {rel.name: rel for rel in skill_dirs(paths.managed_skills)},
        "hub": hub_skills(root / ".hub" / "lock.json"),
        "made": agent_created(root / ".usage.json"),
        "upstream": upstream_skills(os.environ if env is None else env),
    }
    found = []
    for relative in skill_dirs(root):
        provenance = skill_provenance(relative.name, root / relative, known)
        if provenance is None:
            continue
        extra = {"hub": known["hub"][relative.name]} if provenance == "hub" else None
        found.append(skill_entry(root, relative, provenance, relative.as_posix(), extra))
    return found + edited_managed_skills(paths, known["managed"])


def edited_managed_skills(paths, managed):
    """Finite's managed skills edited in place, against the image's copy."""
    shipped = {rel.name: rel for rel in skill_dirs(paths.image_skills)}
    return [
        skill_entry(paths.managed_skills, relative, "modified-managed", f"managed-edits/{name}")
        for name, relative in sorted(managed.items())
        if name in shipped
        and dir_hash(paths.managed_skills / relative, without_caches=True)
        != dir_hash(paths.image_skills / shipped[name], without_caches=True)
    ]


def export_skills(paths, stage, max_bytes):
    """The person's and agent's own skills with their files, and how often
    each mentions the old platform; every other skill by name alone."""
    index = []
    for skill in find_skills(paths):
        if skill["comes_over"]:
            copied = stage.copy_tree(
                skill["source"], f"skills/files/{skill['path']}", skip_common, max_bytes
            )
            facts = {
                **copied,
                "exported": True,
                "old_platform_refs": count_matches(skill["source"], OLD_PLATFORM),
            }
        else:
            facts = {"files": 0, "bytes": 0, "exported": False}
        index.append({k: v for k, v in {**skill, **facts}.items() if k != "source"})
    stage.write_json("skills/index.json", {"skills": index})
    return {"skills": sum(1 for s in index if s["exported"]), "listed": len(index)}


# --- Cron --------------------------------------------------------------------


def load_jobs(paths):
    data = read_json(paths.hermes_home / "cron" / "jobs.json", {}) or {}
    jobs = data.get("jobs", []) if isinstance(data, dict) else data
    if isinstance(jobs, dict):
        jobs = [{"id": key, **value} for key, value in jobs.items() if isinstance(value, dict)]
    return [job for job in jobs if isinstance(job, dict)] if isinstance(jobs, list) else []


def last_output(paths, job_id):
    outputs = paths.hermes_home / "cron" / "output" / safe_name(str(job_id))
    files = sorted(p for p in outputs.glob("*") if p.is_file()) if outputs.is_dir() else []
    return clip(read_text(files[-1]), CRON_OUTPUT_MAX) if files else None


# What becomes of a job on the new Finite, worked out here so the import
# need not: its schedule in UTC (the new Finite's routines run in UTC) and
# the rule that applies. A routine fires at most hourly here; anything more
# often is an app's cron trigger, not a routine.
DOW_NAMES = {"sun": 0, "mon": 1, "tue": 2, "wed": 3, "thu": 4, "fri": 5, "sat": 6}
ROUTINE_TEXT_MAX = 3500  # the routine's record holds at most 4 KiB


def fires_more_than_hourly(expr):
    """Whether a cron expression fires more than once in an hour."""
    fields = expr.split()
    return len(fields) == 5 and not fields[0].isdigit()


def interval_cron(minutes):
    """The cron expression for an interval, or None: too often (under an
    hour), or no clean fit."""
    if minutes < 60:
        return None
    if minutes == 60:
        return "0 * * * *"
    if minutes == 1440:
        return "0 0 * * *"
    if minutes == 10080:
        return "0 0 * * 1"
    if minutes % 60 == 0 and 24 % (minutes // 60) == 0:
        return f"0 */{minutes // 60} * * *"
    return None


def dow_days(field):
    """A day-of-week field as a set of days (0 Sunday), or None."""
    days = set()
    for item in field.lower().split(","):
        ends = [DOW_NAMES.get(part, part) for part in item.split("-")]
        try:
            numbers = [int(end) % 7 for end in ends]  # 7 is Sunday too
        except ValueError:
            return None
        if len(numbers) == 1:
            days.add(numbers[0])
        elif len(numbers) == 2 and numbers[0] <= numbers[1]:
            days.update(range(numbers[0], numbers[1] + 1))
        else:
            return None
    return days


def dow_field(days):
    """Days as a field: runs of three or more as ranges, else a list."""
    ordered, parts, start = sorted(days), [], None
    for index, day in enumerate(ordered):
        start = day if start is None else start
        if index + 1 == len(ordered) or ordered[index + 1] != day + 1:
            parts.extend(
                [f"{start}-{day}"] if day - start >= 2 else [str(d) for d in range(start, day + 1)]
            )
            start = None
    return ",".join(parts)


def shift_days(dom, month, dow, shift):
    """The day fields after a move of `shift` days (-1, 0, 1), or None
    when it crosses a date (a day of the month moves across months)."""
    if shift == 0:
        return dom, month, dow
    if dom != "*" or month != "*":
        return None
    if dow == "*":
        return dom, month, dow
    days = dow_days(dow)
    return None if days is None else (dom, month, dow_field({(d + shift) % 7 for d in days}))


def cron_to_utc(expr, offset_minutes):
    """A 5-field cron expression read in a zone `offset_minutes` east of
    UTC, written in UTC; None when it can't be said simply."""
    fields = expr.split()
    if len(fields) != 5:
        return None
    if offset_minutes == 0:
        return expr
    minute, hour, dom, month, dow = fields
    if hour == "*":
        if offset_minutes % 60 == 0:
            return expr
        return (
            f"{(int(minute) - offset_minutes) % 60} * {dom} {month} {dow}"
            if minute.isdigit()
            else None
        )
    if not all(h.isdigit() for h in hour.split(",")):
        return None
    if not minute.isdigit() and offset_minutes % 60:
        return None
    base = int(minute) if minute.isdigit() else 0
    moved = [int(h) * 60 + base - offset_minutes for h in hour.split(",")]
    shifts = {m // 1440 for m in moved}
    if len(shifts) != 1:
        return None
    days = shift_days(dom, month, dow, shifts.pop())
    if days is None:
        return None
    new_minute = str(moved[0] % 60) if minute.isdigit() else minute
    new_hours = ",".join(str(h) for h in sorted({(m % 1440) // 60 for m in moved}))
    return " ".join([new_minute, new_hours, *days])


def zone_offsets(name):
    """(minutes east of UTC now, whether the zone moves its clocks), or
    (None, None) when this computer can't tell."""
    if name in (None, "", "UTC", "Etc/UTC", "GMT"):
        return 0, False
    try:
        from zoneinfo import ZoneInfo

        zone = ZoneInfo(name)
    except Exception:
        return None, None
    now = datetime.datetime.now(zone)
    january, july = (datetime.datetime(now.year, m, 1, 12, tzinfo=zone).utcoffset() for m in (1, 7))
    return int(now.utcoffset().total_seconds() // 60), january != july


def once_rule(schedule):
    try:
        when = datetime.datetime.fromisoformat(str(schedule.get("run_at")))
    except ValueError:
        return "convert_by_hand", None, "its one time to run couldn't be read"
    future = when.timestamp() > datetime.datetime.now(datetime.UTC).timestamp()
    if future:
        return (
            "once_future",
            None,
            f"runs once, at {when.isoformat()}: ask your owner nearer the time",
        )
    return "once_past", None, "ran once, already"


def job_rule(job, offset):
    """(suggestion, its UTC schedule, why) for one job."""
    schedule = job.get("schedule") or {}
    kind = schedule.get("kind")
    if job.get("enabled") is False or job.get("state") == "paused":
        return "paused", None, "it was paused"
    if job.get("no_agent"):
        return (
            "no_agent",
            None,
            "its script's output was the whole message, with no agent: an app's cron trigger fits it",
        )
    if kind == "once":
        return once_rule(schedule)
    if kind == "interval":
        minutes = int(schedule.get("minutes") or 0)
        if minutes < 60:
            return "too_frequent", None, f"every {minutes} minutes: more than hourly"
        cron = interval_cron(minutes)
        return (
            ("routine", cron, f"every {minutes} minutes")
            if cron
            else ("convert_by_hand", None, f"every {minutes} minutes fits no cron")
        )
    if kind == "cron":
        expr = str(schedule.get("expr") or "")
        if fires_more_than_hourly(expr):
            return "too_frequent", None, f"{expr} fires more than hourly"
        utc = cron_to_utc(expr, offset) if offset is not None else None
        return (
            ("routine", utc, "converted to UTC")
            if utc
            else ("convert_by_hand", None, f"{expr} can't be moved to UTC simply")
        )
    return "convert_by_hand", None, "an unknown kind of schedule"


def normalize_job(paths, job, zone):
    keys = (
        "id",
        "name",
        "prompt",
        "schedule",
        "enabled",
        "state",
        "deliver",
        "script",
        "no_agent",
        "model",
        "workdir",
        "repeat",
        "last_run_at",
        "last_status",
        "context_from",
        "enabled_toolsets",
    )
    skills = job.get("skills") or ([job["skill"]] if job.get("skill") else [])
    offset, dst = zone_offsets(zone)
    suggestion, utc, why = job_rule(job, offset)
    facts = {
        "timezone": zone or "UTC",
        "dst": dst,
        "schedule_utc": utc,
        "suggestion": suggestion,
        "why": why,
        "text_too_long": len(str(job.get("prompt") or "").encode("utf-8")) > ROUTINE_TEXT_MAX,
    }
    return {
        **{key: job.get(key) for key in keys},
        "skills": skills,
        **facts,
        "last_output": last_output(paths, job.get("id")),
    }


def survey_cron(paths):
    return [
        {
            "name": job.get("name") or job.get("id"),
            "schedule": (job.get("schedule") or {}).get("display"),
            "enabled": job.get("enabled", True),
        }
        for job in load_jobs(paths)
    ]


def job_zone(paths, env):
    """The zone Hermes ran its jobs in: config.yaml's `timezone`, else
    `HERMES_TIMEZONE`, else UTC."""
    return (
        config_settings(read_text(paths.hermes_home / "config.yaml")).get("timezone")
        or env.get("HERMES_TIMEZONE")
        or None
    )


def export_cron(paths, stage, max_bytes):
    zone = job_zone(paths, os.environ)
    jobs = [normalize_job(paths, job, zone) for job in load_jobs(paths)]
    raw = paths.hermes_home / "cron" / "jobs.json"
    if raw.is_file():
        stage.copy(raw, "cron/jobs.json")
    scripts = paths.hermes_home / "scripts"
    if scripts.is_dir():
        stage.copy_tree(scripts, "cron/scripts", skip_common, max_bytes)
    stage.write_json("cron/index.json", {"jobs": jobs})
    return {"jobs": len(jobs)}


# --- Brains ------------------------------------------------------------------


def skip_brain_dir(relative, _absolute):
    return relative.name in SKIP_DIRS or relative.name == "_wiki" or relative.name.startswith(".")


def list_brains():
    listed, error = run_json(
        ["fbrain", "brain", "list", "--json", "--existing-identity"], timeout=120
    )
    if listed is None:
        return [], error
    brains = listed.get("brains", []) if isinstance(listed, dict) else []
    return [b for b in brains if isinstance(b, dict) and b.get("brainId")], ""


def brain_size(tree):
    files = [p for p, _ in walk_files(tree, skip_brain_dir) if p.name not in BRAIN_GENERATED]
    return {"files": len(files), "bytes": sum(p.stat().st_size for p in files)}


def survey_brains(paths):
    brains, error = list_brains()
    rows = []
    for brain in brains:
        tree = paths.brain_root / brain["brainId"]
        local = tree.is_dir()
        invited = brain.get("role") == "invited"
        rows.append(
            {
                "id": brain["brainId"],
                "name": brain.get("name"),
                "kind": brain.get("kind"),
                "role": brain.get("role"),
                "can_export": not invited,
                **({"why_not": "an invitation this agent hasn't accepted"} if invited else {}),
                "local_copy": local,
                **(brain_size(tree) if local else {"files": None, "bytes": None}),
            }
        )
    return rows, error


def open_brain(paths, brain_id):
    """Open the brain's readable working copy and sync it now. Any sign
    that the copy is not current comes back as a warning."""
    opened, error = run_json(["fbrain", "open", brain_id, "--json"], timeout=600)
    if opened is None:
        return None, None, error, []
    tree = Path(
        opened.get("nextCommandWorkingDirectory")
        or opened.get("path")
        or paths.brain_root / brain_id
    )
    synced, sync_error = run_json(["fbrain", "sync", "now", "--json"], cwd=str(tree), timeout=600)
    warnings = []
    status = str(opened.get("syncStatus") or "")
    if "block" in status or "error" in status or "fail" in status:
        warnings.append(f"its sync is {status}")
    if synced is None:
        warnings.append(f"syncing it now failed: {sync_error}")
    elif isinstance(synced, dict) and synced.get("conflicts"):
        warnings.append(f"it has {len(synced['conflicts'])} sync conflicts")
    return tree, synced if synced is not None else {"error": sync_error}, "", warnings


def asset_path(resource):
    if resource.startswith("file://"):
        return Path(urllib.parse.unquote(urllib.parse.urlparse(resource).path))
    if resource.startswith("/"):
        return Path(resource)
    return None


def within(path, roots):
    real = os.path.realpath(path)
    return any(
        real == os.path.realpath(r) or real.startswith(os.path.realpath(r) + os.sep) for r in roots
    )


def asset_refusal(local, allowed, private, max_bytes):
    """Why an asset's local file is not copied, or None. Only a regular
    file inside the workspace, never one of the agent's own homes, and
    never one that looks like a credential: a brain note anyone in an
    organization could write must not export the agent's keys."""
    if not local.is_file():
        return "not on this computer"
    if local.is_symlink() or not within(local, allowed) or within(local, private):
        return "outside the workspace: left as a link"
    return (
        secret_reason(local.name)
        or too_large(local, max_bytes)
        or ("looks like it holds a secret" if secret_in_text(local) else None)
    )


def copy_asset(note, resource, prefix, stage, limits):
    local = asset_path(resource)
    base = {"note": note, "resource": resource, "copied": False}
    if local is None:
        return {**base, "reason": "a web address: left as a link"}
    reason = asset_refusal(local, limits["allowed"], limits["private"], limits["max_bytes"])
    if reason:
        return {**base, "reason": reason}
    digest = sha256_file(local)
    name = f"assets/{digest[:12]}-{safe_name(local.name)}"
    stage.copy(local, f"{prefix}/{name}")
    content_type = mimetypes.guess_type(local.name)[0] or "application/octet-stream"
    return {
        **base,
        "copied": True,
        "file": name,
        "bytes": local.stat().st_size,
        "sha256": digest,
        "content_type": content_type,
    }


def brain_assets(tree, prefix, stage, limits):
    """Asset source notes (frontmatter `resource:`) whose bytes are in the
    workspace get their bytes copied; the rest are listed as links."""
    assets = []
    for path, relative in walk_files(tree, skip_brain_dir):
        if path.suffix != ".md" or path.name in BRAIN_GENERATED:
            continue
        resource = frontmatter(read_text(path)).get("resource")
        if resource:
            assets.append(copy_asset(relative.as_posix(), resource, prefix, stage, limits))
    return assets


def export_brain(paths, brain, stage, max_bytes):
    brain_id = brain["brainId"]
    label = brain.get("name") or brain_id
    prefix = f"brains/{safe_name(brain_id)}"
    tree, synced, error, warnings = open_brain(paths, brain_id)
    if tree is None or not tree.is_dir():
        stage.problem(f"brain {label}: could not open its working copy: {error}")
        return None
    copied = stage.copy_tree(
        tree,
        f"{prefix}/files",
        skip_brain_dir,
        max_bytes,
        silent=lambda name: name in BRAIN_GENERATED,
    )
    if not copied["files"]:
        warnings.append("its working copy has no files")
    for warning in warnings:
        stage.problem(f"brain {label}: {warning}; run `fbrain doctor`")
    folders, _ = run_json(["fbrain", "folder", "list", "--brain", brain_id, "--json"], timeout=120)
    limits = {
        "allowed": [paths.workspace],
        "private": [paths.finite_home, paths.hermes_home],
        "max_bytes": max_bytes,
    }
    meta = {
        "id": brain_id,
        "name": brain.get("name"),
        "kind": brain.get("kind"),
        "role": brain.get("role"),
        "sync": synced,
        "folders": folders,
        "assets": brain_assets(tree, prefix, stage, limits),
        **copied,
    }
    stage.write_json(f"{prefix}/meta.json", meta)
    return {"id": brain_id, "name": brain.get("name"), "kind": brain.get("kind"), **copied}


def export_brains(paths, chosen, stage, max_bytes):
    if not chosen:
        return []
    brains, error = list_brains()
    if error:
        stage.problem(f"brains: {error}")
    known = {b["brainId"]: b for b in brains if b.get("role") != "invited"}
    for missing in sorted(set(chosen) - set(known)):
        stage.problem(f"brain {missing}: not a brain this agent can read")
    exported = [export_brain(paths, known[i], stage, max_bytes) for i in chosen if i in known]
    return [e for e in exported if e]


# --- Sites --------------------------------------------------------------------


def list_projects():
    listed, error = run_json(
        ["fsite", "project", "list", "--output", "json", "--existing-identity"], timeout=120
    )
    if listed is None:
        return [], error
    projects = listed.get("projects", []) if isinstance(listed, dict) else []
    return [p for p in projects if isinstance(p, dict) and p.get("slug")], ""


def site_decision(project):
    """yes: the agent owns it; ask: it may only edit it; no: no website."""
    if not project.get("site"):
        return "no"
    return "yes" if project.get("role") == "owner" else "ask"


def survey_sites():
    projects, error = list_projects()
    rows = [
        {
            "slug": p["slug"],
            "role": p.get("role"),
            "export": site_decision(p),
            "url": (p.get("site") or {}).get("url"),
            "visibility": (p.get("site") or {}).get("visibility"),
        }
        for p in projects
    ]
    return rows, error


def skip_site_dir(relative, _absolute):
    return relative.name in {".git", "node_modules", "__pycache__", ".venv", ".cache"}


def shallow_clone(remote, branch, dest):
    out, error = run(
        ["git", "clone", "--depth", "1", "--single-branch", "--branch", branch, remote, str(dest)],
        timeout=300,
    )
    if out is None:
        shutil.rmtree(dest, ignore_errors=True)
        out, error = run(["git", "clone", "--depth", "1", remote, str(dest)], timeout=300)
    if out is None:
        shutil.rmtree(dest, ignore_errors=True)
    return out is not None, error


def clone_site(project, scratch):
    """The site's files at the head of its deploy branch, without history:
    with the git credential the agent already keeps, and only if that
    fails, a new one from `fsite auth git` (as the Sites skill does)."""
    slug_name = project["slug"]
    branch = (project.get("site") or {}).get("branch") or "main"
    dest = Path(scratch) / "sites" / safe_name(slug_name)
    remote = project.get("git_remote_url")
    if remote:
        cloned, error = shallow_clone(remote, branch, dest)
        if cloned:
            return dest, ""
    auth, auth_error = run_json(
        ["fsite", "auth", "git", slug_name, "--store", "--output", "json"], timeout=120
    )
    remote = (auth or {}).get("git_remote_url") or remote
    if not remote:
        return None, auth_error or "no git remote"
    cloned, error = shallow_clone(remote, branch, dest)
    return (dest, "") if cloned else (None, error)


def site_meta(project):
    site = project.get("site") or {}
    return {
        "slug": project["slug"],
        "project_id": project.get("project_id"),
        "role": project.get("role"),
        "name": site.get("name"),
        "url": site.get("url"),
        "visibility": "public" if site.get("visibility") == "public" else "private",
        "old_visibility": site.get("visibility"),
        "branch": site.get("branch"),
        "path": site.get("path") or ".",
        "spa": bool(site.get("spa")),
        "active_version": site.get("active_version"),
    }


# What in a site's served files only worked on Finite Sites (its old
# address aside, which site_attention counts on its own).
SITE_OLD_PLATFORM = re.compile(
    r"fsite|fbrain|finitebrain|finite\.site/api|/data/(?:agent|workspace)|FINITECHAT", re.IGNORECASE
)


# The new Finite serves a site's files by their extension, with `nosniff`
# on every answer, so a browser runs a script or applies a stylesheet only
# from a file named as one (crates/core/src/site.rs, mime_for_path, in
# fragment). Finite Sites sent no `nosniff`, so `<script src="data.json">`
# ran there and is refused here.
CODE_EXTENSIONS = {"script": {"js", "mjs", "cjs"}, "stylesheet": {"css"}}
CODE_TAG = re.compile(r"<(script|link)\b([^>]*)>", re.IGNORECASE)
TAG_ATTR = re.compile(r"""([a-zA-Z-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))""")
ELSEWHERE = re.compile(r"^(?:[a-z][a-z0-9+.-]*:|//)", re.IGNORECASE)


def code_load(tag, attrs):
    """What a <script> or <link> tag loads as code, as (kind, src), or None."""
    kind = attrs.get("type", "").strip().lower()
    if tag == "script" and (kind in ("", "module") or "javascript" in kind or "ecmascript" in kind):
        return "script", attrs.get("src")
    if tag == "link" and "stylesheet" in attrs.get("rel", "").lower().split():
        return "stylesheet", attrs.get("href")
    return None


def wrong_type_code(html):
    """The scripts and stylesheets a page loads from its own site under a
    name the new Finite won't serve as code, in the page's order."""
    found = []
    for match in CODE_TAG.finditer(html):
        attrs = {
            m.group(1).lower(): next(v for v in m.groups()[1:] if v is not None)
            for m in TAG_ATTR.finditer(match.group(2))
        }
        load = code_load(match.group(1).lower(), attrs)
        if load is None or not load[1] or ELSEWHERE.match(load[1]):
            continue
        name = re.split(r"[?#]", load[1])[0].rsplit("/", 1)[-1]
        extension = name.rsplit(".", 1)[1].lower() if "." in name else ""
        if extension not in CODE_EXTENSIONS[load[0]]:
            found.append({"tag": load[0], "src": load[1]})
    return found


def site_wrong_type_code(served):
    found = []
    for path, relative in walk_files(served, skip_site_dir):
        if path.suffix.lower() not in (".html", ".htm"):
            continue
        try:
            if path.stat().st_size > SCAN_MAX_BYTES:
                continue
        except OSError:
            continue
        found += [
            {"page": relative.as_posix(), **load} for load in wrong_type_code(read_text(path))
        ]
    return found


def wrong_type_line(loads):
    names = sorted({load["src"] for load in loads})
    shown = ", ".join(names[:3]) + (f" and {len(names) - 3} more" if len(names) > 3 else "")
    return f"loads {shown} as code under a name a browser won't run here"


def site_attention(source, meta):
    """Facts about a site's served folder, and what in it needs an
    agent's eye: everything else moves as it is."""
    served = source / meta["path"]
    host = urllib.parse.urlparse(meta.get("url") or "").netloc
    self_links = (
        count_matches(
            served, re.compile(rf"https?://{re.escape(host)}", re.IGNORECASE), skip_site_dir
        )
        if host
        else 0
    )
    old_refs = count_matches(served, SITE_OLD_PLATFORM, skip_site_dir)
    has_index = (served / "index.html").is_file()
    wrong_type = site_wrong_type_code(served)
    attention = (
        (["a single-page app (spa = true)"] if meta.get("spa") else [])
        + (
            [f"{self_links} link{'s' if self_links != 1 else ''} to its old address"]
            if self_links
            else []
        )
        + ([f"mentions the old platform ({old_refs} times)"] if old_refs else [])
        + ([] if has_index else ["no index.html in its site folder"])
        + ([wrong_type_line(wrong_type)] if wrong_type else [])
    )
    return {
        "self_links": self_links,
        "old_platform_refs": old_refs,
        "has_index": has_index,
        "wrong_type_code": wrong_type,
        "attention": attention,
    }


def export_sites(chosen, left_out, stage, scratch, max_bytes):
    projects, error = list_projects()
    if error:
        stage.problem(f"sites: {error}")
    for slug in sorted(set(left_out)):
        stage.skip(f"sites/{safe_name(slug)}", "left out by the person's choice")
    wanted = [
        p
        for p in projects
        if p["slug"] not in left_out
        and (site_decision(p) == "yes" or (site_decision(p) == "ask" and p["slug"] in chosen))
    ]
    exported = []
    for project in sorted(wanted, key=lambda p: p["slug"]):
        prefix = f"sites/{safe_name(project['slug'])}"
        source, why = clone_site(project, scratch)
        if source is None:
            stage.problem(f"site {project['slug']}: could not get its files: {why}")
            continue
        copied = stage.copy_tree(source, f"{prefix}/source", skip_site_dir, max_bytes)
        meta = site_meta(project)
        if not (source / meta["path"]).exists():
            stage.problem(
                f"site {project['slug']}: its site folder {meta['path']!r} is not in its files"
            )
        stage.write_json(f"{prefix}/meta.json", {**meta, **copied, **site_attention(source, meta)})
        exported.append(project["slug"])
    return exported


# --- The workspace -------------------------------------------------------------


def is_site_checkout(directory):
    """A git checkout of a Finite Sites project: exported as a site when
    it is one, and left out when the person said no to it."""
    config = directory / ".git" / "config"
    return config.is_file() and "finite.site" in read_text(config)


def beside_project_file(folder):
    try:
        with os.scandir(folder) as entries:
            return any(
                e.is_file() and (e.name in PROJECT_FILES or e.name.endswith(PROJECT_SUFFIXES))
                for e in entries
            )
    except OSError:
        return False


def is_build_output(relative, absolute):
    """A folder a tool made: dependencies anywhere, and a build folder
    beside the project file that builds it."""
    name = relative.name
    if name in SKIP_DIRS and not name.startswith("."):
        return True
    return name in BUILD_DIRS and beside_project_file(absolute.parent)


def folder_summary(folders):
    return {"folders": len(folders), "examples": sorted(folders)[:KIND_EXAMPLES]}


def kind_summary(files):
    largest = sorted(files, key=lambda f: (-f["bytes"], f["path"]))[:KIND_EXAMPLES]
    return {"files": len(files), "bytes": sum(f["bytes"] for f in files), "largest": largest}


def is_media_or_binary(path):
    return path.suffix.lower() in MEDIA_AND_BINARIES


def workspace_skipper(paths, out_dir, on_build_output=lambda relative: None):
    left_out = {Path(p).resolve() for p in (paths.brain_root, out_dir)}

    def skip(relative, absolute):
        if is_build_output(relative, absolute):
            on_build_output(relative.as_posix())
            return True
        return (
            relative.name in SKIP_DIRS
            or relative.name.startswith(".")
            or absolute.resolve() in left_out
            or (absolute / ".finitebrain").is_dir()  # a brain's working copy opened elsewhere
            or is_site_checkout(absolute)
        )

    return skip


def survey_workspace(paths, out_dir, max_bytes):
    """What of the workspace would go in, and what is left out by kind
    without asking: build output, media and binaries, files too large."""
    build_output = []
    skip = workspace_skipper(paths, out_dir, build_output.append)
    found = list(walk_files(paths.workspace, skip)) if paths.workspace.is_dir() else []
    files = [(p, {"path": rel.as_posix(), "bytes": p.stat().st_size}) for p, rel in found]
    media = [f for p, f in files if is_media_or_binary(p)]
    rest = [f for p, f in files if not is_media_or_binary(p) and not secret_reason(p.name)]
    large = sorted((f for f in rest if f["bytes"] > max_bytes), key=lambda f: -f["bytes"])
    going = [f["bytes"] for f in rest if f["bytes"] <= max_bytes]
    return {
        "files": len(going),
        "bytes": sum(going),
        "bytes_under_1mb": sum(b for b in going if b <= MiB),
        "ask": sum(going) > WORKSPACE_ASK_BYTES,
        "too_large": large[:KIND_EXAMPLES],
        "too_large_count": len(large),
        "too_large_bytes": sum(f["bytes"] for f in large),
        "build_output": folder_summary(build_output),
        "media_and_binaries": kind_summary(media),
    }


def export_workspace(paths, stage, out_dir, max_bytes):
    if not paths.workspace.is_dir():
        return {"files": 0, "bytes": 0}
    build_output, media = [], []

    def set_aside(path, target):
        if not is_media_or_binary(path):
            return False
        media.append({"path": target, "bytes": path.stat().st_size})
        return True

    skip = workspace_skipper(
        paths, out_dir, lambda relative: build_output.append(f"workspace/{relative}")
    )
    copied = stage.copy_tree(paths.workspace, "workspace", skip, max_bytes, set_aside=set_aside)
    if build_output:
        stage.skip_kind(
            "workspace",
            "build_output",
            "build output and dependencies: made again from their projects' source",
            folder_summary(build_output),
        )
    if media:
        stage.skip_kind(
            "workspace",
            "media_and_binaries",
            "video, audio, archives and other binaries: left out by kind",
            kind_summary(media),
        )
    return copied


# --- Survey and build ----------------------------------------------------------


def guarded(stage, name, step, default):
    """Run one section; a failure is noted and the export goes on."""
    try:
        return step()
    except Exception as error:
        stage.problem(f"{name}: {type(error).__name__}: {error}")
        return default


def survey(paths, env, out_dir, max_bytes):
    notes = Stage(tempfile.mkdtemp(prefix="finite-survey-"))
    try:
        brains, brain_error = guarded(notes, "brains", lambda: survey_brains(paths), ([], ""))
        sites, site_error = guarded(notes, "sites", survey_sites, ([], ""))
        found = {
            "format": FORMAT,
            "agent": guarded(notes, "agent", lambda: agent_profile(paths, env), {}),
            "chats": guarded(notes, "chats", lambda: survey_chats(paths, notes.root), {}),
            "memories": [p.name for p in memory_files(paths, max_bytes)],
            "skills": [
                {k: s[k] for k in ("name", "provenance", "comes_over", "description")}
                for s in guarded(notes, "skills", lambda: find_skills(paths), [])
            ],
            "cron": guarded(notes, "cron", lambda: survey_cron(paths), []),
            "brains": brains,
            "sites": sites,
            "workspace": guarded(
                notes, "workspace", lambda: survey_workspace(paths, out_dir, max_bytes), {}
            ),
            "integrations": guarded(notes, "integrations", lambda: integrations(paths), {}),
        }
        problems = notes.problems + [e for e in (brain_error, site_error) if e]
        return {**found, "notes": survey_notes(paths), "problems": problems}
    finally:
        shutil.rmtree(notes.root, ignore_errors=True)


def survey_notes(paths):
    notes = []
    if paths.legacy.is_dir():
        notes.append(
            f"{paths.legacy} holds what an earlier Finite migration kept; it is not exported."
        )
    if (paths.hermes_home / "memory_store.db").is_file():
        notes.append(
            "This agent has a memory store (memory_store.db); it is exported with the memories."
        )
    return notes


def readme(profile, counts, choices):
    lines = [
        f"# {profile.get('name', 'Agent')}, exported from the old Finite",
        "",
        f"Made {now_iso()} by export.py ({FORMAT}). manifest.json lists every file and its SHA-256,",
        "what was left out and why. No password, key or token is in this zip:",
        "the new agent signs in to everything again.",
        "",
        f"- Conversations: {counts.get('chats', {}).get('conversations', 0)}",
        f"- Skills: {counts.get('skills', {}).get('skills', 0)}",
        f"- Scheduled jobs: {counts.get('cron', {}).get('jobs', 0)}",
        f"- Brains: {', '.join(b['name'] or b['id'] for b in counts.get('brains', [])) or 'none (by choice)'}",
        f"- Sites: {', '.join(choices['sites']) or 'none'}",
        f"- Workspace files: {counts.get('workspace', {}).get('files', 0) if choices['workspace'] else 'not included'}",
    ]
    return "\n".join(lines) + "\n"


def write_manifest(stage, profile, counts, choices):
    files = sorted(p for p in stage.root.rglob("*") if p.is_file())
    listed = [
        {
            "path": p.relative_to(stage.root).as_posix(),
            "bytes": p.stat().st_size,
            "sha256": sha256_file(p),
        }
        for p in files
    ]
    stage.write_json(
        "manifest.json",
        {
            "format": FORMAT,
            "created_at": now_iso(),
            "source": {"platform": "finite-mono", "hermes_version": profile.get("hermes_version")},
            "agent": {"name": profile.get("name")},
            "counts": counts,
            "choices": choices,
            "skipped": stage.skipped,
            "problems": stage.problems,
            "files": listed,
        },
    )


def make_zip(stage, target):
    target.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in sorted(p for p in stage.root.rglob("*") if p.is_file()):
            archive.write(path, path.relative_to(stage.root).as_posix())
    return target


def largest_parts(target, limit=8):
    totals = {}
    with zipfile.ZipFile(target) as archive:
        for info in archive.infolist():
            part = "/".join(info.filename.split("/")[:2])
            totals[part] = totals.get(part, 0) + info.compress_size
    ranked = sorted(totals.items(), key=lambda item: -item[1])[:limit]
    return [{"part": part, "bytes": size} for part, size in ranked]


def build(paths, env, options):
    max_bytes = options.max_file_mb * MiB
    out_dir = Path(options.out) if options.out else paths.workspace / "finite-export"
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "last-build.json").unlink(missing_ok=True)
    scratch = Path(tempfile.mkdtemp(prefix="finite-export-"))
    stage = Stage(scratch / "zip")
    try:
        profile = guarded(
            stage, "agent", lambda: export_agent(paths, env, stage), {"name": "Finite Agent"}
        )
        counts = {
            "chats": guarded(
                stage, "chats", lambda: export_chats(paths, stage, scratch, profile.get("name")), {}
            ),
            "memories": guarded(
                stage, "memories", lambda: export_memories(paths, stage, max_bytes), {}
            ),
            "skills": guarded(stage, "skills", lambda: export_skills(paths, stage, max_bytes), {}),
            "cron": guarded(stage, "cron", lambda: export_cron(paths, stage, max_bytes), {}),
            "brains": guarded(
                stage, "brains", lambda: export_brains(paths, options.brain, stage, max_bytes), []
            ),
        }
        sites = guarded(
            stage,
            "sites",
            lambda: export_sites(options.site, options.leave_out_site, stage, scratch, max_bytes),
            [],
        )
        if not options.no_workspace:
            counts = {
                **counts,
                "workspace": guarded(
                    stage,
                    "workspace",
                    lambda: export_workspace(paths, stage, out_dir, max_bytes),
                    {},
                ),
            }
        stage.write_json(
            "integrations.json", guarded(stage, "integrations", lambda: integrations(paths), {})
        )
        choices = {
            "brains": list(options.brain),
            "sites": sites,
            "sites_left_out": sorted(set(options.leave_out_site)),
            "workspace": not options.no_workspace,
            "max_file_mb": options.max_file_mb,
        }
        stage.write_text("README.md", readme(profile, counts, choices))
        write_manifest(stage, profile, counts, choices)
        stamp = datetime.datetime.now(datetime.UTC).strftime("%Y%m%d-%H%M")
        target = make_zip(
            stage, out_dir / f"finite-export-{slug(profile.get('name') or 'agent')}-{stamp}.zip"
        )
        size = target.stat().st_size
        return {
            "zip": str(target),
            "bytes": size,
            "limit_bytes": LIMIT_BYTES,
            "fits": size <= LIMIT_BYTES,
            "send_line": f"MEDIA:{target}",
            "largest": largest_parts(target),
            "counts": counts,
            "choices": choices,
            "skipped": len(stage.skipped),
            "problems": stage.problems,
            "agent": {"name": profile.get("name"), "hermes_version": profile.get("hermes_version")},
        }
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


# --- Reporting ----------------------------------------------------------------


def script_version():
    try:
        return hashlib.sha256(Path(__file__).read_bytes()).hexdigest()[:12]
    except OSError:
        return "unknown"


def agent_identity(paths, env, name):
    """Who ran the export, by what the old platform knows it as: the
    runner's agent id, the agent's public key (`fbrain auth status`,
    which never makes one) and its owners' account ids. No secret."""
    status, _ = run_json(["fbrain", "auth", "status", "--json"], timeout=30)
    npub = status.get("npub") if isinstance(status, dict) else None
    owners = [
        line.strip()[:128]
        for line in read_text(paths.finite_home / "allowed-users").splitlines()
        if line.strip()
    ]
    identity = {
        "agent_id": (env.get("FINITE_AGENT_ID") or "")[:128],
        "agent_npub": (npub or "")[:80],
        "agent_name": (name or "")[:120],
        "owners": owners[:8],
    }
    return {key: value for key, value in identity.items() if value}


def survey_counts(found):
    chats = found.get("chats") or {}
    return {
        "conversations": chats.get("conversations", 0),
        "chats": (chats.get("by_kind") or {}).get("chat", 0),
        "memories": len(found.get("memories") or []),
        "skills": sum(1 for skill in found.get("skills") or [] if skill.get("comes_over")),
        "cron": len(found.get("cron") or []),
        "brains_offered": sum(1 for b in found.get("brains") or [] if b.get("can_export")),
        "sites_offered": sum(
            1 for s in found.get("sites") or [] if s.get("export") in ("yes", "ask")
        ),
        "workspace_files": (found.get("workspace") or {}).get("files", 0),
        "problems": len(found.get("problems") or []),
    }


def build_counts(result):
    counts = result.get("counts") or {}
    chats = counts.get("chats") or {}
    return {
        "conversations": chats.get("conversations", 0),
        "chats": (chats.get("by_kind") or {}).get("chat", 0),
        "memories": len((counts.get("memories") or {}).get("files", [])),
        "skills": (counts.get("skills") or {}).get("skills", 0),
        "cron": (counts.get("cron") or {}).get("jobs", 0),
        "brains": len(counts.get("brains") or []),
        "sites": len((result.get("choices") or {}).get("sites", [])),
        "workspace_files": (counts.get("workspace") or {}).get("files", 0),
        "bytes": result.get("bytes", 0),
        "fits": bool(result.get("fits")),
        "problems": len(result.get("problems") or []),
    }


def send_report(url, event, identity, counts, hermes):
    """Tell the move site this run happened. Best effort: an answer it
    can't get is noted in the result, never a failure of the export."""
    if not url:
        return "off"
    report = {
        "event": event,
        "run": RUN_ID,
        **identity,
        "hermes": (hermes or "")[:120],
        "script": script_version(),
        "counts": counts,
    }
    body = {
        "id": uuid.uuid4().hex,
        "input": {key: value for key, value in report.items() if value not in (None, "", [], {})},
    }
    request = urllib.request.Request(
        url,
        data=json.dumps(body).encode("utf-8"),
        method="POST",
        headers={
            "content-type": "application/json",
            "user-agent": f"finite-export/{script_version()}",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=REPORT_TIMEOUT_S) as response:
            response.read(4096)
    except (OSError, ValueError) as error:
        return f"not sent: {error}"
    return "sent"


def reported(result, event, paths, env, enabled):
    agent = result.get("agent") or {}
    counts = survey_counts(result) if event == "survey" else build_counts(result)
    url = env.get("FINITE_EXPORT_REPORT_URL", REPORT_URL) if enabled else ""
    identity = agent_identity(paths, env, agent.get("name")) if url else {}
    return {
        **result,
        "reported": send_report(url, event, identity, counts, agent.get("hermes_version")),
    }


# --- Which Finite this is ----------------------------------------------------

WRONG_PLATFORM_EXIT = 3
# What a Fragment computer (the new Finite) has that the old runtime never
# does: its image's own files, and its agents' identity in their terminal.
NEW_FINITE_MARKER = "/opt/fragment"


def platform(paths, env):
    """`old` on the old Finite's agent runtime (finite.computer), `new` on a
    Fragment computer (the new Finite, finite.chat), else `unknown`."""
    marker = Path(env.get("FINITE_EXPORT_NEW_MARKER", NEW_FINITE_MARKER))
    if env.get("FRAGMENT_AS_AGENT") or env.get("FRAGMENT_FOR") or marker.is_dir():
        return "new"
    if (
        paths.finite_home.is_dir()
        or paths.hermes_home.is_dir()
        or shutil.which("fsite")
        or shutil.which("fbrain")
    ):
        return "old"
    return "unknown"


def wrong_platform(where):
    """What to say instead of exporting, anywhere but the old Finite."""
    if where == "new":
        message = (
            "This export is for an agent on the old Finite (finite.computer), but this agent is on the new "
            "Finite (finite.chat). Nothing was exported. To move: send the export prompt to your old agent at "
            f"finite.computer, then attach the zip it sends you in a chat here, with the import prompt from {GUIDE_URL}."
        )
    else:
        message = (
            "This export is for an agent on the old Finite (finite.computer), and there is no old Finite agent "
            f"here. Nothing was exported. Send the export prompt from {GUIDE_URL} to your agent at finite.computer."
        )
    return {"error": "wrong_platform", "platform": where, "message": message}


def parse(argv):
    parser = argparse.ArgumentParser(
        description="Export this agent for the new Finite (see export.md)."
    )
    commands = parser.add_subparsers(dest="command", required=True)
    survey_cmd = commands.add_parser("survey", help="what there is to export; changes nothing")
    survey_cmd.add_argument("--max-file-mb", type=int, default=DEFAULT_MAX_FILE_MB)
    survey_cmd.add_argument(
        "--no-report", action="store_true", help="don't tell the move site this ran"
    )
    build_cmd = commands.add_parser("build", help="write the zip")
    build_cmd.add_argument(
        "--brain", action="append", default=[], help="a brain's id to include (repeat)"
    )
    build_cmd.add_argument(
        "--site",
        action="append",
        default=[],
        help="a site the agent may edit but does not own (repeat)",
    )
    build_cmd.add_argument(
        "--leave-out-site",
        action="append",
        default=[],
        help="a site to leave out, by its slug (repeat)",
    )
    build_cmd.add_argument(
        "--no-workspace", action="store_true", help="leave the workspace's files out"
    )
    build_cmd.add_argument(
        "--max-file-mb",
        type=int,
        default=DEFAULT_MAX_FILE_MB,
        help="leave out any file larger than this",
    )
    build_cmd.add_argument(
        "--out", help="the folder for the zip (default: <workspace>/finite-export)"
    )
    build_cmd.add_argument(
        "--no-report", action="store_true", help="don't tell the move site this ran"
    )
    options = parser.parse_args(argv)
    if options.max_file_mb < 1:
        parser.error("--max-file-mb must be at least 1")
    return options


def main(argv=None):
    options = parse(sys.argv[1:] if argv is None else argv)
    paths = paths_from_env(os.environ)
    where = platform(paths, os.environ)
    if where != "old":
        json.dump(wrong_platform(where), sys.stdout, ensure_ascii=False, indent=2)
        sys.stdout.write("\n")
        return WRONG_PLATFORM_EXIT
    if options.command == "survey":
        out_dir = paths.workspace / "finite-export"
        found = survey(paths, os.environ, out_dir, options.max_file_mb * MiB)
        result = reported(found, "survey", paths, os.environ, not options.no_report)
    else:
        result = reported(
            build(paths, os.environ, options), "build", paths, os.environ, not options.no_report
        )
        # Kept beside the zip too, for a build run in the background.
        Path(result["zip"]).with_name("last-build.json").write_text(
            json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
    json.dump(result, sys.stdout, ensure_ascii=False, indent=2)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
