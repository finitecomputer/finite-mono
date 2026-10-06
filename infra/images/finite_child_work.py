"""Durable Finite Chat child-work ownership for the pinned Hermes gateway.

The inbox owns foreground replay. This journal owns only accepted child work.
Execution is never replayed after it may have performed effects. Delivery is
at least once: a crash after transport success but before commit can repeat a
reply, but cannot repeat model/tool execution. Keep this file in Recovery Sets.
"""

import asyncio
import contextvars
import hashlib
import json
import logging
import os
import shutil
import sqlite3
from pathlib import Path

logger = logging.getLogger(__name__)
_capture = contextvars.ContextVar("finite_child_delivery", default=None)
SEND_METHODS = frozenset(
    {"send", "send_image", "send_image_file", "send_voice", "send_video", "send_document"}
)
CONTRACT = "finite-child-work-v1"


class Journal:
    def __init__(self, home):
        self.home = Path(home)
        path = self.home / "finite-child-work.sqlite3"
        path.parent.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(path)
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA synchronous=FULL")
        self.db.execute("""CREATE TABLE IF NOT EXISTS work (
            id TEXT PRIMARY KEY, kind TEXT NOT NULL, state TEXT NOT NULL,
            payload TEXT NOT NULL, deliveries TEXT NOT NULL DEFAULT '[]',
            delivered INTEGER NOT NULL DEFAULT 0)""")
        self.db.commit()
        path.chmod(0o600)

    def get(self, key):
        row = self.db.execute("SELECT * FROM work WHERE id=?", (key,)).fetchone()
        return dict(row) if row else None

    def accept(self, key, kind, payload):
        with self.db:
            if self.get(key):
                return False
            if self.count() >= 4096:
                raise RuntimeError(
                    "Too many pending child outcomes; resolve delivery before accepting more work"
                )
            self.db.execute(
                "INSERT INTO work(id,kind,state,payload) VALUES(?,?,'accepted',?)",
                (key, kind, json.dumps(payload)),
            )
        return True

    def update(self, key, state, *, payload=None, deliveries=None, delivered=None):
        fields, values = ["state=?"], [state]
        for name, value in (
            ("payload", payload),
            ("deliveries", deliveries),
            ("delivered", delivered),
        ):
            if value is not None:
                fields.append(name + "=?")
                values.append(json.dumps(value) if name != "delivered" else value)
        with self.db:
            self.db.execute("UPDATE work SET " + ",".join(fields) + " WHERE id=?", (*values, key))

    def pending(self):
        return [
            dict(row)
            for row in self.db.execute(
                "SELECT * FROM work WHERE state!='done' ORDER BY rowid LIMIT 4096"
            )
        ]

    def count(self):
        return self.db.execute("SELECT count(*) FROM work WHERE state!='done'").fetchone()[0]


def journal(runner):
    if not hasattr(runner, "_finite_child_journal"):
        from gateway.run import _hermes_home

        runner._finite_child_journal = Journal(_hermes_home)
        runner._finite_child_live = set()
    return runner._finite_child_journal


def finite(runner, source):
    return bool(getattr(runner._adapter_for_source(source), "durable_child_work", False))


def identity(event, kind):
    if not event.message_id:
        raise RuntimeError("Child work requires a durable command identity")
    source = event.source
    return hashlib.sha256(
        json.dumps(
            [
                kind,
                source.platform.value,
                source.chat_id,
                source.thread_id,
                source.user_id,
                getattr(source, "profile", None),
                event.message_id,
            ]
        ).encode()
    ).hexdigest()


class CaptureAdapter:
    def __init__(self, adapter, deliveries):
        self.adapter, self.deliveries = adapter, deliveries

    def __getattr__(self, name):
        if name not in SEND_METHODS:
            return getattr(self.adapter, name)

        async def capture(*args, **kwargs):
            from gateway.platforms.base import SendResult

            item = {"method": name, "args": args, "kwargs": kwargs}
            json.dumps(item)  # Reject non-durable payloads before claiming success.
            if len(self.deliveries) >= 256:
                raise RuntimeError("Child result exceeds delivery limit")
            self.deliveries.append(item)
            return SendResult(success=True, message_id="durable-child-pending")

        return capture


def capture_adapter(adapter):
    deliveries = _capture.get()
    return (
        CaptureAdapter(adapter, deliveries)
        if adapter is not None and deliveries is not None
        else adapter
    )


def interrupted(store, row):
    payload = json.loads(row["payload"])
    source = payload["source"]
    if row["kind"] == "goal":
        from hermes_cli.goals import GoalManager

        manager = GoalManager(session_id=payload["session_id"])
        if manager.state and json.loads(manager.state.to_json()) == payload["goal"]:
            manager.pause(reason="runtime-interrupted; effects require review")
    notice = (
        f"{row['kind']} work was interrupted by a runtime stop or restart. "
        "It was not automatically rerun because effects may already have occurred. "
        "Review its effects before requesting another run."
    )
    retained = json.loads(row["deliveries"])
    store.update(
        row["id"],
        "outcome",
        deliveries=[
            *retained,
            {
                "method": "send",
                "args": [source["chat_id"], notice],
                "kwargs": {"metadata": payload.get("metadata")},
            },
        ],
    )


async def deliver(runner, key):
    from gateway.session import SessionSource

    store = journal(runner)
    row = store.get(key)
    if row["state"] != "outcome":
        return
    payload = json.loads(row["payload"])
    source = SessionSource.from_dict(payload["source"])
    if not runner._is_user_authorized(source):
        return
    adapter = runner._adapter_for_source(source)
    if adapter is None:
        return
    deliveries = json.loads(row["deliveries"])
    for i in range(row["delivered"], len(deliveries)):
        item = deliveries[i]
        if item["method"] not in SEND_METHODS:
            raise RuntimeError("Unsupported durable delivery")
        result = await getattr(adapter, item["method"])(*item["args"], **item["kwargs"])
        if not getattr(result, "success", False):
            return
        store.update(key, "outcome", delivered=i + 1)
    store.update(key, "done")
    runner._persist_active_agents()


def start(runner, event, kind, callback):
    """Commit ownership synchronously, before task creation and command ack."""
    if not finite(runner, event.source):
        task = asyncio.create_task(callback())
    else:
        store = journal(runner)
        key = identity(event, kind)
        payload = {
            "source": event.source.to_dict(),
            "metadata": runner._thread_metadata_for_source(
                event.source, runner._reply_anchor_for_event(event)
            ),
        }
        if not store.accept(key, kind, payload):
            return
        runner._finite_child_live.add(key)
        runner._persist_active_agents()

        async def run():
            deliveries = []
            token = _capture.set(deliveries)
            try:
                # This commit precedes any model/tool effect.
                store.update(key, "running")
                await callback()
                retain_media(store, key, deliveries)
                store.update(key, "outcome", deliveries=deliveries)
            except BaseException:
                interrupted(store, store.get(key))
                raise
            finally:
                _capture.reset(token)
            try:
                await deliver(runner, key)
            finally:
                runner._finite_child_live.discard(key)

        task = asyncio.create_task(run())
        # Also release ownership if cancelled before the coroutine ever ran.
        task.add_done_callback(lambda _task: runner._finite_child_live.discard(key))
        schedule_recovery(runner)
    runner._background_tasks.add(task)
    task.add_done_callback(runner._background_tasks.discard)


def active_count(runner):
    store = getattr(runner, "_finite_child_journal", None)
    count = store.count() if store is not None else 0
    # Hermes also owns async delegations/process watchers outside the inbox.
    return count + int(runner._scale_to_zero_has_live_background_work())


def schedule_recovery(runner):
    """One bounded, retrying outbox owner per live gateway; never re-executes bg/btw."""
    task = getattr(runner, "_finite_child_recovery", None)
    if task is not None and not task.done():
        return
    if not any(
        getattr(adapter, "durable_child_work", False) for adapter in runner.adapters.values()
    ):
        return
    store = journal(runner)
    runner._persist_active_agents()

    async def recover():
        delay = 1
        while not getattr(runner, "_draining", False):
            for row in store.pending():
                if row["kind"] == "goal" and row["state"] == "queued":
                    from hermes_cli.goals import load_goal

                    payload = json.loads(row["payload"])
                    state = load_goal(payload["session_id"])
                    if state is None or json.loads(state.to_json()) != payload["goal"]:
                        store.update(row["id"], "done")
                        runner._finite_child_live.discard(row["id"])
                        continue
                if row["id"] in runner._finite_child_live:
                    continue
                try:
                    if row["kind"] == "goal" and row["state"] == "queued":
                        await resume_goal(runner, row)
                    elif row["state"] in {"accepted", "running"}:
                        interrupted(store, row)
                    await deliver(runner, row["id"])
                except Exception:
                    logger.exception("Child work recovery failed for %s", row["id"])
            await asyncio.sleep(delay)
            delay = min(delay * 2, 30)

    task = asyncio.create_task(recover())
    task._hermes_supervised_watcher = True
    runner._finite_child_recovery = task
    runner._background_tasks.add(task)
    task.add_done_callback(runner._background_tasks.discard)


def save_goal(runner, event, child, session_id):
    if not finite(runner, event.source):
        return
    from hermes_cli.goals import load_goal

    store = journal(runner)
    state = load_goal(session_id)
    key = (
        identity(event, "goal")
        if event.message_id
        else hashlib.sha256(
            json.dumps(
                [session_id, json.loads(state.to_json()) if state else None], sort_keys=True
            ).encode()
        ).hexdigest()
    )
    if state is None:
        raise RuntimeError("Goal continuation requires persisted goal state")
    payload = {
        "source": event.source.to_dict(),
        "session_id": session_id,
        "goal": json.loads(state.to_json()),
        "text": child.text,
        "metadata": runner._thread_metadata_for_source(event.source, event.message_id),
    }
    if store.accept(key, "goal", payload):
        store.update(key, "queued")
    child._finite_goal_work = key
    runner._finite_child_live.add(key)
    runner._persist_active_agents()
    schedule_recovery(runner)


def begin_goal(runner, event):
    key = getattr(event, "_finite_goal_work", None)
    if key:
        journal(runner).update(key, "running")


def end_goal(runner, event, completed):
    key = getattr(event, "_finite_goal_work", None)
    if key:
        store = journal(runner)
        row = store.get(key)
        if row["state"] == "outcome" and completed:
            if row["delivered"] == len(json.loads(row["deliveries"])):
                store.update(key, "done")
        elif row["state"] == "running":
            if completed:
                store.update(key, "done")
            else:
                interrupted(store, row)
        runner._finite_child_live.discard(key)
        runner._persist_active_agents()


async def resume_goal(runner, row):
    from gateway.platforms.base import MessageEvent
    from gateway.session import SessionSource
    from hermes_cli.goals import load_goal

    payload = json.loads(row["payload"])
    source = SessionSource.from_dict(payload["source"])
    state = load_goal(payload["session_id"])
    # A pause/clear/replacement/gate/wait after acceptance remains authoritative.
    if state is None or json.loads(state.to_json()) != payload["goal"] or state.status != "active":
        journal(runner).update(row["id"], "done")
        return
    if not runner._is_user_authorized(source):
        return
    adapter = runner._adapter_for_source(source)
    key = runner._session_key_for_source(source)
    if (
        adapter is None
        or runner._is_session_running(key)
        or adapter._session_is_active(key)
        or getattr(runner, "_startup_restore_in_progress", False)
        or getattr(adapter, "_deferred_admissions", {}).get(key)
    ):
        return
    event = MessageEvent(text=payload["text"], source=source, internal=True)
    event._finite_goal_work = row["id"]
    runner._finite_child_live.add(row["id"])
    await adapter.handle_message(event)


def prior_goal(runner, event):
    return bool(
        finite(runner, event.source)
        and event.message_id
        and journal(runner).get(identity(event, "goal"))
    )


def retain_media(store, key, deliveries):
    """Result attachments must outlive the worker's temporary directory."""
    directory = store.home / "finite-child-results" / key
    total = 0
    for i, item in enumerate(deliveries):
        for field in ("image_path", "audio_path", "video_path", "file_path"):
            value = item["kwargs"].get(field)
            if not value:
                continue
            source = Path(value)
            size = source.stat().st_size
            total += size
            if size > 32 * 1024 * 1024 or total > 128 * 1024 * 1024:
                raise RuntimeError("Child result media exceeds retention limit")
            directory.mkdir(parents=True, exist_ok=True, mode=0o700)
            target = directory / (str(i) + "-" + source.name)
            with source.open("rb") as src, target.open("wb") as dst:
                shutil.copyfileobj(src, dst)
                dst.flush()
                os.fsync(dst.fileno())
            item["kwargs"][field] = str(target)
    if directory.exists():
        fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)


def goal_result(runner, key, delivery):
    """Own the goal's final text before transport can acknowledge its result."""
    store = journal(runner)
    if store.get(key)["state"] == "queued":
        return False  # A drain refused launch; retain the unstarted continuation.
    store.update(key, "outcome", deliveries=[delivery])
    return True
