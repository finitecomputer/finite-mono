"""Owner-only topics layered on the pinned SimpleX adapter, without a second socket.

The daemon owns membership/identity. This plugin alone writes topics.json; only
exact recorded groups are enabled. PairingStore remains the authority for the
owner. No new approval store, group links, or Home-channel writes.
"""

import asyncio
import concurrent.futures
import hashlib
import importlib
import json
import logging
import os
import re
import uuid
from dataclasses import fields
from pathlib import Path

from gateway.pairing import PairingStore
from gateway.session_context import get_session_env


def sole_owner():
    approved = PairingStore().list_approved("simplex")
    if len(approved) != 1 or not str(approved[0]["user_id"]).isdecimal():
        raise ValueError("Topic groups require exactly one approved SimpleX owner contact")
    return str(approved[0]["user_id"])


def write_state(path, value):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    temporary = path.with_suffix(".tmp")
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as stream:
        json.dump(value, stream)
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)
    fd = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


class OwnerTopics:
    """Mixin: inherited adapter owns transport, batching, sessions and media."""

    def __init__(self, config, **kwargs):
        super().__init__(config, **kwargs)
        self.topic_path = (
            Path(os.environ.get("FINITECHAT_HOME", "/data/agent")) / "simplex/topics.json"
        )
        self.topic_error = False
        try:
            self.topics = (
                json.loads(self.topic_path.read_text()) if self.topic_path.exists() else {}
            )
            if not isinstance(self.topics, dict) or any(
                not isinstance(row, dict)
                or not isinstance(row.get("owner"), str)
                or not row["owner"].isdecimal()
                or (
                    "group_id" in row
                    and (not isinstance(row["group_id"], str) or not row["group_id"].isdecimal())
                )
                for row in self.topics.values()
            ):
                raise ValueError("Invalid retained SimpleX topic registry")
        except (ValueError, OSError):
            # Do not replace corrupt state or take down existing private DMs.
            logging.getLogger(__name__).exception("Owner topic registry is unavailable")
            self.topics = {}
            self.topic_error = True
        self.group_allow_from = {r["group_id"] for r in self.topics.values() if r.get("group_id")}
        self.topic_lock = asyncio.Lock()
        self.admission_lock = asyncio.Lock()
        self.admissions = set()
        self.topic_loop = None

    async def connect(self, **kwargs):
        self.topic_loop = asyncio.get_running_loop()
        return await super().connect(**kwargs)

    async def disconnect(self):
        for task in list(self.admissions):
            task.cancel()
        await asyncio.gather(*self.admissions, return_exceptions=True)
        self.topic_loop = None
        await super().disconnect()

    async def command(self, command):
        response = await super()._send_command(command, timeout=15)
        if not response or response.get("type") in ("chatCmdError", "chatError"):
            raise RuntimeError("SimpleX could not confirm the operation; retry the same topic")
        return response

    def topic(self, group_id):
        rows = [r for r in self.topics.values() if r.get("group_id") == group_id]
        if len(rows) != 1 or rows[0].get("blocked") or rows[0]["owner"] != sole_owner():
            raise ValueError("This is not an active owner-only topic")
        return rows[0]

    async def members(self, group_id):
        result = await self.command(f"/_members #{group_id}")
        if result.get("type") != "groupMembers":
            raise ValueError("Cannot verify SimpleX membership")
        return result["group"]

    async def check_group(self, group_id, *, sender=None):
        row = self.topic(group_id)
        group = await self.members(group_id)
        own = group["groupInfo"]["membership"]
        members = group["members"]
        # The owner is deliberately a member, never admin: only the agent can
        # invite. Include pending/removed members; any unexpected history closes
        # the topic permanently, rather than silently reopening private context.
        valid = (
            own["memberRole"] == "owner"
            and len(members) == 1
            and str(members[0].get("memberContactId")) == row["owner"]
            and members[0]["memberRole"] == "member"
            and members[0]["memberStatus"] in ("invited", "accepted", "connected", "complete")
        )
        if not valid:
            row["blocked"] = True
            write_state(self.topic_path, self.topics)
            raise ValueError("Topic membership changed; owner-private processing is disabled")
        if sender is not None and str(members[0]["memberId"]) != sender:
            raise ValueError("Message is not from this topic's owner")
        return row["owner"]

    async def handle_message(self, event):
        if event.source.chat_type != "group":
            return await super().handle_message(event)
        # This method can run inside upstream's socket reader. Admission must
        # not wait there for a command response that only that reader can consume.
        task = asyncio.create_task(self.admit(event))
        self.admissions.add(task)
        task.add_done_callback(self.admissions.discard)

    async def admit(self, event):
        async with self.admission_lock:
            try:
                owner = await self.check_group(
                    event.source.chat_id.removeprefix("group:"), sender=event.source.user_id
                )
            except (ValueError, RuntimeError, KeyError, TypeError):
                return  # Fail closed, including unknown response shapes.
            # Preserve raw_message's member identity; canonical auth/context uses
            # the verified contact ID, so ordinary Hermes DM approval still rules.
            event.source.user_id = owner
            await super().handle_message(event)

    async def guard_send(self, command):
        target = re.match(r"/_send #([0-9]+)(?:\s|$)", command)
        if target:
            await self.check_group(target[1])

    async def _send_ws(self, payload):
        await self.guard_send(payload.get("cmd", ""))
        return await super()._send_ws(payload)

    async def _send_command(self, command, **kwargs):
        await self.guard_send(command)
        return await super()._send_command(command, **kwargs)

    async def create_topic(self, title, owner):
        if self.topic_error:
            raise ValueError("Retained topic state needs repair; no groups were changed")
        if owner != sole_owner():
            raise ValueError("Only the paired owner can create a topic")
        if (
            not isinstance(title, str)
            or not 1 <= len(title.strip()) <= 80
            or any(ord(c) < 32 for c in title)
        ):
            raise ValueError("Use a topic name of 1-80 characters without control characters")
        title = title.strip()
        key = hashlib.sha256((owner + ":" + title.casefold()).encode()).hexdigest()
        async with self.topic_lock:
            row = self.topics.get(key)
            if row is None:
                user = (await self.command("/u"))["user"]["userId"]
                row = {
                    "owner": owner,
                    "title": title,
                    "marker": "Finite owner topic " + uuid.uuid4().hex,
                }
                self.topics[key] = row
                write_state(self.topic_path, self.topics)
                # Journal before the external mutation. An uncertain create is
                # reconciled by its unique description, never blindly repeated.
                row["creating"] = True
                write_state(self.topic_path, self.topics)
                profile = {"displayName": title, "fullName": "", "description": row["marker"]}
                result = await self.command(f"/_group {user} incognito=off " + json.dumps(profile))
                if result.get("type") != "groupCreated":
                    raise ValueError("SimpleX did not confirm group creation")
                row["group_id"] = str(result["groupInfo"]["groupId"])
                write_state(self.topic_path, self.topics)
            if not row.get("group_id"):
                result = await self.command("/groups")
                groups = [g[0] if isinstance(g, list) else g for g in result.get("groups", [])]
                matches = [
                    g
                    for g in groups
                    if g.get("groupProfile", {}).get("description") == row["marker"]
                ]
                if len(matches) != 1:
                    raise ValueError(
                        "Previous creation is unconfirmed; refusing to create a duplicate topic"
                    )
                row["group_id"] = str(matches[0]["groupId"])
                write_state(self.topic_path, self.topics)
            group_id = row["group_id"]
            self.topic(group_id)
            group = await self.members(group_id)
            if not row.get("named"):
                # Clear the reconciliation marker before inviting the owner.
                profile = {
                    "displayName": row["title"],
                    "fullName": "",
                    "description": "A private topic with your agent.",
                }
                await self.command(f"/_group_profile #{group_id} " + json.dumps(profile))
                row["named"] = True
                write_state(self.topic_path, self.topics)
            if not group["members"]:
                # An interrupted invitation is inspected on retry. Only the
                # paired contact is ever passed to /_add; never a model argument.
                await self.command(f"/_add #{group_id} {owner} member")
            await self.check_group(group_id)
            self.group_allow_from.add(group_id)
            return {
                "group_id": group_id,
                "topic": row["title"],
                "status": "invited",
                "home_unchanged": True,
            }


def register(ctx):
    from gateway.platform_registry import platform_registry

    entry = platform_registry.get("simplex")
    if entry is None:
        return  # Finite Chat remains usable in Hermes installations without SimpleX.
    upstream = importlib.import_module(entry.adapter_factory.__module__)

    class TopicAdapter(OwnerTopics, upstream.SimplexAdapter):
        pass

    live = []

    def factory(config):
        if config.extra.get("finite_managed") is not True:
            return entry.adapter_factory(config)
        adapter = TopicAdapter(config)
        live[:] = [adapter]
        return adapter

    metadata = {
        f.name: getattr(entry, f.name)
        for f in fields(entry)
        if f.name not in {"source", "plugin_name", "adapter_factory"}
    }
    ctx.register_platform(**metadata, adapter_factory=factory)

    def create(args, **_kwargs):
        try:
            owner = sole_owner()
            if (
                get_session_env("HERMES_SESSION_PLATFORM") != "simplex"
                or get_session_env("HERMES_SESSION_CHAT_TYPE") != "dm"
                or get_session_env("HERMES_SESSION_USER_ID") != owner
                or get_session_env("HERMES_SESSION_CHAT_ID") != owner
            ):
                raise ValueError(
                    "Ask to create a topic in your approved private SimpleX conversation"
                )
            if not live or live[0].topic_loop is None:
                raise ValueError("Managed SimpleX is not connected")
            future = asyncio.run_coroutine_threadsafe(
                live[0].create_topic(args.get("topic"), owner), live[0].topic_loop
            )
            try:
                return json.dumps(future.result(timeout=75))
            except concurrent.futures.TimeoutError:
                future.cancel()
                raise ValueError("Creation is still unconfirmed; retry the same topic") from None
        except (ValueError, RuntimeError, KeyError, TypeError) as exc:
            return json.dumps({"error": str(exc)})

    ctx.register_tool(
        name="simplex_create_topic",
        toolset="simplex",
        handler=create,
        description="Create an owner-only SimpleX topic and invite the paired owner",
        schema={
            "name": "simplex_create_topic",
            "description": "When the owner explicitly asks in their private SimpleX DM, create a topic group and invite them. Same topic name reuses the group. Keeps Home unchanged. Only the agent and owner are supported; no other invitees.",
            "parameters": {
                "type": "object",
                "properties": {"topic": {"type": "string"}},
                "required": ["topic"],
                "additionalProperties": False,
            },
        },
    )
