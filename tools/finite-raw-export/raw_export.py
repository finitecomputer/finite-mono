#!/usr/bin/env python3
"""Export an old finite.computer agent's raw folder for the new Finite.

    python3 raw_export.py --out /data/agent/exports/raw.zip [--brain ID]...
                          [--site SLUG=DIR]... [--fact PATH=FILE]... [--root /data]

It writes one zip of the agent's own folder in the `finite-computer` layout
(paths relative to `/data`: `agent/hermes-home/...`, `workspace/...`,
`sites/<slug>/...`), minus secrets, caches and the runtime. The new agent
reads it with `fragment home` and the `import-agent-home` skill, exactly as
it reads a stage the mover makes.

This is the reference for export.py's `build --raw`. It ports the one
classifier of `finite-mover-host` (finite-mono,
finitecomputer-v2/crates/finite-mover-host/src/classify.rs), its credential
scan (src/shapes.rs) and its config copy (src/config_copy.rs). A test there
(`tests/host.rs`, `the_raw_exporter_matches_the_classifier`) runs this
script on the host reader's fixture and requires the same answer. The
spec: FIN-463, "The raw-folder export".

Differences from the host reader, because this runs inside the live agent:
- `state.db` and `memory_store.db` are copied with SQLite's `VACUUM INTO`,
  a consistent copy of their live rows, so their `-wal`, `-shm` and
  `-journal` are not needed. A database SQLite can't open is copied as it
  is, with its sidecars.
- No writer lease is taken; the agent itself holds it.
- Owned brains, sites and the small fact files come from the caller
  (export.py knows them through `fbrain`, `fsite` and the agent's env).

Standard library only, so it runs on the old runtime's own Python. It never
follows a link, never reads a file with a second name, and never writes
inside the folder it exports. Each run prints one JSON line.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sqlite3
import stat as stat_module
import sys
import tempfile
import time
import zipfile
from pathlib import Path

SUMMARY_FORMAT = "finite-mover-host/summary/1"
SUMMARY_ENTRY = ".finite-stage/summary.json"
CONFIG_COPY = (b"agent", b"hermes-home", b"config.public.yaml")
HERMES_CONFIG = (b"agent", b"hermes-home", b"config.yaml")
LEFT_MAX = 1000
DEPTH_MAX = 128
CONFIG_MAX_BYTES = 4 * 1024 * 1024
BRAIN_DIRECTORY_MAX_BYTES = 1024 * 1024
TEXT_SNAPSHOT_MAX = 256 * 1024 * 1024
FACT_MAX_BYTES = 64 * 1024

# --- The classifier (classify.rs) -------------------------------------------

# Class and reason, in classify.rs's declaration order: the summary breaks
# ties in this order.
CLASSES = ("secret", "config", "cache", "runtime", "history", "work")
STAGED = {"config", "history", "work"}
WHYS = (
    "nostr-identity",
    "hermes-env",
    "hermes-config",
    "agentd",
    "chat-client-store",
    "google-credentials",
    "google-workspace-cli",
    "simplex",
    "pairing",
    "credential-name",
    "token-in-text",
    "uncheckable-text",
    "config-copy",
    "replaced-by-copy",
    "cache-dir",
    "build-output",
    "not-owned-brain",
    "not-a-file",
    "uncarried-name",
    "history",
    "workspace",
    "hidden-folder",
    "earlier-export",
    "site",
    "runtime",
)

BUILD_DIRS = {b"dist", b"build", b"out", b"target", b"_build", b"obj", b"deps", b"vendor", b"coverage"}
PROJECT_FILES = {
    b"package.json", b"deno.json", b"Cargo.toml", b"go.mod", b"pyproject.toml", b"setup.py",
    b"pom.xml", b"build.gradle", b"build.gradle.kts", b"Makefile", b"CMakeLists.txt", b"mix.exs",
    b"Gemfile", b"composer.json", b"pubspec.yaml", b"Package.swift", b"build.zig", b"stack.yaml",
}  # fmt: skip
PROJECT_SUFFIXES = (b".csproj", b".fsproj", b".sln", b".cabal")
CACHE_DIRS = {
    b".git", b"node_modules", b".venv", b"venv", b"__pycache__", b".cache", b".next",
    b".finitebrain", b".mypy_cache", b".pytest_cache", b".tox", b".gradle",
    b"bower_components", b"site-packages", b"Pods", b"DerivedData", b"elm-stuff",
}  # fmt: skip
SECRET_NAMES = {
    b".env", b".envrc", b"auth.json", b"google_token.json", b"google_client_secret.json",
    b"google_oauth_pending.json", b"identity.json", b"git-credentials", b".git-credentials",
    b".netrc", b".npmrc", b".pypirc", b".pgpass", b"id_rsa", b"id_dsa", b"id_ecdsa",
    b"id_ed25519", b"id_ecdsa_sk", b"id_ed25519_sk", b"credentials", b"credentials.json",
    b"client_secret.json", b"token.json", b"secrets.yaml", b"secrets.yml", b"secrets.json",
    b".htpasswd", b"_netrc",
}  # fmt: skip
SECRET_SUFFIXES = (b".pem", b".key", b".p12", b".pfx", b".keystore", b".env")
SECRET_DIRS = {b".ssh", b".gnupg", b".aws"}
GOOGLE_NAMES = {b"google_token.json", b"google_client_secret.json", b"google_oauth_pending.json"}
HISTORY_FILES = {b"SOUL.md"}
HISTORY_DATABASES = (b"state.db", b"memory_store.db")
HISTORY_DIRS = {b"memories", b"skills", b"cron", b"scripts"}
SQLITE_SIDECARS = (b"-wal", b"-shm", b"-journal")


class Verdict:
    __slots__ = ("cls", "why", "depth")

    def __init__(self, cls, why, depth):
        self.cls, self.why, self.depth = cls, why, depth

    def __eq__(self, other):
        return (self.cls, self.why, self.depth) == (other.cls, other.why, other.depth)

    def __repr__(self):
        return f"Verdict({self.cls}, {self.why}, {self.depth})"


def project_file(name):
    return name in PROJECT_FILES or name.endswith(PROJECT_SUFFIXES)


def secret_name(name):
    """`secret_reason` of export.py, ignoring case, plus ssh keys."""
    lower = name.lower()
    return (
        lower in SECRET_NAMES
        or lower.startswith(b".env.")
        or lower.endswith(SECRET_SUFFIXES)
        or (lower.endswith(b".json") and (lower.startswith(b"client_secret") or service_account(lower)))
    )


def service_account(lower):
    if not lower.startswith(b"service"):
        return False
    rest = lower[len(b"service") :]
    if rest[:1] in (b"-", b"_"):
        rest = rest[1:]
    return rest.startswith(b"account")


def carried_name(part):
    try:
        text = part.decode("utf-8")
    except UnicodeDecodeError:
        return False
    return bool(text) and not any(c == "\\" or is_control(c) for c in text)


def is_control(character):
    # Rust's char::is_control: the Cc category, U+0000-U+001F and U+007F-U+009F.
    code = ord(character)
    return code < 0x20 or 0x7F <= code <= 0x9F


def classify(parts, kind="file", brain=None, build_output=None):
    """One path's class. `parts` are the path's components as bytes; `kind`
    is "file" or "other" (a link, pipe, device, a file with a second name or
    a folder on another filesystem); `brain` is (depth, owned) of the brain
    working tree the path is in, if any; `build_output` the depth of the
    build folder it is under, if any."""
    length = len(parts)
    secret = secret_rule(parts)
    if secret:
        return secret
    for index, part in enumerate(parts[:-1]):
        if part in CACHE_DIRS:
            return Verdict("cache", "cache-dir", index + 1)
    if brain is not None and not brain[1]:
        return Verdict("runtime", "not-owned-brain", brain[0])
    if build_output is not None:
        return Verdict("cache", "build-output", build_output)
    if kind != "file":
        return Verdict("runtime", "not-a-file", length)
    for index, part in enumerate(parts):
        if not carried_name(part):
            return Verdict("runtime", "uncarried-name", index + 1)
    return allow_list(parts)


def secret_rule(parts):
    length = len(parts)

    def at(index, name):
        return index < length and parts[index] == name

    if at(0, b"agent"):
        if at(1, b"identity") and length > 2:
            return Verdict("secret", "nostr-identity", 2)
        if at(1, b"simplex") and length > 2:
            return Verdict("secret", "simplex", 2)
        if length == 2 and parts[1].startswith(b"client.sqlite3"):
            return Verdict("secret", "chat-client-store", 2)
        if at(1, b"agentd") and length > 2:
            return Verdict("secret", "agentd", 2)
        if at(1, b"hermes-home") and length > 2:
            name = parts[2]
            if length == 3 and name == b".env":
                return Verdict("secret", "hermes-env", 3)
            if length == 3 and (name.startswith(b"config.yaml") or name.startswith(b".config.yaml")):
                return Verdict("secret", "hermes-config", 3)
            if at(2, b"gws") and length > 3:
                return Verdict("secret", "google-workspace-cli", 3)
            if at(2, b"pairing") and length > 3:
                return Verdict("secret", "pairing", 3)
            if at(2, b"platforms") and at(3, b"pairing") and length > 4:
                return Verdict("secret", "pairing", 4)
    for index, part in enumerate(parts[:-1]):
        if part.lower() in SECRET_DIRS:
            return Verdict("secret", "credential-name", index + 1)
    if not parts:
        return None
    name = parts[-1]
    if length >= 2 and parts[-2] == b"pairing" and name.endswith(b"-approved.json"):
        return Verdict("secret", "pairing", length)
    if name.lower() in GOOGLE_NAMES:
        return Verdict("secret", "google-credentials", length)
    if secret_name(name):
        return Verdict("secret", "credential-name", length)
    return None


def allow_list(parts):
    length = len(parts)

    def at(index, name):
        return index < length and parts[index] == name

    def runtime(depth):
        return Verdict("runtime", "runtime", min(depth, length))

    if at(0, b"workspace") and length > 1:
        if at(1, b"finite-export") and length > 2:
            return Verdict("runtime", "earlier-export", 2)
        for index, part in enumerate(parts[1:-1]):
            if part.startswith(b"."):
                return Verdict("runtime", "hidden-folder", index + 2)
        return Verdict("work", "workspace", 1)
    if at(0, b"sites") and length > 2:
        return Verdict("work", "site", 1)
    if not at(0, b"agent"):
        return runtime(1)
    if at(1, b"hermes-home") and length > 2:
        name = parts[2]
        if length == 3:
            if tuple(parts) == CONFIG_COPY:
                return Verdict("runtime", "replaced-by-copy", 3)
            if name in HISTORY_FILES or history_database(name):
                return Verdict("history", "history", 3)
        if length > 3 and name in HISTORY_DIRS:
            return Verdict("history", "history", 3)
        return runtime(3)
    if at(1, b"managed-skills"):
        if not at(2, b"finite"):
            return runtime(3)
        if not at(3, b"current"):
            return runtime(4)
        if length > 4:
            return Verdict("history", "history", 4)
        return runtime(4)
    return runtime(2)


def history_database(name):
    for database in HISTORY_DATABASES:
        if name.startswith(database):
            rest = name[len(database) :]
            if not rest or rest in SQLITE_SIDECARS:
                return True
    return False


# --- Credential-shaped text (shapes.rs, from export.py's TOKEN_SHAPES) ------

CHUNK = 1024 * 1024
BINARY_PROBE = 8192
CARRY_MIN = 4096
CARRY_MAX = 16 * 1024 * 1024
WORD_START = rb"(?<![A-Za-z0-9_-])"
# Each shape on its own, found at every position (a zero-width look-ahead),
# so every candidate the host reader tries is tried here too.
TOKEN_SHAPES = [
    re.compile(WORD_START + rb"(?=(" + shape + rb"))")
    for shape in (
        rb"sk-[A-Za-z0-9_-]{16,}",
        rb"sk_live_[A-Za-z0-9]{16,}",
        rb"rk_live_[A-Za-z0-9]{16,}",
        rb"ghp_[A-Za-z0-9]{20,}",
        rb"github_pat_[A-Za-z0-9_]{20,}",
        rb"glpat-[A-Za-z0-9_-]{20,}",
        rb"xox[abpr]-[A-Za-z0-9-]{10,}",
        rb"AIza[A-Za-z0-9_-]{30,}",
        rb"AKIA[0-9A-Z]{16}",
        rb"ntn_[A-Za-z0-9]{20,}",
        rb"hf_[A-Za-z0-9]{30,}",
        rb"[0-9]{6,}:[A-Za-z0-9_-]{30,}",
        rb"nsec1[0-9a-z]{20,}",
        rb"eyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{10,}",
    )
]
ALWAYS_SECRET = [
    re.compile(rb"(?=-----BEGIN [A-Z ]*PRIVATE KEY-----)"),
    re.compile(rb"(?=discord(?:app)?\.com/api/webhooks/[0-9]+/[A-Za-z0-9_\x80-\xff-])"),
]
# Rust's is_ascii_whitespace, which leaves out \x0b.
URL_PASSWORD = re.compile(rb"(?=https?://[^ \t\n\x0c\r/:@]+:([^ \t\n\x0c\r/@]+)@)")
PLACEHOLDER_WORDS = (b"xxxx", b"****", b"...", b"your", b"example", b"placeholder", b"redacted", b"changeme")


def placeholder(text):
    lower = text.lower()
    return any(word in lower for word in PLACEHOLDER_WORDS) or any(c in lower for c in b"<>${}")


def secret_token(token):
    return not placeholder(token) and (any(48 <= c <= 57 for c in token) or len(token) >= 32)


def secret_password(password):
    return (
        len(password) >= 6
        and not placeholder(password)
        and not re.fullmatch(rb"[A-Z0-9_]+", password)
    )


def holds_secret(text):
    if any(shape.search(text) for shape in ALWAYS_SECRET):
        return True
    if any(secret_password(m.group(1)) for m in URL_PASSWORD.finditer(text)):
        return True
    return any(secret_token(m.group(1)) for shape in TOKEN_SHAPES for m in shape.finditer(text))


def carry_start(text):
    after_space = max(text.rfind(c) for c in (b" ", b"\n", b"\r", b"\t", b"\x0c", b"\x0b")) + 1
    return min(after_space, max(len(text) - CARRY_MIN, 0))


def utf16_ascii(chunk, little):
    low, high = (chunk[0::2], chunk[1::2]) if little else (chunk[1::2], chunk[0::2])
    return bytes(lo if hi == 0 and lo < 0x80 else 0x80 for lo, hi in zip(low, high))


def scan(data):
    """`clean`, `binary`, `secret` or `uncheckable`, read in the host's 1 MiB
    chunks with its carry, so the two always agree."""
    first = data[:CHUNK]
    if first.startswith(b"\xff\xfe"):
        utf16 = True
    elif first.startswith(b"\xfe\xff"):
        utf16 = False
    elif 0 in first[:BINARY_PROBE]:
        return "binary"
    else:
        utf16 = None
    text = b""
    for offset in range(0, len(data), CHUNK):
        chunk = data[offset : offset + CHUNK]
        text += chunk if utf16 is None else utf16_ascii(chunk[: len(chunk) & ~1], utf16)
        if holds_secret(text):
            return "secret"
        text = text[carry_start(text) :]
        if len(text) > CARRY_MAX:
            return "uncheckable"
    return "clean"


def snapshot(handle, size):
    """Read a file once: ("binary", None), ("text", bytes), ("secret", None)
    or ("uncheckable", None). A text file is staged from these bytes."""
    first = handle.read(BINARY_PROBE)
    if not first.startswith((b"\xff\xfe", b"\xfe\xff")) and 0 in first:
        return "binary", None
    if size > TEXT_SNAPSHOT_MAX:
        return "uncheckable", None
    text = first + handle.read(TEXT_SNAPSHOT_MAX + 1 - len(first))
    if len(text) > TEXT_SNAPSHOT_MAX:
        return "uncheckable", None
    verdict = scan(text)
    if verdict in ("clean", "binary"):
        return "text", text
    return verdict, None


# --- The allow-listed config (config_copy.rs) -------------------------------

VALUE_MAX_BYTES = 64 * 1024


def public_config(original):
    """`config.public.yaml` from `config.yaml`: timezone, agent.personality,
    model.default and gateway.platforms.telegram.enabled, values that look
    like a credential dropped. None if it can't be read."""
    document = read_yaml(original)
    if not isinstance(document, dict):
        return None

    def text(value):
        if not isinstance(value, str):
            return None
        encoded = value.encode("utf-8")
        return value if len(encoded) <= VALUE_MAX_BYTES and not holds_secret(encoded) else None

    def get(mapping, *keys):
        for key in keys:
            mapping = mapping.get(key) if isinstance(mapping, dict) else None
        return mapping

    lines = []
    timezone = text(document.get("timezone"))
    if timezone is not None:
        lines.append(f"timezone: {quoted(timezone)}")
    personality = get(document, "agent", "personality")
    if personality is None:
        personality = document.get("personality")
    personality = text(personality)
    if personality is not None:
        lines += ["agent:", f"  personality: {quoted(personality)}"]
    model = document.get("model")
    default = get(document, "model", "default") if isinstance(model, dict) else model
    default = text(default)
    if default is not None:
        lines += ["model:", f"  default: {quoted(default)}"]
    enabled = get(document, "gateway", "platforms", "telegram", "enabled")
    if isinstance(enabled, bool):
        lines += ["gateway:", "  platforms:", "    telegram:", f"      enabled: {str(enabled).lower()}"]
    copy = ("\n".join(lines) + "\n" if lines else "{}\n").encode("utf-8")
    return None if holds_secret(copy) else copy


def quoted(value):
    # A JSON string is a YAML double-quoted scalar.
    return json.dumps(value, ensure_ascii=False)


def read_yaml(original):
    """`config.yaml` read with PyYAML, which Hermes itself reads it with.
    Without PyYAML there is no copy: a reader of our own could misread a
    value (FIN-463 review of #1148)."""
    try:
        import yaml
    except ImportError:
        return None
    try:
        return yaml.safe_load(original.decode("utf-8"))
    except (UnicodeDecodeError, yaml.YAMLError):
        return None


# --- The walk ------------------------------------------------------------------


class TreeChanged(Exception):
    pass


class Export:
    def __init__(self, root, out, owned_brains, extra_name_check=None, extra_text_check=None):
        self.root = Path(root)
        self.out = Path(out)
        self.owned = set(owned_brains)
        self.extra_name_check = extra_name_check
        self.extra_text_check = extra_text_check
        self.classes = {cls: [0, 0] for cls in CLASSES}
        self.left = {}
        # Each staged file's absolute path and fstat, checked again at the end.
        self.staged_files = []
        self.temporary = []
        self.vacuumed = set()
        # The folder being walked, and how many leading parts of a path
        # name it in the zip rather than on disk (`sites/<slug>`).
        self.walked = (os.fsencode(self.root), 0)

    # The summary (summary.rs).
    def add_left(self, parts, verdict, size):
        self.classes[verdict.cls][0] += 1
        self.classes[verdict.cls][1] += size
        prefix = "/".join(p.decode("utf-8", "replace") for p in parts[: max(1, min(verdict.depth, len(parts)))])
        key = (prefix, CLASSES.index(verdict.cls), WHYS.index(verdict.why))
        line = self.left.setdefault(key, [0, 0])
        line[0] += 1
        line[1] += size

    def add_staged(self, cls, size):
        self.classes[cls][0] += 1
        self.classes[cls][1] += size

    def summary(self, summary_id):
        lines = [
            {"path": path, "class": CLASSES[cls], "why": WHYS[why], "files": files, "bytes": size}
            for (path, cls, why), (files, size) in sorted(self.left.items())
        ]
        lines.sort(key=lambda line: (-line["bytes"], line["path"]))
        more = lines[LEFT_MAX:]
        staged = [sum(self.classes[c][i] for c in CLASSES if c in STAGED) for i in (0, 1)]
        return {
            "format": SUMMARY_FORMAT,
            "id": summary_id,
            "classes": {c: {"files": f, "bytes": b} for c, (f, b) in self.classes.items()},
            "staged": {"files": staged[0], "bytes": staged[1]},
            "left": lines[:LEFT_MAX],
            "left_more": {"files": sum(l["files"] for l in more), "bytes": sum(l["bytes"] for l in more)},
        }

    def run(self, zip_file, sites, facts, summary_id):
        root_fd = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
        try:
            self.device = os.fstat(root_fd).st_dev
            self.zip = zip_file
            self.directory(root_fd, [], None, None)
        finally:
            os.close(root_fd)
        # The tree wins: a clone or fact the tree already holds is not added
        # again, so no name is in the zip twice.
        in_tree = set(self.zip.namelist())
        for slug, folder in sorted(sites.items()):
            if any(name.startswith(f"sites/{slug}/") for name in in_tree):
                continue
            fd = os.open(folder, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
            try:
                self.device = os.fstat(fd).st_dev
                self.walked = (os.fsencode(folder), 2)
                self.directory(fd, [b"sites", slug.encode()], None, None)
            finally:
                os.close(fd)
        for name, data in sorted(facts.items()):
            if name in in_tree:
                continue
            self.write_bytes(name, data, 0o644, 0)
            self.add_staged("config", len(data))
        # A file written to after it was staged would leave a stale stage.
        for absolute, before in self.staged_files:
            if not still_the_same(absolute, before):
                raise TreeChanged(os.fsdecode(absolute))
        summary = json.dumps(self.summary(summary_id), indent=2, ensure_ascii=False).encode()
        self.write_bytes(SUMMARY_ENTRY, summary, 0o644, 0)

    def directory(self, fd, path, brain, build):
        if len(path) >= DEPTH_MAX:
            raise ValueError("deeper than 128 folders: " + "/".join(p.decode("utf-8", "replace") for p in path))
        names = sorted(os.fsencode(name) for name in os.listdir(fd))
        project_here = any(
            project_file(name) and stat_module.S_ISREG(lstat(fd, name).st_mode) for name in names
        )
        for name in names:
            self.entry(fd, name, path + [name], brain, build, project_here)

    def entry(self, fd, name, path, brain, build, project_here):
        try:
            st = lstat(fd, name)
        except FileNotFoundError:
            raise TreeChanged(path_text(path)) from None
        if stat_module.S_ISDIR(st.st_mode) and st.st_dev == self.device:
            child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd)
            try:
                if build is None and path[0] == b"workspace" and project_here and name in BUILD_DIRS:
                    build = len(path)
                self.directory(child, path, self.brain_scope(child, path, brain), build)
            finally:
                os.close(child)
            return
        regular = stat_module.S_ISREG(st.st_mode) and st.st_nlink == 1
        verdict = classify(path, "file" if regular else "other", brain, build)
        size = st.st_size if stat_module.S_ISREG(st.st_mode) else 0
        if verdict.cls in STAGED:
            if self.extra_name_check and self.extra_name_check(name.decode("utf-8", "replace")):
                verdict = Verdict("secret", "credential-name", len(path))
            elif self.is_output(fd, st):
                verdict = Verdict("runtime", "earlier-export", len(path))
            else:
                return self.staged(fd, name, path, verdict, st)
        self.add_left(path, verdict, size)
        if regular and tuple(path) == HERMES_CONFIG:
            self.config_copy(fd, name)

    def is_output(self, fd, st):
        # Never export the zip being written, wherever --out points.
        try:
            out = os.stat(self.out)
        except FileNotFoundError:
            return False
        return (out.st_dev, out.st_ino) == (st.st_dev, st.st_ino)

    def staged(self, fd, name, path, verdict, st):
        if path[-1].startswith(HISTORY_DATABASES) and verdict.cls == "history" and len(path) == 3:
            if self.database(fd, name, path, st):
                return
        handle = open_regular(fd, name)
        if handle is None:
            raise TreeChanged(path_text(path))
        with handle:
            opened = os.fstat(handle.fileno())
            if not same_file(st, opened):
                raise TreeChanged(path_text(path))
            kind, text = snapshot(handle, opened.st_size)
            if kind == "text" and self.extra_text_check and self.extra_text_check(text):
                kind = "secret"
            if kind in ("secret", "uncheckable"):
                why = "token-in-text" if kind == "secret" else "uncheckable-text"
                return self.add_left(path, Verdict("secret", why, len(path)), opened.st_size)
            arcname = b"/".join(path).decode("utf-8")
            if kind == "text":
                if len(text) != opened.st_size:
                    raise TreeChanged(path_text(path))
                self.write_bytes(arcname, text, opened.st_mode, opened.st_mtime)
            else:
                handle.seek(0)
                self.write_stream(arcname, handle, opened)
                if not same_file(opened, os.fstat(handle.fileno())):
                    raise TreeChanged(path_text(path))
            self.add_staged(verdict.cls, opened.st_size)
            self.staged_files.append((self.absolute(path), opened))

    def absolute(self, path):
        """Where a staged path is on disk: under the folder being walked."""
        folder, skip = self.walked
        return os.path.join(folder, *path[skip:])

    def database(self, fd, name, path, st):
        """A history database through `VACUUM INTO`: its live rows, whole,
        without its sidecars. False if SQLite can't copy it."""
        database = path[-1]
        if database not in HISTORY_DATABASES:
            # A sidecar: staged only with a database that SQLite could not copy.
            main = database
            for sidecar in SQLITE_SIDECARS:
                if database.endswith(sidecar):
                    main = database[: -len(sidecar)]
            return main in self.vacuumed
        folder = Path(os.path.abspath(self.root.joinpath(*(p.decode("utf-8") for p in path[:-1]))))
        if not self.sqlite_reads_what_was_walked(fd, folder, database):
            # Copying the main file alone would leave the rows in its WAL
            # behind, and SQLite would read a linked one: stop instead.
            raise ValueError(
                f"{path_text(path)} or one of its -wal, -shm or -journal files has another name "
                "or isn't a plain file, so its history can't be copied whole; remove the extra "
                "link and export again"
            )
        target = Path(tempfile.mkstemp(prefix=".raw-export-", suffix=".db", dir=self.out.parent)[1])
        target.unlink()
        self.temporary.append(target)
        try:
            conn = sqlite3.connect((folder / database.decode("utf-8")).as_uri() + "?mode=ro", uri=True)
            try:
                conn.execute("VACUUM INTO ?", (str(target),))
            finally:
                conn.close()
        except sqlite3.Error:
            target.unlink(missing_ok=True)
            return False
        if not self.sqlite_reads_what_was_walked(fd, folder, database):
            raise TreeChanged(path_text(path))
        copied = target.stat()
        with open(target, "rb") as handle:
            self.write_stream(b"/".join(path).decode("utf-8"), handle, copied, st.st_mode, st.st_mtime)
        target.unlink()
        self.add_staged("history", copied.st_size)
        self.vacuumed.add(database)
        return True

    def sqlite_reads_what_was_walked(self, fd, folder, database):
        """SQLite opens a database and its sidecars by name, past the walk's
        checks. So the folder at that name must be the one walked, and the
        database and each sidecar there a regular file with one name on the
        tree's filesystem: never a link to rows outside it."""
        try:
            here = os.stat(folder)
        except OSError:
            return False
        walked = os.fstat(fd)
        if (here.st_dev, here.st_ino) != (walked.st_dev, walked.st_ino):
            return False
        for suffix in (b"",) + SQLITE_SIDECARS:
            try:
                st = lstat(fd, database + suffix)
            except FileNotFoundError:
                if suffix:
                    continue
                return False
            if not stat_module.S_ISREG(st.st_mode) or st.st_nlink != 1 or st.st_dev != self.device:
                return False
        return True

    def config_copy(self, fd, name):
        handle = open_regular(fd, name)
        if handle is None:
            return
        with handle:
            original = handle.read(CONFIG_MAX_BYTES + 1)
        if len(original) > CONFIG_MAX_BYTES:
            return
        copy = public_config(original)
        if copy is not None:
            self.write_bytes(b"/".join(CONFIG_COPY).decode(), copy, 0o644, 0)
            self.add_staged("config", len(copy))

    def brain_scope(self, fd, path, outer):
        if outer is not None and not outer[1]:
            return outer
        try:
            if not stat_module.S_ISDIR(lstat(fd, b".finitebrain").st_mode):
                return outer
        except OSError:
            return outer
        return (len(path), brain_id(fd) in self.owned)

    def write_bytes(self, name, data, mode, mtime):
        info = zip_info(name, mode, mtime)
        with self.zip.open(info, "w", force_zip64=True) as entry:
            entry.write(data)

    def write_stream(self, name, handle, st, mode=None, mtime=None):
        info = zip_info(name, st.st_mode if mode is None else mode, st.st_mtime if mtime is None else mtime)
        copied = 0
        with self.zip.open(info, "w", force_zip64=True) as entry:
            while True:
                block = handle.read(CHUNK)
                if not block:
                    break
                entry.write(block)
                copied += len(block)
        if copied != st.st_size:
            raise TreeChanged(name)


def lstat(fd, name):
    return os.stat(name, dir_fd=fd, follow_symlinks=False)


def open_regular(fd, name):
    """A regular file with one name, never through a link or a pipe."""
    flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_NOCTTY | os.O_CLOEXEC
    try:
        file_fd = os.open(name, flags, dir_fd=fd)
    except (FileNotFoundError, OSError):
        return None
    st = os.fstat(file_fd)
    if not stat_module.S_ISREG(st.st_mode) or st.st_nlink != 1:
        os.close(file_fd)
        return None
    return os.fdopen(file_fd, "rb")


def same_file(a, b):
    return (a.st_dev, a.st_ino, a.st_size, a.st_mtime_ns) == (b.st_dev, b.st_ino, b.st_size, b.st_mtime_ns)


def still_the_same(absolute, before):
    try:
        now = os.stat(absolute, follow_symlinks=False)
    except OSError:
        return False
    return now.st_nlink == 1 and same_file(before, now)


def brain_id(fd):
    try:
        control = os.open(b".finitebrain", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd)
    except OSError:
        return None
    try:
        handle = open_regular(control, b"brain-directory.json")
        if handle is None:
            return None
        with handle:
            data = handle.read(BRAIN_DIRECTORY_MAX_BYTES + 1)
    finally:
        os.close(control)
    if len(data) > BRAIN_DIRECTORY_MAX_BYTES:
        return None
    try:
        manifest = json.loads(data)
        identifier = manifest["brain"]["id"]
    except (ValueError, KeyError, TypeError):
        return None
    return identifier if isinstance(identifier, str) else None


def path_text(path):
    return "/".join(part.decode("utf-8", "replace") for part in path)


def zip_info(name, mode, mtime):
    info = zipfile.ZipInfo(name, date_time=dos_time(mtime))
    info.compress_type = zipfile.ZIP_STORED
    info.create_system = 3
    info.external_attr = (stat_module.S_IFREG | (mode & 0o777)) << 16
    return info


def dos_time(seconds):
    moment = time.gmtime(max(int(seconds), 0))
    if moment.tm_year < 1980:
        return (1980, 1, 1, 0, 0, 0)
    if moment.tm_year > 2107:
        return (2107, 12, 31, 23, 59, 58)
    return moment[:6]


# --- The command ---------------------------------------------------------------

FACT_PATHS = re.compile(r"^(agent/profile\.json|workspace/finitebrain/[A-Za-z0-9_.-]+\.json|sites/[A-Za-z0-9_.-]+\.json)$")
SITE_SLUG = re.compile(r"^[A-Za-z0-9_.-]{1,128}$")


def export_raw(root, out, owned_brains=(), sites=None, facts=None, summary_id="raw",
               extra_name_check=None, extra_text_check=None):  # fmt: skip
    """Write the raw-folder zip to `out` and return its summary. `sites`
    maps a slug to a cloned site folder; `facts` maps an allowed fact path
    to its JSON bytes. On any failure the partial zip is removed."""
    out = Path(out)
    sites = dict(sites or {})
    facts = dict(facts or {})
    for slug in sites:
        if not SITE_SLUG.match(slug) or slug in (".", ".."):
            raise ValueError(f"{slug!r} is not a site slug")
    for name, data in facts.items():
        if not FACT_PATHS.match(name) or len(data) > FACT_MAX_BYTES or holds_secret(data):
            raise ValueError(f"{name} is not a fact file the import reads, or is not clean")
        json.loads(data)
    export = Export(root, out, owned_brains, extra_name_check, extra_text_check)
    out.parent.mkdir(parents=True, exist_ok=True)
    try:
        with zipfile.ZipFile(out, "w", zipfile.ZIP_STORED, allowZip64=True) as zip_file:
            export.run(zip_file, sites, facts, summary_id)
    except BaseException:
        out.unlink(missing_ok=True)
        raise
    finally:
        for temporary in export.temporary:
            temporary.unlink(missing_ok=True)
    return export.summary(summary_id)


def pairs(values, what):
    result = {}
    for value in values:
        key, separator, path = value.partition("=")
        if not separator or not key or not path:
            raise SystemExit(f"--{what} takes NAME=PATH")
        result[key] = path
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description="Export this agent's raw folder (FIN-463).")
    parser.add_argument("--root", default="/data", help="the agent's /data")
    parser.add_argument("--out", required=True, help="the zip to write, outside the workspace")
    parser.add_argument("--brain", action="append", default=[], help="an owned brain's id (repeat)")
    parser.add_argument("--site", action="append", default=[], help="SLUG=DIR, a cloned owned site (repeat)")
    parser.add_argument("--fact", action="append", default=[], help="ZIP_PATH=FILE, a fact file (repeat)")
    parser.add_argument("--id", default="raw", help="the summary's id")
    options = parser.parse_args(argv)
    facts = {name: Path(path).read_bytes() for name, path in pairs(options.fact, "fact").items()}
    try:
        summary = export_raw(
            options.root,
            options.out,
            options.brain,
            {slug: Path(path) for slug, path in pairs(options.site, "site").items()},
            facts,
            options.id,
        )
    except TreeChanged as changed:
        print(json.dumps({"error": f"{changed} changed while it was read; run again"}))
        return 75
    except (OSError, ValueError) as error:
        print(json.dumps({"error": str(error)}))
        return 74
    result = {
        "zip": str(Path(options.out)),
        "zip_bytes": Path(options.out).stat().st_size,
        "files": summary["staged"]["files"],
        "bytes": summary["staged"]["bytes"],
        "left": len(summary["left"]) + (1 if summary["left_more"]["files"] else 0),
        "summary": SUMMARY_ENTRY,
    }
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
