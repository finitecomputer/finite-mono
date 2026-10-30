"""Owner-started export of the whole Agent, built in the background and
downloaded from the dashboard.

Native Hermes authenticates before routing. Requests name no command, path or
file: the plugin owns one folder in the Finite Home and runs only the exporter
beside this file (`export.py --everything`), never the model. One survey and
one export exist at a time; a new one replaces the last, and a finished one is
deleted after seven days. Job state lives in that folder, so a Hermes restart
neither loses a finished export nor reports a dead job as running.
"""

import asyncio
import errno
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
from contextlib import suppress
from datetime import UTC, datetime
from pathlib import Path

from fastapi import APIRouter, Request
from fastapi.responses import FileResponse, JSONResponse

router = APIRouter()
EXPORTER = Path(__file__).with_name("export.py")
KEEP_SECONDS = 7 * 24 * 60 * 60
# Free space an export must leave on the disk it shares with other agents,
# after its staging copy and its zip (each about the size of the data).
RESERVE_BYTES = 20 * 1024**3
# A running survey or export stops below this much free space: nothing can
# size in advance the sites and brains that aren't on this computer yet.
FLOOR_BYTES = RESERVE_BYTES // 2
WATCH_SECONDS = 15
MAX_BODY_BYTES = 64 * 1024
# Choices become exporter arguments, so none may look like a flag.
NAME = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}")
HEADERS = {"Cache-Control": "no-store"}
# Runs one exporter command in the job's folder and records how it ended,
# even when Hermes restarts meanwhile.
WRAPPER = '"$@" >stdout.json 2>stderr.txt; echo $? >exit.part && mv exit.part exit'
_lock = threading.Lock()
_children = []


def exports_root():
    return Path(os.environ.get("FINITE_HOME", "/data/agent")) / "exports"


def error(status, code):
    return JSONResponse({"error": code}, status, HEADERS)


def iso(seconds):
    if seconds is None:
        return None
    return datetime.fromtimestamp(seconds, UTC).isoformat().replace("+00:00", "Z")


def read_json(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError, RecursionError):
        return None


def records(value):
    return [item for item in value if isinstance(item, dict)] if isinstance(value, list) else []


def count(value):
    return value if type(value) is int and value >= 0 else 0


def label(value, limit=200):
    return value[:limit] if isinstance(value, str) and value.strip() else None


# --- Jobs ---------------------------------------------------------------------


def process_start(pid):
    """When a live process started (it tells a reused pid apart), or None."""
    try:
        stat = Path(f"/proc/{pid}/stat").read_text()
    except OSError:
        return None
    fields = stat[stat.rindex(")") + 2 :].split()
    return None if fields[0] == "Z" else fields[19]


def alive(record):
    pid = record.get("pid")
    if type(pid) is not int or pid <= 0:
        return False
    if Path("/proc/self/stat").exists():
        return record.get("pidStart") is not None and process_start(pid) == record["pidStart"]
    try:  # No /proc (a developer's Mac): the pid alone.
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def reap():
    _children[:] = [child for child in _children if child.poll() is None]


def job(folder):
    """A job's state, read from its folder; None when there is none."""
    record = read_json(folder / "job.json")
    if not isinstance(record, dict) or not isinstance(record.get("startedAt"), int | float):
        return None
    try:
        code = (folder / "exit").read_text().strip()
        finished = (folder / "exit").stat().st_mtime
    except OSError:
        code = finished = None
    result = None
    if code == "0":
        result = read_json(folder / "stdout.json")
        state = "ready" if isinstance(result, dict) else "failed"
    elif code is not None or (folder / "stopped").exists():
        state = "failed"
    elif alive(record):
        state = "running"
    else:
        state = "interrupted"
    if state in ("failed", "interrupted"):
        # Free the disk now; the record says what happened.
        for part in ("tmp", "out"):
            shutil.rmtree(folder / part, ignore_errors=True)
    return {
        "folder": folder,
        "record": record,
        "state": state,
        "finished": finished,
        "result": result,
    }


def start(kind, argv, **extra):
    folder = exports_root() / kind
    shutil.rmtree(folder, ignore_errors=True)
    (folder / "tmp").mkdir(parents=True)
    nice = shutil.which("nice")
    command = [*([nice, "-n", "10"] if nice else []), sys.executable, "-I", str(EXPORTER), *argv]
    child = subprocess.Popen(
        [shutil.which("sh") or "/bin/sh", "-c", WRAPPER, "sh", *command],
        cwd=folder,
        # The exporter stages its copy on the Agent's disk beside the job, not in /tmp.
        env={**os.environ, "TMPDIR": str(folder / "tmp")},
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    _children.append(child)
    record = {
        "version": 1,
        "pid": child.pid,
        "pidStart": process_start(child.pid),
        "startedAt": time.time(),
        **extra,
    }
    try:
        part = folder / "job.json.part"
        part.write_text(json.dumps(record), encoding="utf-8")
        part.replace(folder / "job.json")
    except OSError:
        # Untracked, it would escape busy(), DELETE and the disk guard.
        with suppress(ProcessLookupError):
            os.killpg(child.pid, signal.SIGKILL)
        child.wait()
        shutil.rmtree(folder, ignore_errors=True)
        raise
    return job(folder)


def busy():
    return any(
        (current := job(exports_root() / kind)) and current["state"] == "running"
        for kind in ("survey", "export")
    )


def sweep(now=None):
    """Delete a survey or export kept for seven days after it ended."""
    now = time.time() if now is None else now
    for kind in ("survey", "export"):
        current = job(exports_root() / kind)
        if current is None or current["state"] == "running":
            continue
        ended = current["finished"] or current["record"]["startedAt"]
        if now - ended > KEEP_SECONDS:
            shutil.rmtree(current["folder"], ignore_errors=True)


def guard():
    """Stop a running survey or export before the disk it shares runs low."""
    for kind in ("survey", "export"):
        current = job(exports_root() / kind)
        if current is None or current["state"] != "running":
            continue
        if shutil.disk_usage(exports_root()).free < FLOOR_BYTES:
            with suppress(ProcessLookupError):
                os.killpg(current["record"]["pid"], signal.SIGKILL)
            for part in ("tmp", "out"):
                shutil.rmtree(current["folder"] / part, ignore_errors=True)
            # Best effort on a full disk: without it the job reads as interrupted.
            with suppress(OSError):
                (current["folder"] / "stopped").write_text("notEnoughSpace", encoding="utf-8")


def _watch():
    while True:
        time.sleep(WATCH_SECONDS)
        with suppress(Exception), _lock:
            reap()
            guard()
            sweep()


threading.Thread(target=_watch, name="finite-export-watch", daemon=True).start()


# --- Views --------------------------------------------------------------------


def offered(found):
    """What the person chooses from: brains the agent may export, and sites
    it owns (in unless left out) or may only edit (out unless chosen)."""
    brains = [
        {
            "id": brain["id"],
            "name": label(brain.get("name")) or brain["id"],
            "kind": "personal" if brain.get("kind") == "personal" else "organization",
            "bytes": brain["bytes"] if type(brain.get("bytes")) is int else None,
        }
        for brain in records(found.get("brains"))
        if brain.get("can_export") is True
        and isinstance(brain.get("id"), str)
        and NAME.fullmatch(brain["id"])
    ]
    sites = [
        {
            "slug": site["slug"],
            "url": label(site.get("url"), 2048),
            "owned": site["export"] == "yes",
        }
        for site in records(found.get("sites"))
        if site.get("export") in ("yes", "ask")
        and isinstance(site.get("slug"), str)
        and NAME.fullmatch(site["slug"])
    ]
    return brains, sites


def summary(found):
    brains, sites = offered(found)
    chats = found.get("chats") if isinstance(found.get("chats"), dict) else {}
    workspace = found.get("workspace") if isinstance(found.get("workspace"), dict) else {}
    memories, problems = found.get("memories"), found.get("problems")
    return {
        "conversations": count(chats.get("conversations")),
        "memories": len(memories) if isinstance(memories, list) else 0,
        "skills": sum(
            1 for skill in records(found.get("skills")) if skill.get("comes_over") is True
        ),
        "scheduledJobs": len(records(found.get("cron"))),
        "files": count(workspace.get("files")),
        "fileBytes": count(workspace.get("bytes")),
        "brains": brains,
        "sites": sites,
        "problems": len(problems) if isinstance(problems, list) else 0,
    }


def archive(current):
    """The finished export's zip, only inside its own folder."""
    name = (current["result"] or {}).get("zip")
    if not isinstance(name, str):
        return None
    out = (current["folder"] / "out").resolve()
    path = Path(name)
    if path.is_symlink() or path.suffix != ".zip":
        return None
    path = path.resolve()
    return path if path.parent == out and path.is_file() else None


def view(current, ready):
    if current is None:
        return None
    base = {
        "state": current["state"],
        "startedAt": iso(current["record"]["startedAt"]),
        "finishedAt": iso(current["finished"]),
    }
    return {**base, **ready(current)} if current["state"] == "ready" else base


def survey_view(current):
    return view(current, lambda done: {"summary": summary(done["result"])})


def export_view(current):
    def ready(done):
        path = archive(done)
        if path is None:
            return {"state": "failed"}
        problems = done["result"].get("problems")
        return {
            "expiresAt": iso(done["finished"] + KEEP_SECONDS),
            "bytes": path.stat().st_size,
            "fileName": path.name,
            "problems": len(problems) if isinstance(problems, list) else 0,
        }

    return view(current, ready)


# --- Routes -------------------------------------------------------------------


def _status():
    with _lock:
        reap()
        guard()
        sweep()
        root = exports_root()
        return JSONResponse(
            {
                "version": 1,
                "survey": survey_view(job(root / "survey")),
                "export": export_view(job(root / "export")),
            },
            headers=HEADERS,
        )


def chat_store_bytes():
    """The chat store a survey or export copies, with its write-ahead log."""
    store = Path(os.environ.get("HERMES_HOME", "")) / "state.db"
    total = 0
    for suffix in ("", "-wal"):
        with suppress(OSError):
            total += Path(f"{store}{suffix}").stat().st_size
    return total


def started(kind, argv, view, **extra):
    """Start a job; a disk too full to set it up answers like the space check."""
    try:
        current = start(kind, argv, **extra)
    except OSError as failure:
        if failure.errno == errno.ENOSPC:
            return error(507, "notEnoughSpace")
        raise
    return JSONResponse({"version": 1, kind: view(current)}, 202, HEADERS)


def _start_survey():
    with _lock:
        reap()
        sweep()
        if busy():
            return error(409, "busy")
        # The Finite Home holds the exports folder, which may not exist yet.
        if shutil.disk_usage(exports_root().parent).free < chat_store_bytes() + RESERVE_BYTES:
            return error(507, "notEnoughSpace")
        return started("survey", ["survey", "--everything", "--no-report"], survey_view)


def estimate(found, brains):
    """About how much the export copies, from the survey."""
    workspace = found.get("workspace") if isinstance(found.get("workspace"), dict) else {}
    chosen = sum(brain["bytes"] or 0 for brain in offered(found)[0] if brain["id"] in brains)
    return count(workspace.get("bytes")) + chosen + chat_store_bytes()


def _start_export(brains, sites):
    with _lock:
        reap()
        sweep()
        if busy():
            return error(409, "busy")
        survey = job(exports_root() / "survey")
        if survey is None or survey["state"] != "ready":
            return error(409, "surveyFirst")
        offered_brains, offered_sites = offered(survey["result"])
        if not set(brains) <= {brain["id"] for brain in offered_brains} or not set(sites) <= {
            site["slug"] for site in offered_sites
        }:
            return error(422, "notOffered")
        needed = 2 * estimate(survey["result"], brains) + RESERVE_BYTES
        if shutil.disk_usage(exports_root()).free < needed:
            return error(507, "notEnoughSpace")
        folder = exports_root() / "export"
        argv = ["build", "--everything", "--no-report", "--out", str(folder / "out")]
        for brain in brains:
            argv += ["--brain", brain]
        for site in offered_sites:
            if not site["owned"] and site["slug"] in sites:
                argv += ["--site", site["slug"]]
            if site["owned"] and site["slug"] not in sites:
                argv += ["--leave-out-site", site["slug"]]
        return started("export", argv, export_view, choices={"brains": brains, "sites": sites})


def _delete_export():
    with _lock:
        folder = exports_root() / "export"
        current = job(folder)
        if current is not None and current["state"] == "running":
            with suppress(ProcessLookupError):
                # The wrapper leads its own session: this stops the exporter too.
                os.killpg(current["record"]["pid"], signal.SIGKILL)
        shutil.rmtree(folder, ignore_errors=True)
        reap()
        return JSONResponse({"version": 1}, headers=HEADERS)


def _ready_archive():
    with _lock:
        sweep()
        current = job(exports_root() / "export")
        return archive(current) if current and current["state"] == "ready" else None


def no_input(request):
    return not request.query_params and request.headers.get("content-length", "0") == "0"


def choice_list(value):
    if (
        not isinstance(value, list)
        or len(value) > 500
        or not all(isinstance(item, str) and NAME.fullmatch(item) for item in value)
        or len(set(value)) != len(value)
    ):
        raise ValueError("invalid choices")
    return value


async def choices(request):
    """The body `{"brains": [ids], "sites": [slugs]}`, or None."""
    if request.query_params:
        return None
    if request.headers.get("content-type", "").split(";")[0].strip() != "application/json":
        return None
    body = await request.body()
    if len(body) > MAX_BODY_BYTES:
        return None
    try:
        value = json.loads(body)
        if not isinstance(value, dict) or set(value) != {"brains", "sites"}:
            return None
        return choice_list(value["brains"]), choice_list(value["sites"])
    except (ValueError, RecursionError):
        return None


@router.get("/status")
async def status(request: Request):
    if not no_input(request):
        return error(400, "invalidInput")
    return await asyncio.to_thread(_status)


@router.post("/survey")
async def start_survey(request: Request):
    if not no_input(request):
        return error(400, "invalidInput")
    return await asyncio.to_thread(_start_survey)


@router.post("/export")
async def start_export(request: Request):
    chosen = await choices(request)
    if chosen is None:
        return error(400, "invalidInput")
    return await asyncio.to_thread(_start_export, *chosen)


@router.delete("/export")
async def delete_export(request: Request):
    if not no_input(request):
        return error(400, "invalidInput")
    return await asyncio.to_thread(_delete_export)


@router.get("/download")
async def download(request: Request):
    if not no_input(request):
        return error(400, "invalidInput")
    path = await asyncio.to_thread(_ready_archive)
    if path is None:
        return error(404, "notReady")
    # Starlette answers Range requests, so a browser can resume a download.
    return FileResponse(path, media_type="application/zip", filename=path.name, headers=HEADERS)
