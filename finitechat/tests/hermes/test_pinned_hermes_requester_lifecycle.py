"""Requester leases across the pinned Hermes plugin lifecycle.

A forced plugin rediscovery imports the Finite Chat plugin again, so the
running adapter and the hooks registered afterwards come from different
module copies. These tests drive the pinned loader, Gateway, hook dispatch
and terminal tool through that reload. Inference is a loopback fake and
Python name resolution is restricted to loopback while each test runs.
"""

import asyncio
import contextlib
import errno
import hashlib
import http.server
import io
import json
import os
import shlex
import shutil
import socket
import sqlite3
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

import tools.tirith_security
from gateway.config import GatewayConfig, Platform, PlatformConfig
from gateway.platform_registry import platform_registry
from gateway.platforms.base import SendResult
from gateway.run import GatewayRunner
from gateway.session import SessionSource, build_session_context
from hermes_cli import plugins
from model_tools import handle_function_call

REPO_ROOT = Path(__file__).resolve().parents[2]
PLUGIN_SRC = REPO_ROOT / "integrations" / "hermes" / "finitechat"
ALICE = "a1" * 32
BOB = "b2" * 32
WAIT_SECS = 90
SENDER = object()
CHILD_PRELUDE = (
    "import socket, sys\n"
    "def guard(host, *a, _o=socket.getaddrinfo, **k):\n"
    "    if host not in {None, 'localhost', '127.0.0.1', '::1'}:\n"
    "        raise socket.gaierror(socket.EAI_NONAME, 'network disabled')\n"
    "    return _o(host, *a, **k)\n"
    "socket.getaddrinfo = guard\n"
    "from hermes_cli import plugins\n"
)
SESSION_DIGEST = shlex.join(
    [
        sys.executable,
        "-c",
        "import hashlib, os; "
        "print(hashlib.sha256(os.environ.get('HERMES_SESSION_KEY', '').encode()).hexdigest())",
    ]
)


def lease_check(directory: str, label: str) -> str:
    """Shell that reports whether the session's lease for its user is in `directory`."""
    return (
        f"k=$({SESSION_DIGEST}); "
        f'f="$FINITE_HOME/{directory}/$k.json"; '
        'if [ -f "$f" ] && grep -q "\\"requesting_user_id\\":\\"$HERMES_SESSION_USER_ID\\"" "$f"; '
        f"then echo {label}_PRESENT; else echo {label}_ABSENT; fi"
    )


LEASE_CHECK = lease_check("requester-context-v1", "LEASE")
BOTH_LEASES_CHECK = f"{LEASE_CHECK}; {lease_check('requester-context-v2', 'V2')}"
SESSION_ECHO = 'echo "SESSION=$HERMES_SESSION_PLATFORM|$HERMES_SESSION_KEY|$HERMES_SESSION_USER_ID"'
BLOCKED = "Terminal is unavailable in this Finite Chat session"
STATE_MODULE = "_finitechat_requester_state_v1"


def lease_name(session_key: str) -> str:
    return hashlib.sha256(session_key.encode("utf-8")).hexdigest() + ".json"


def wait_until(predicate, what: str) -> None:
    deadline = time.monotonic() + WAIT_SECS
    while not predicate():
        if time.monotonic() > deadline:
            raise AssertionError(f"timed out waiting for {what}")
        time.sleep(0.02)


def loopback_only(original):
    def resolve(host, *args, **kwargs):
        if host not in {None, "localhost", "127.0.0.1", "::1"}:
            raise socket.gaierror(socket.EAI_NONAME, "network disabled in requester tests")
        return original(host, *args, **kwargs)

    return resolve


def kill_gateway_mid_call(session_key: str, user: str, v2=None) -> None:
    """A child gateway takes a terminal lease, then is SIGKILLed mid-call."""
    gateway = (
        CHILD_PRELUDE + "import os, signal\n"
        "from gateway.config import GatewayConfig\n"
        "from gateway.run import GatewayRunner\n"
        "from gateway.session import SessionSource, build_session_context\n"
        "plugins.discover_plugins()\n"
        "adapter = next(m for n, m in sys.modules.items() if n.endswith('finitechat.adapter'))\n"
        "source = SessionSource(platform=adapter._finite_platform(), chat_id='room-1',\n"
        f"    chat_type='group', user_id={user!r}, thread_id='seg-1')\n"
        "context = build_session_context(source, GatewayConfig())\n"
        f"context.session_key = {session_key!r}\n"
        "runner = object.__new__(GatewayRunner)\n"
        "runner._set_session_env(context)\n"
        f"adapter._AUTHENTICATED_FINITE_TURN_USER.set({user!r})\n"
        f"adapter._AUTHENTICATED_FINITE_REQUESTER_CONTEXT.set({v2!r})\n"
        "plugins.invoke_hook('pre_tool_call', tool_name='terminal', tool_call_id='killed')\n"
        "os.kill(os.getpid(), signal.SIGKILL)\n"
    )
    result = subprocess.run(
        [sys.executable, "-c", gateway], capture_output=True, text=True, timeout=WAIT_SECS
    )
    assert result.returncode == -9, result.stderr


@contextlib.contextmanager
def unlink_fails(*names: tuple[str, str]):
    """Removing these (lease directory, file name) entries fails until exit."""
    unlink = Path.unlink

    def guarded(path, *args, **kwargs):
        if (path.parent.name, path.name) in names:
            raise PermissionError(errno.EACCES, "Permission denied", str(path))
        return unlink(path, *args, **kwargs)

    with patch.object(Path, "unlink", guarded):
        yield


def blocked_message(result: str) -> str:
    """The error the pinned dispatcher returned for a blocked tool call."""
    error = json.loads(result).get("error")
    assert isinstance(error, str) and error.startswith(BLOCKED), result
    return error


def leases_in(finite_home: Path) -> list[str]:
    return sorted(
        p.name
        for root in ("requester-context-v1", "requester-context-v2")
        if (finite_home / root).exists()
        for p in (finite_home / root).iterdir()
    )


def tree_snapshot(top: Path) -> dict[str, tuple]:
    """Every entry under `top`, links not followed: type, mode, contents' size and mtime."""
    snapshot = {}
    for directory, dirs, files in os.walk(top):
        for name in dirs + files:
            info = (Path(directory) / name).lstat()
            # A directory's mtime moves with its entries; compare files only.
            written = None if stat.S_ISDIR(info.st_mode) else (info.st_size, info.st_mtime_ns)
            snapshot[os.path.join(directory, name)] = (info.st_mode, written)
    return snapshot


def plant_leftovers(finite_home: Path) -> tuple[list[str], list[str]]:
    """Lease-named entries no reader accepts in both lease directories.

    Returns every planted path and those standing for unreadable files; tests
    fault opening those rather than relying on the uid they run as.
    """
    outside = finite_home / "outside.json"
    outside.write_text(json.dumps({"expires_at_unix": 0}))
    contents = {
        "list": b"[]",
        "null": b"null",
        "scalar": b"7",
        "string": b'"x"',
        "nested": b"[" * 100_000,
        "bad-bytes": b"\xff\xfe\x00",
        "oversized": b'{"expires_at_unix":0,"pad":"' + b"x" * (4 << 20) + b'"}',
        "unreadable": json.dumps({"expires_at_unix": 0}).encode(),
    }
    planted, unreadable = [], []
    for root in (finite_home / "requester-context-v1", finite_home / "requester-context-v2"):
        root.mkdir(mode=0o700, exist_ok=True)
        for kind, content in contents.items():
            path = root / lease_name(f"{root.name}:{kind}")
            path.write_bytes(content)
            planted.append(str(path))
        fifo, link, directory = (
            root / lease_name(f"{root.name}:{kind}") for kind in ("fifo", "symlink", "directory")
        )
        os.mkfifo(fifo)
        link.symlink_to(outside)
        directory.mkdir()
        planted += [str(fifo), str(link), str(directory)]
        unreadable.append(str(root / lease_name(f"{root.name}:unreadable")))
    return planted, unreadable


def as_new_gateway_process() -> Path:
    """Forget that this process's gateway has connected for the module home.

    A gateway started after a crash is a new process with no requester
    state. The pinned terminal keeps the first FINITE_HOME for the process
    lifetime, so the tests reuse that home and drop only the record of the
    completed startup cleanup.
    """
    state = sys.modules[STATE_MODULE]
    state.started_roots.discard(os.path.realpath(HOME.v1))
    getattr(state, "quarantine", {}).pop(os.path.realpath(HOME.v1), None)
    return HOME.finite_home


class PluginHome:
    """An isolated Hermes home with the Finite Chat plugin installed.

    One home serves the module: the pinned terminal and logging keep the
    first home's paths for the process lifetime.
    """

    def __init__(self, model_url: str):
        scratch = Path(tempfile.mkdtemp(prefix="fin117-"))
        unittest.addModuleCleanup(shutil.rmtree, scratch, True)
        self.scratch = scratch
        self.hermes_home = scratch / "hermes-home"
        self.finite_home = scratch / "agent"
        workspace = scratch / "workspace"
        for directory in (self.hermes_home / "plugins", self.finite_home, workspace):
            directory.mkdir(parents=True)
        shutil.copytree(
            PLUGIN_SRC,
            self.hermes_home / "plugins" / "finitechat",
            ignore=shutil.ignore_patterns("__pycache__"),
        )
        (self.hermes_home / "config.yaml").write_text(
            json.dumps(
                {
                    "model": {
                        "default": "fake",
                        "provider": "custom",
                        "base_url": model_url,
                        "api_key": "sk-fake",
                        "api_mode": "chat_completions",
                        "context_length": 200000,
                    },
                    "terminal": {"backend": "local", "cwd": str(workspace)},
                    "approvals": {"mode": "off"},
                    "display": {"streaming": False},
                    "plugins": {"enabled": ["finitechat"]},
                    # A background review would ask the fake model for one
                    # more terminal call after a turn has finished.
                    "memory": {"nudge_interval": 0},
                    "skills": {"creation_nudge_interval": 0},
                    "_config_version": 10,
                }
            )
        )
        self.env = {
            "HERMES_HOME": str(self.hermes_home),
            "FINITE_HOME": str(self.finite_home),
            "FINITECHAT_HOME": str(self.finite_home),
            "FINITECHAT_ALLOW_ALL_USERS": "true",
            "FINITECHAT_BIN": "/bin/echo",
            "GATEWAY_ALLOW_ALL_USERS": "true",
        }
        for patcher in (
            patch.dict(os.environ, self.env),
            patch.object(socket, "getaddrinfo", loopback_only(socket.getaddrinfo)),
            patch.object(tools.tirith_security, "ensure_installed", lambda **_kw: None),
        ):
            patcher.start()
            unittest.addModuleCleanup(patcher.stop)
        self.v1 = self.finite_home / "requester-context-v1"
        self.v2 = self.finite_home / "requester-context-v2"

    def adapter_modules(self) -> list:
        wanted = os.path.realpath(self.hermes_home / "plugins" / "finitechat" / "adapter.py")
        return [
            module
            for module in list(sys.modules.values())
            if os.path.realpath(getattr(module, "__file__", None) or "") == wanted
        ]

    def current_module(self):
        modules = self.adapter_modules()
        assert len(modules) == 1, modules
        return modules[0]

    def leases(self) -> list[str]:
        return sorted(
            p.name for root in (self.v1, self.v2) if root.exists() for p in root.iterdir()
        )


class FakeModel:
    """Loopback chat completions: each user turn runs one terminal command."""

    def __init__(self):
        self.command = LEASE_CHECK
        self.results: list[str] = []
        model = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                last = (body.get("messages") or [{}])[-1]
                if last.get("role") == "tool":
                    model.results.append(str(last.get("content")))
                    message, finish = {"role": "assistant", "content": "done"}, "stop"
                else:
                    call = {
                        "id": f"call_{len(model.results)}",
                        "type": "function",
                        "function": {
                            "name": "terminal",
                            "arguments": json.dumps({"command": model.command}),
                        },
                    }
                    message = {"role": "assistant", "content": None, "tool_calls": [call]}
                    finish = "tool_calls"
                usage = {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
                base = {"id": "x", "created": 0, "model": "fake"}
                if body.get("stream"):
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.end_headers()
                    delta: dict[str, Any] = dict(message)
                    if "tool_calls" in delta:
                        delta["tool_calls"] = [dict(c, index=0) for c in delta["tool_calls"]]
                    chunks: list[dict[str, Any]] = [
                        {"choices": [{"index": 0, "delta": delta, "finish_reason": None}]},
                        {
                            "choices": [{"index": 0, "delta": {}, "finish_reason": finish}],
                            "usage": usage,
                        },
                    ]
                    for chunk in chunks:
                        chunk.update(base, object="chat.completion.chunk")
                        self.wfile.write(f"data: {json.dumps(chunk)}\n\n".encode())
                    self.wfile.write(b"data: [DONE]\n\n")
                    return
                payload = {
                    **base,
                    "object": "chat.completion",
                    "choices": [{"index": 0, "message": message, "finish_reason": finish}],
                    "usage": usage,
                }
                self._json(payload)

            def do_GET(self):
                self._json({"data": [{"id": "fake", "object": "model"}]})

            def _json(self, payload):
                data = json.dumps(payload).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        unittest.addModuleCleanup(server.server_close)
        unittest.addModuleCleanup(server.shutdown)
        self.url = f"http://127.0.0.1:{server.server_address[1]}/v1"


MODEL: FakeModel
HOME: PluginHome
TURN_SEQ = 0


def forget_finitechat_plugin() -> None:
    """Leave later test modules in this process without the loaded plugin.

    Discovery registers `finitechat` as a platform, and the enum caches it,
    so modules that load the adapter directly would otherwise see it.
    """
    plugins._reset_plugin_managers_for_tests()
    platform_registry.unregister("finitechat", scope=None)
    Platform._value2member_map_.pop("finitechat", None)
    Platform._member_map_.pop("FINITECHAT", None)


def setUpModule():
    global MODEL, HOME
    unittest.addModuleCleanup(forget_finitechat_plugin)
    MODEL = FakeModel()
    HOME = PluginHome(MODEL.url)


class LeaseTestCase(unittest.TestCase):
    def setUp(self):
        self.home = HOME
        plugins.discover_plugins()
        MODEL.command = LEASE_CHECK

    def tearDown(self):
        # Leave the shared home clean even when a test fails, so one failure
        # does not fail every later test.
        left = self.home.leases()
        for root in (self.home.v1, self.home.v2):
            if root.exists():
                for path in root.iterdir():
                    shutil.rmtree(path) if path.is_dir() else path.unlink()
        quarantine = getattr(sys.modules[STATE_MODULE], "quarantine", {})
        quarantine.pop(os.path.realpath(self.home.v1), None)
        self.assertEqual(left, [])

    def assert_plugin_registered(self, module) -> None:
        """Hook dispatch reaches `module`'s broker and Gateways get its adapter."""
        hooks = plugins.get_plugin_manager()._hooks
        for name in ("pre_tool_call", "post_tool_call"):
            owners = [getattr(callback, "__self__", None) for callback in hooks.get(name, [])]
            brokers = [
                owner for owner in owners if isinstance(owner, module._RequesterContextBroker)
            ]
            self.assertEqual(len(brokers), 1, name)
        self.assertIsNotNone(platform_registry.get("finitechat"))
        self.assertIs(GatewayHarness().adapter_module, module)


class GatewayHarness:
    """The pinned Gateway driving a Finite Chat adapter from a fake sidecar."""

    def __init__(self, finite_home: Path | None = None, runner: GatewayRunner | None = None):
        self.model = MODEL
        self.home = HOME
        # One session per segment, so different senders share a cached session.
        self.runner = runner or GatewayRunner(
            GatewayConfig(
                sessions_dir=Path(tempfile.mkdtemp(dir=self.home.scratch)),
                group_sessions_per_user=False,
            )
        )
        finite_home = finite_home or self.home.finite_home
        config = PlatformConfig(
            enabled=True, extra={"home": str(finite_home), "finitechat_bin": "/bin/echo"}
        )
        config.typing_indicator = False
        adapter = platform_registry.create_adapter("finitechat", config)
        assert adapter is not None
        self.adapter: Any = adapter
        self.adapter_module: Any = sys.modules[type(adapter).__module__]
        self.adapter.gateway_runner = self.runner
        self.adapter._home_channel_hydrated = True
        self.runner.adapters[self.adapter.platform] = self.adapter

        async def sidecar(_action, _payload, *, timeout):
            _ = timeout
            return self.adapter_module._FiniteChatResult(True, {}, None, False)

        async def service_ready():
            return None

        async def no_inbound():
            return None

        self.adapter._finitechat_json = sidecar
        self.adapter._ensure_service = service_ready
        self.adapter._poll_loop = no_inbound
        self.adapter._stream_loop = no_inbound
        self.sent: list[str] = []

        async def send(*args, **kwargs):
            self.sent.append(str(kwargs.get("content", args[1] if len(args) > 1 else "")))
            return SendResult(success=True, message_id="out")

        self.adapter.send = send
        self.adapter_module._finite_private_control_request = lambda *_a: None
        self.adapter.set_message_handler(self.runner._handle_message)
        self.seq = 0

    async def connect(self, *, reconnect: bool) -> bool:
        """Connect through the pinned Gateway's startup or reconnect path."""
        platform = self.adapter.platform
        if reconnect:
            return await self.runner._connect_adapter_with_timeout(
                self.adapter, platform, is_reconnect=True
            )
        return await self.runner._connect_initial_adapter_with_timeout(self.adapter, platform)

    async def turn(
        self, user: str, segment: str, *, internal: bool = False, v2=None, text: str = ""
    ) -> str:
        global TURN_SEQ
        # Adapters for one home share delivered-event dedup, so IDs never repeat.
        TURN_SEQ += 1
        self.seq = TURN_SEQ
        before = len(self.model.results)
        requester = {}
        if v2 is not None:
            requester = {"requester_email": v2[0], "sites_requester_assertion": v2[1]}
        await self.adapter._handle_finitechat_event(
            {
                **requester,
                "room_id": "room-1",
                "seq": self.seq,
                "message_id": f"msg-{self.seq}",
                "conversation_id": "home",
                "segment_id": segment,
                "text": text or f"turn {self.seq}",
                "message_type": "text",
                "source": {
                    "platform": "finitechat",
                    "chat_id": "room-1",
                    "chat_type": "group",
                    "user_id": user,
                    "thread_id": segment,
                    "is_bot": False,
                },
                "attachments": [],
                "internal": internal,
            }
        )
        deadline = time.monotonic() + WAIT_SECS
        while (
            len(self.model.results) == before
            or self.adapter._session_tasks
            or self.runner._background_tasks
        ):
            if time.monotonic() > deadline:
                raise AssertionError(f"turn {self.seq} did not finish its terminal call")
            await asyncio.sleep(0.05)
        return self.model.results[-1]


class GatewayReloadTests(unittest.IsolatedAsyncioTestCase, LeaseTestCase):
    async def test_authenticated_group_turns_keep_lease_across_forced_plugin_reload(self):
        gateway = GatewayHarness()
        home = gateway.home
        first_module = gateway.adapter_module
        observed = [await gateway.turn(ALICE, "seg-1")]
        plugins.discover_plugins(force=True)
        reloaded = home.current_module()
        observed.append(await gateway.turn(ALICE, "seg-1"))
        plugins.discover_plugins(force=True)
        # Another sender in the same cached session, then a distinct session.
        observed.append(await gateway.turn(BOB, "seg-1"))
        observed.append(await gateway.turn(ALICE, "seg-2"))
        for result in observed:
            self.assertIn("LEASE_PRESENT", result)
        self.assertEqual(home.leases(), [])
        # The adapter kept running the first copy's code while hooks came
        # from the reloaded copy. Only the requester data is shared.
        self.assertIsNot(reloaded, first_module)
        self.assertIs(
            reloaded._AUTHENTICATED_FINITE_TURN_USER, first_module._AUTHENTICATED_FINITE_TURN_USER
        )

        # Internal gateway events carry no authenticated sender.
        self.assertIn("LEASE_ABSENT", await gateway.turn(ALICE, "seg-1", internal=True))
        self.assertEqual(home.leases(), [])
        await gateway.adapter.cancel_background_tasks()


class HookLifecycleTests(LeaseTestCase):
    """Real hook dispatch through the pinned plugin manager."""

    def setUp(self):
        super().setUp()
        self.runner = object.__new__(GatewayRunner)
        self.runner.adapters = {}

    @contextlib.contextmanager
    def turn(self, user: str, session_key: str, *, marker=SENDER, v2=None):
        """Bind the session env and the adapter's markers like a Finite turn."""
        module = self.home.current_module()
        source = SessionSource(
            platform=module._finite_platform(),
            chat_id="room-1",
            chat_type="group",
            user_id=user,
            thread_id="seg-1",
        )
        context = build_session_context(source, GatewayConfig())
        context.session_key = session_key
        tokens = self.runner._set_session_env(context)
        user_token = module._AUTHENTICATED_FINITE_TURN_USER.set(
            user if marker is SENDER else marker
        )
        context_token = module._AUTHENTICATED_FINITE_REQUESTER_CONTEXT.set(v2)
        try:
            yield
        finally:
            module._AUTHENTICATED_FINITE_REQUESTER_CONTEXT.reset(context_token)
            module._AUTHENTICATED_FINITE_TURN_USER.reset(user_token)
            self.runner._clear_session_env(tokens)

    def lease(self, session_key: str, root: Path | None = None) -> dict[str, Any] | None:
        path = (root or self.home.v1) / lease_name(session_key)
        return json.loads(path.read_text()) if path.exists() else None

    def held(self, session_key: str, root: Path | None = None) -> dict[str, Any]:
        lease = self.lease(session_key, root)
        assert lease is not None, f"no lease for {session_key}"
        return lease

    def terminal_in_thread(self, user: str, session_key: str, gate: Path, out: Path, v2=None):
        command = (
            f"touch {shlex.quote(str(gate))}.started; "
            f"while [ ! -e {shlex.quote(str(gate))} ]; do sleep 0.05; done; "
            f"({LEASE_CHECK}) > {shlex.quote(str(out))}"
        )

        def run():
            with self.turn(user, session_key, v2=v2):
                handle_function_call(
                    "terminal",
                    {"command": command, "timeout": WAIT_SECS},
                    task_id=f"task-{gate.name}",
                    tool_call_id=f"call-{gate.name}",
                )

        thread = threading.Thread(target=run)
        thread.start()
        self.addCleanup(thread.join, WAIT_SECS)
        self.addCleanup(gate.touch)
        wait_until(Path(f"{gate}.started").exists, f"{gate.name} to start")
        return thread

    def test_reload_mid_call_keeps_lease_until_last_parallel_call_finishes(self):
        session = "agent:main:finitechat:group:room-1:seg-1"
        gates = Path(tempfile.mkdtemp(dir=self.home.scratch))
        v2 = ("owner@example.com", "assertion-1")
        first = self.terminal_in_thread(ALICE, session, gates / "a", gates / "a.out", v2)
        second = self.terminal_in_thread(ALICE, session, gates / "b", gates / "b.out", v2)
        before = self.home.current_module()
        plugins.discover_plugins(force=True)
        self.assertIsNot(self.home.current_module(), before)
        self.assertEqual(self.held(session)["requesting_user_id"], ALICE)
        self.assertEqual(self.held(session, self.home.v2)["owner_email"], "owner@example.com")

        (gates / "a").touch()
        first.join(WAIT_SECS)
        self.assertEqual((gates / "a.out").read_text().strip(), "LEASE_PRESENT")
        self.assertIsNotNone(self.lease(session))
        self.assertIsNotNone(self.lease(session, self.home.v2))

        (gates / "b").touch()
        second.join(WAIT_SECS)
        self.assertEqual((gates / "b.out").read_text().strip(), "LEASE_PRESENT")
        self.assertEqual(self.home.leases(), [])

    def test_other_process_registration_keeps_an_active_lease(self):
        session = "agent:main:finitechat:group:room-1:seg-1"
        gates = Path(tempfile.mkdtemp(dir=self.home.scratch))
        call = self.terminal_in_thread(ALICE, session, gates / "a", gates / "a.out")
        held = self.lease(session)
        # A CLI process with the same homes registers the plugin and fires
        # stray hooks for the same session without an authenticated turn.
        cli = (
            CHILD_PRELUDE + "plugins.discover_plugins()\n"
            "for hook in ('pre_tool_call', 'post_tool_call'):\n"
            "    plugins.invoke_hook(hook, tool_name='terminal', tool_call_id='call-a')\n"
            "assert any(m.endswith('finitechat.adapter') for m in sys.modules)\n"
        )
        env = {
            **os.environ,
            "HERMES_SESSION_PLATFORM": "local",
            "HERMES_SESSION_KEY": session,
            "HERMES_SESSION_USER_ID": BOB,
        }
        result = subprocess.run(
            [sys.executable, "-c", cli], env=env, capture_output=True, text=True, timeout=WAIT_SECS
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.lease(session), held)

        (gates / "a").touch()
        call.join(WAIT_SECS)
        self.assertEqual((gates / "a.out").read_text().strip(), "LEASE_PRESENT")
        self.assertEqual(self.home.leases(), [])

    def test_sibling_roots_and_concurrent_sessions_keep_separate_counts(self):
        session_a = "agent:main:finitechat:group:room-1:seg-a"
        session_b = "agent:main:finitechat:group:room-1:seg-b"
        module = self.home.current_module()
        scratch = Path(tempfile.mkdtemp(dir=self.home.scratch))
        sibling = module._RequesterContextBroker(scratch / "sibling" / "requester-context-v1")
        alias_root = scratch / "alias"
        alias_root.symlink_to(self.home.finite_home)
        alias = module._RequesterContextBroker(alias_root / "requester-context-v1")
        hook = {"tool_name": "terminal", "tool_call_id": "call-a"}

        with self.turn(ALICE, session_a):
            plugins.invoke_hook("pre_tool_call", **hook)
            plugins.invoke_hook("pre_tool_call", tool_name="terminal", tool_call_id="call-b")
        with self.turn(BOB, session_b):
            plugins.invoke_hook("pre_tool_call", **hook)
        self.assertEqual(self.held(session_a)["requesting_user_id"], ALICE)
        self.assertEqual(self.held(session_b)["requesting_user_id"], BOB)

        with self.turn(ALICE, session_a):
            # A sibling FINITE_HOME holds no call here and removes nothing.
            sibling.after_tool_call(**hook)
            self.assertIsNotNone(self.lease(session_a))
            sibling.before_tool_call(**hook)
            sibling.after_tool_call(**hook)
            self.assertIsNotNone(self.lease(session_a))
            # The same root through another path shares its counts.
            alias.after_tool_call(**hook)
            self.assertIsNotNone(self.lease(session_a))
            alias.after_tool_call(tool_name="terminal", tool_call_id="call-b")
        self.assertIsNone(self.lease(session_a))
        self.assertEqual(self.held(session_b)["requesting_user_id"], BOB)
        with self.turn(BOB, session_b):
            plugins.invoke_hook("post_tool_call", **hook)
        self.assertEqual(self.home.leases(), [])

    def test_only_authenticated_finite_terminal_calls_lease_after_reload(self):
        session = "agent:main:finitechat:group:room-1:seg-1"
        plugins.discover_plugins(force=True)
        hook = {"tool_name": "terminal", "tool_call_id": "call-a"}
        module = self.home.current_module()
        internal = module.MessageEvent(text="wake", internal=True)
        self.assertIsNone(module._authenticated_requester_for_event(internal))
        with self.turn(ALICE, session, marker=None):
            plugins.invoke_hook("pre_tool_call", **hook)
        with self.turn(ALICE, session, marker=BOB):
            plugins.invoke_hook("pre_tool_call", **hook)
        with self.turn(ALICE, session):
            plugins.invoke_hook("pre_tool_call", tool_name="read_file", tool_call_id="call-a")
        with (
            self.turn(ALICE, session),
            patch("gateway.session_context.get_session_env") as session_env,
        ):
            session_env.side_effect = lambda name, default="": {
                "HERMES_SESSION_PLATFORM": "telegram",
                "HERMES_SESSION_KEY": session,
                "HERMES_SESSION_USER_ID": ALICE,
            }.get(name, default)
            plugins.invoke_hook("pre_tool_call", **hook)
        self.assertEqual(self.home.leases(), [])

    def test_registration_alone_keeps_a_killed_gateways_lease(self):
        session = "agent:main:finitechat:group:room-1:seg-1"
        # Registering the plugin (a reload, or a CLI process) cannot tell a
        # dead call from a live one, so it leaves lease files alone, expired
        # or not; readers refuse expired ones. The next gateway's connect
        # removes them; see GatewayStartTests.
        start = int(time.time())
        kill_gateway_mid_call(session, ALICE)
        held = self.held(session)

        plugins.discover_plugins(force=True)
        self.assertEqual(self.lease(session), held)
        module = self.home.current_module()
        with patch("time.time", return_value=start + module.REQUESTER_CONTEXT_TTL_SECS + 1):
            plugins.discover_plugins(force=True)
        self.assertEqual(self.lease(session), held)
        (self.home.v1 / lease_name(session)).unlink()

    def test_registration_never_opens_leftover_lease_files(self):
        session = "agent:main:finitechat:group:room-1:seg-1"
        planted, unreadable = plant_leftovers(self.home.finite_home)
        self.addCleanup((self.home.finite_home / "outside.json").unlink)
        before = tree_snapshot(self.home.finite_home)

        # A new process's first discovery, as a gateway or CLI starts.
        child = (
            CHILD_PRELUDE + "import builtins, io, json, os\n"
            "from gateway.platform_registry import platform_registry\n"
            "from gateway.session_context import set_session_vars\n"
            f"planted, unreadable = {planted!r}, {unreadable!r}\n"
            "opened = []\n"
            "def watch(original):\n"
            "    def guarded(file, *args, **kwargs):\n"
            "        if not isinstance(file, int) and os.fspath(file) in planted:\n"
            "            opened.append(os.fspath(file))\n"
            "            if os.fspath(file) in unreadable:\n"
            "                raise PermissionError(13, 'Permission denied', os.fspath(file))\n"
            "        return original(file, *args, **kwargs)\n"
            "    return guarded\n"
            "builtins.open = io.open = watch(io.open)\n"
            "os.open = watch(os.open)\n"
            "plugins.discover_plugins()\n"
            "m = next(m for n, m in sys.modules.items() if n.endswith('finitechat.adapter'))\n"
            "hooks = plugins.get_plugin_manager()._hooks\n"
            "brokers = {name: sum(isinstance(getattr(c, '__self__', None),\n"
            "    m._RequesterContextBroker) for c in hooks.get(name, []))\n"
            "    for name in ('pre_tool_call', 'post_tool_call')}\n"
            f"set_session_vars(platform='finitechat', session_key={session!r}, user_id={ALICE!r})\n"
            f"m._AUTHENTICATED_FINITE_TURN_USER.set({ALICE!r})\n"
            f"lease = m._requester_context_root() / m._requester_context_filename({session!r})\n"
            "plugins.invoke_hook('pre_tool_call', tool_name='terminal', tool_call_id='c')\n"
            "held = lease.exists()\n"
            "plugins.invoke_hook('post_tool_call', tool_name='terminal', tool_call_id='c')\n"
            "print(json.dumps({'opened': opened, 'brokers': brokers, 'held': held,\n"
            "    'released': not lease.exists(),\n"
            "    'platform': platform_registry.get('finitechat') is not None}))\n"
        )
        result = subprocess.run(
            [sys.executable, "-c", child], capture_output=True, text=True, timeout=WAIT_SECS
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            json.loads(result.stdout.splitlines()[-1]),
            {
                "opened": [],
                "brokers": {"pre_tool_call": 1, "post_tool_call": 1},
                "held": True,
                "released": True,
                "platform": True,
            },
        )
        self.assertEqual(tree_snapshot(self.home.finite_home), before)

        # A forced rediscovery in this process, then a terminal call.
        attempts = []

        def refuse(original):
            def guarded(file, *args, **kwargs):
                if not isinstance(file, int) and os.fspath(file) in planted:
                    attempts.append(os.fspath(file))
                    raise PermissionError(errno.EACCES, "Permission denied", os.fspath(file))
                return original(file, *args, **kwargs)

            return guarded

        previous = self.home.current_module()
        with (
            patch("io.open", refuse(io.open)),
            patch("builtins.open", refuse(open)),
            patch("os.open", refuse(os.open)),
        ):
            plugins.discover_plugins(force=True)
            self.assertIsNot(self.home.current_module(), previous)
            self.assert_plugin_registered(self.home.current_module())
            with self.turn(ALICE, session):
                self.assertIn("LEASE_PRESENT", self.terminal(LEASE_CHECK, "after-reload"))
        self.assertEqual(attempts, [])
        self.assertEqual(tree_snapshot(self.home.finite_home), before)
        for path in map(Path, planted):
            shutil.rmtree(path) if path.is_dir() else path.unlink()

    def test_failed_terminal_call_cleans_up_and_abandoned_call_expires(self):
        session = "agent:main:finitechat:group:room-1:seg-1"
        other = "agent:main:finitechat:group:room-1:seg-2"
        with self.turn(ALICE, session):
            result = handle_function_call(
                "terminal",
                {"command": f"({LEASE_CHECK}); exit 3"},
                task_id="task-fail",
                tool_call_id="call-fail",
            )
        self.assertIn("LEASE_PRESENT", result)
        self.assertEqual(self.home.leases(), [])

        # A call cancelled before its post hook leaves a lease behind; a
        # later broker removes it once expired and keeps a live one.
        start = int(time.time())
        with self.turn(ALICE, session):
            plugins.invoke_hook("pre_tool_call", tool_name="terminal", tool_call_id="abandoned")
        module = self.home.current_module()
        with patch("time.time", return_value=start + module.REQUESTER_CONTEXT_TTL_SECS + 1):
            plugins.discover_plugins(force=True)
            self.assertIsNone(self.lease(session))
            with self.turn(BOB, other):
                plugins.invoke_hook("pre_tool_call", tool_name="terminal", tool_call_id="live")
        self.assertEqual(self.held(other)["requesting_user_id"], BOB)
        with self.turn(BOB, other):
            plugins.invoke_hook("post_tool_call", tool_name="terminal", tool_call_id="live")
        self.assertEqual(self.home.leases(), [])

    def quarantine(self, *sessions: str) -> None:
        """Run startup cleanup while these sessions' leases cannot be removed."""
        as_new_gateway_process()
        with unlink_fails(*(("requester-context-v1", lease_name(key)) for key in sessions)):
            self.home.current_module()._clear_requester_leases_at_gateway_start()

    def terminal(self, command: str, call_id: str) -> str:
        return handle_function_call(
            "terminal", {"command": command}, task_id=f"task-{call_id}", tool_call_id=call_id
        )

    def test_leftovers_that_cannot_be_removed_block_until_a_reader_would_reject_them(self):
        keys = {
            name: f"agent:main:finitechat:group:room-1:{name}"
            for name in ("expired", "live", "junk", "nested", "unreadable")
        }
        as_new_gateway_process()
        now = int(time.time())

        def lease(name: str, expires_at: int) -> bytes:
            payload = {
                "version": 1,
                "session_key": keys[name],
                "platform": "finitechat",
                "requesting_user_id": ALICE,
                "expires_at_unix": expires_at,
            }
            return json.dumps(payload).encode()

        self.home.v1.mkdir(parents=True, exist_ok=True)
        contents = {
            "expired": lease("expired", now),
            "live": lease("live", now + 5),
            "junk": b"{not json",
            "nested": b"[" * 3000,
            "unreadable": lease("unreadable", now),
        }
        for name, content in contents.items():
            (self.home.v1 / lease_name(keys[name])).write_bytes(content)
        unreadable = self.home.v1 / lease_name(keys["unreadable"])
        unreadable.chmod(0)
        (self.home.v1 / "notes.tmp").write_text("not a lease name")
        stuck = [("requester-context-v1", lease_name(key)) for key in keys.values()]
        stuck.append(("requester-context-v1", "notes.tmp"))
        state = sys.modules[STATE_MODULE]
        root = os.path.realpath(self.home.v1)

        with unlink_fails(*stuck):
            self.home.current_module()._clear_requester_leases_at_gateway_start()
            # Only an expired regular lease is known to be harmless; files
            # that are not lease names are never read.
            quarantined = {Path(path).name for path in state.quarantine[root][0]}
            blocked = ["live", "junk", "nested"]
            if not os.access(unreadable, os.R_OK):  # root reads it anyway
                blocked.append("unreadable")
            self.assertEqual(quarantined, {lease_name(keys[name]) for name in blocked})
            with self.turn(ALICE, keys["expired"], marker=None):
                self.assertIn("exit_code", json.loads(self.terminal(LEASE_CHECK, "expired")))
            for name in blocked:
                with self.turn(ALICE, keys[name], marker=None):
                    blocked_message(self.terminal(LEASE_CHECK, name))

            # Once the live lease expires, the next call unblocks its session
            # though the file cannot be removed.
            while int(time.time()) < now + 5:
                time.sleep(0.1)
            with self.turn(ALICE, keys["live"], marker=None):
                self.assertIn("exit_code", json.loads(self.terminal(LEASE_CHECK, "live-later")))
            with self.turn(ALICE, keys["junk"], marker=None):
                blocked_message(self.terminal(LEASE_CHECK, "junk-later"))
        self.assertNotIn(root, state.started_roots)
        unreadable.chmod(0o600)
        with self.turn(ALICE, keys["junk"], marker=None):
            self.assertIn("LEASE_ABSENT", self.terminal(LEASE_CHECK, "junk-cleared"))
        self.assertIn(root, state.started_roots)
        # A retry revisits only quarantined files, so harmless leftovers stay
        # until something removes them.
        leftovers = [self.home.v1 / lease_name(keys[name]) for name in ("expired", "live")]
        leftovers.append(self.home.v1 / "notes.tmp")
        for path in leftovers:
            self.assertTrue(path.exists(), path)
            path.unlink()

    def test_a_failing_quarantine_check_blocks_the_call(self):
        session = "agent:main:finitechat:group:room-1:seg-1"
        other = "agent:main:finitechat:group:room-1:seg-2"
        kill_gateway_mid_call(session, ALICE)
        self.quarantine(session)
        module = self.home.current_module()
        # Hermes treats a raising pre-tool hook as allowing the call.
        with (
            patch.object(module, "_sweep_stale_requester_leases", side_effect=RuntimeError("boom")),
            self.assertLogs(level="ERROR"),
            self.turn(BOB, other),
        ):
            self.assertIn("the check failed: boom", blocked_message(self.terminal("true", "boom")))
        with self.turn(BOB, other):
            self.assertIn("LEASE_PRESENT", self.terminal(LEASE_CHECK, "after"))

    def test_execute_code_reaches_a_stale_session_only_through_the_gate(self):
        session = "agent:main:finitechat:group:room-1:seg-1"
        kill_gateway_mid_call(session, ALICE)
        code = (
            "import os\n"
            "names = ('HERMES_SESSION_PLATFORM', 'HERMES_SESSION_KEY', 'HERMES_SESSION_USER_ID')\n"
            "print('DIRECT=' + '|'.join(os.environ.get(name, '') for name in names))\n"
            "from hermes_tools import terminal\n"
            f"print('RPC=' + str(terminal({LEASE_CHECK!r})))\n"
        )
        with unlink_fails(("requester-context-v1", lease_name(session))):
            self.quarantine(session)
            # An internal turn: the session and user, no authenticated marker.
            with self.turn(ALICE, session, marker=None):
                result = json.loads(
                    handle_function_call(
                        "execute_code", {"code": code}, task_id="task-exec", tool_call_id="exec"
                    )
                )
        # The sandbox's own environment has no session for a reader to use,
        # and its terminal calls pass through the same pre-tool hook.
        self.assertIn("DIRECT=||\n", result["output"])
        self.assertIn(f"RPC={{'error': '{BLOCKED}", result["output"])
        with self.turn(ALICE, session, marker=None):
            self.assertIn("LEASE_ABSENT", self.terminal(LEASE_CHECK, "internal"))


class GatewayStartTests(unittest.IsolatedAsyncioTestCase, LeaseTestCase):
    """A gateway's connect removes leases a killed gateway left behind."""

    SESSION = "agent:main:finitechat:group:room-1:seg-1"
    V2 = ("owner@example.com", "assertion-1")

    async def test_reconnect_after_a_failed_first_connect_refuses_a_killed_gateways_lease(self):
        finite_home = as_new_gateway_process()
        kill_gateway_mid_call(self.SESSION, ALICE, self.V2)
        self.assertEqual(leases_in(finite_home), [lease_name(self.SESSION)] * 2)
        # Cleanup owns only the lease directories; turn owners and anything
        # else in the home are recovery state it must not touch.
        unrelated = [
            finite_home / "hermes-turn-owners" / "owner.json",
            finite_home / "unrelated.json",
            finite_home / "requester-context-v1" / "nested" / "kept.json",
        ]

        def remove_unrelated():
            for path in unrelated:
                path.unlink(missing_ok=True)
                if path.parent != finite_home:
                    shutil.rmtree(path.parent, ignore_errors=True)

        for path in unrelated:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("{}")
        self.addCleanup(remove_unrelated)

        # The first connect times out in inbox recovery, before cleanup.
        first = GatewayHarness(finite_home)

        async def recovery_hangs():
            await asyncio.Event().wait()

        first.adapter._recover_interrupted_turns = recovery_hangs
        with (
            patch.dict(os.environ, {"HERMES_GATEWAY_PLATFORM_CONNECT_TIMEOUT": "0.5"}),
            self.assertRaises(TimeoutError),
        ):
            await first.connect(reconnect=False)
        self.assertEqual(leases_in(finite_home), [*[lease_name(self.SESSION)] * 2, "nested"])

        # Hermes retries with a new adapter and is_reconnect=True.
        second = GatewayHarness(finite_home, first.runner)
        self.assertTrue(await second.connect(reconnect=True))
        self.assertEqual(leases_in(finite_home), ["nested"])
        self.assertTrue(all(path.exists() for path in unrelated))
        remove_unrelated()
        # An internal turn in the dead call's session and user finds no lease.
        self.assertIn("LEASE_ABSENT", await second.turn(ALICE, "seg-1", internal=True))
        self.assertIn("LEASE_PRESENT", await second.turn(ALICE, "seg-1"))
        self.assertEqual(leases_in(finite_home), [])
        await second.adapter.cancel_background_tasks()

    async def test_lease_that_cannot_be_removed_blocks_only_its_sessions_terminal(self):
        root = os.path.realpath(HOME.v1)
        state = sys.modules[STATE_MODULE]
        for kept in ("requester-context-v1", "requester-context-v2"):
            with self.subTest(kept=kept):
                finite_home = as_new_gateway_process()
                kill_gateway_mid_call(self.SESSION, ALICE, self.V2)
                gateway = GatewayHarness(finite_home)
                MODEL.command = BOTH_LEASES_CHECK
                with unlink_fails((kept, lease_name(self.SESSION))):
                    with self.assertLogs(level="ERROR") as logs:
                        self.assertTrue(await gateway.connect(reconnect=False))
                    self.assertTrue(gateway.adapter.is_connected)
                    self.assertIn("could not clear requester leases", "\n".join(logs.output))
                    self.assertNotIn(root, state.started_roots)
                    self.assertEqual(leases_in(finite_home), [lease_name(self.SESSION)])

                    # Internal and human turns in the dead call's session still
                    # answer, and the model gets the reason as the tool result.
                    for internal in (True, False):
                        sent = len(gateway.sent)
                        result = await gateway.turn(ALICE, "seg-1", internal=internal, v2=self.V2)
                        message = blocked_message(result)
                        self.assertIn("cannot remove", message)
                        self.assertIn("Other sessions are not affected.", message)
                        self.assertIn("done", gateway.sent[sent:])
                    # Another session's terminal and lease work as before.
                    result = await gateway.turn(ALICE, "seg-2", v2=self.V2)
                    self.assertIn("LEASE_PRESENT", result)
                    self.assertIn("V2_PRESENT", result)
                    self.assertEqual(leases_in(finite_home), [lease_name(self.SESSION)])

                # Once the file can be removed, the next terminal call clears
                # it, without a reconnect, and the session works again.
                result = await gateway.turn(ALICE, "seg-1", v2=self.V2)
                self.assertIn("LEASE_PRESENT", result)
                self.assertIn("V2_PRESENT", result)
                self.assertIn(root, state.started_roots)
                self.assertNotIn(root, state.quarantine)
                result = await gateway.turn(ALICE, "seg-1", internal=True)
                self.assertIn("LEASE_ABSENT", result)
                self.assertIn("V2_ABSENT", result)
                self.assertEqual(leases_in(finite_home), [])
                await gateway.adapter.cancel_background_tasks()

    async def test_quarantine_survives_reconnects_and_reloads_until_cleanup_completes(self):
        finite_home = as_new_gateway_process()
        root = os.path.realpath(HOME.v1)
        state = sys.modules[STATE_MODULE]
        kill_gateway_mid_call(self.SESSION, ALICE)
        live_session = "agent:main:finitechat:group:room-1:seg-2"
        old = GatewayHarness(finite_home)
        with unlink_fails(("requester-context-v1", lease_name(self.SESSION))):
            self.assertTrue(await old.connect(reconnect=False))

            # A live call in another session holds its lease meanwhile.
            gate = Path(tempfile.mkdtemp(dir=HOME.scratch)) / "gate"
            MODEL.command = (
                f"touch {shlex.quote(str(gate))}.started; "
                f"while [ ! -e {shlex.quote(str(gate))} ]; do sleep 0.05; done; {LEASE_CHECK}"
            )
            held_turn = asyncio.create_task(old.turn(BOB, "seg-2"))
            self.addCleanup(gate.touch)
            while not Path(f"{gate}.started").exists():
                await asyncio.sleep(0.05)
            MODEL.command = LEASE_CHECK
            both = sorted([lease_name(self.SESSION), lease_name(live_session)])
            self.assertEqual(leases_in(finite_home), both)

            # Reconnects, a repeated startup connect and a forced reload all
            # keep Chat up, the quarantine and the live call's lease.
            gateways = [old]
            for reconnect, reload in ((True, False), (False, False), (True, True)):
                if reload:
                    plugins.discover_plugins(force=True)
                gateways.append(GatewayHarness(finite_home, old.runner))
                self.assertTrue(await gateways[-1].connect(reconnect=reconnect))
                self.assertNotIn(root, state.started_roots)
                self.assertEqual(leases_in(finite_home), both)
            result = await gateways[-1].turn(ALICE, "seg-1", internal=True)
            self.assertIn("cannot remove", blocked_message(result))

        # With the fault gone, the next connect completes the cleanup.
        gateways.append(GatewayHarness(finite_home, old.runner))
        self.assertTrue(await gateways[-1].connect(reconnect=True))
        self.assertIn(root, state.started_roots)
        self.assertNotIn(root, state.quarantine)
        self.assertEqual(leases_in(finite_home), [lease_name(live_session)])
        gate.touch()
        self.assertIn("LEASE_PRESENT", await held_turn)
        self.assertIn("LEASE_ABSENT", await gateways[-1].turn(ALICE, "seg-1", internal=True))
        self.assertIn("LEASE_PRESENT", await gateways[-1].turn(ALICE, "seg-1"))
        self.assertEqual(leases_in(finite_home), [])
        for gateway in gateways:
            await gateway.adapter.cancel_background_tasks()

    async def test_forced_reload_beside_malformed_leftovers_keeps_the_session_blocked(self):
        finite_home = as_new_gateway_process()
        root = os.path.realpath(HOME.v1)
        state = sys.modules[STATE_MODULE]
        kill_gateway_mid_call(self.SESSION, ALICE, self.V2)
        # A leftover no reader accepts, which cannot be removed either.
        junk = HOME.v1 / lease_name("agent:main:finitechat:group:room-1:seg-unknown")
        junk.write_bytes(b"[]")
        stuck = [(kept, lease_name(self.SESSION)) for kept in (HOME.v1.name, HOME.v2.name)]
        gateway = GatewayHarness(finite_home)
        MODEL.command = BOTH_LEASES_CHECK
        with unlink_fails(*stuck, (HOME.v1.name, junk.name)):
            with self.assertLogs(level="ERROR"):
                self.assertTrue(await gateway.connect(reconnect=False))
            self.assertIn(
                "cannot remove", blocked_message(await gateway.turn(ALICE, "seg-1", internal=True))
            )
            held = tree_snapshot(finite_home)

            previous = HOME.current_module()
            plugins.discover_plugins(force=True)
            self.assertIsNot(HOME.current_module(), previous)
            self.assert_plugin_registered(HOME.current_module())
            # The running adapter, with the reloaded hooks, still refuses the
            # dead call's session, internal turn or not, and keeps its lease.
            for internal in (True, False):
                message = blocked_message(
                    await gateway.turn(ALICE, "seg-1", internal=internal, v2=self.V2)
                )
                self.assertIn("cannot remove", message)
                self.assertIn("Other sessions are not affected.", message)
            self.assertEqual(tree_snapshot(finite_home), held)
            self.assertNotIn(root, state.started_roots)
            result = await gateway.turn(BOB, "seg-2", v2=self.V2)
            self.assertIn("LEASE_PRESENT", result)
            self.assertIn("V2_PRESENT", result)

        # With the fault gone, the next call clears both, without a restart.
        result = await gateway.turn(ALICE, "seg-1", v2=self.V2)
        self.assertIn("LEASE_PRESENT", result)
        self.assertIn("V2_PRESENT", result)
        self.assertIn(root, state.started_roots)
        self.assertEqual(leases_in(finite_home), [])
        self.assertIn("LEASE_ABSENT", await gateway.turn(ALICE, "seg-1", internal=True))
        await gateway.adapter.cancel_background_tasks()

    async def test_unlistable_lease_directory_blocks_every_session_until_it_lists(self):
        finite_home = as_new_gateway_process()
        kill_gateway_mid_call(self.SESSION, ALICE)
        iterdir = Path.iterdir

        def unlistable(path):
            if path.name == "requester-context-v1":
                raise PermissionError(errno.EACCES, "Permission denied", str(path))
            return iterdir(path)

        gateway = GatewayHarness(finite_home)
        with patch.object(Path, "iterdir", unlistable):
            self.assertTrue(await gateway.connect(reconnect=False))
            for user, segment in ((ALICE, "seg-1"), (BOB, "seg-2")):
                message = blocked_message(await gateway.turn(user, segment))
                self.assertIn("cannot list", message)
                self.assertIn("Every Finite Chat session is affected.", message)
        self.assertEqual(leases_in(finite_home), [lease_name(self.SESSION)])

        self.assertIn("LEASE_PRESENT", await gateway.turn(BOB, "seg-2"))
        self.assertIn("LEASE_ABSENT", await gateway.turn(ALICE, "seg-1", internal=True))
        self.assertEqual(leases_in(finite_home), [])
        await gateway.adapter.cancel_background_tasks()

    async def test_bg_and_btw_children_carry_no_session_to_use_a_lease(self):
        # Pinned Hermes resets the session variables when it handles a
        # message and dispatches /bg and /btw before it binds the session, so
        # their children run with an empty session the readers never match.
        finite_home = as_new_gateway_process()
        kill_gateway_mid_call(self.SESSION, ALICE)
        gateway = GatewayHarness(finite_home)
        with unlink_fails(("requester-context-v1", lease_name(self.SESSION))):
            self.assertTrue(await gateway.connect(reconnect=False))
            MODEL.command = f"{SESSION_ECHO}; {LEASE_CHECK}"
            blocked_message(await gateway.turn(ALICE, "seg-1"))
            for command in ("/bg check the lease", "/btw check the lease"):
                result = json.loads(await gateway.turn(ALICE, "seg-1", text=command))
                self.assertEqual(result["exit_code"], 0, result)
                self.assertIn("SESSION=||\n", result["output"])
                self.assertIn("LEASE_ABSENT", result["output"])
            self.assertEqual(leases_in(finite_home), [lease_name(self.SESSION)])
        MODEL.command = LEASE_CHECK
        self.assertIn("LEASE_ABSENT", await gateway.turn(ALICE, "seg-1", internal=True))
        await gateway.adapter.cancel_background_tasks()

    async def test_startup_cleanup_keeps_calls_this_process_holds(self):
        finite_home = as_new_gateway_process()
        kill_gateway_mid_call("agent:main:finitechat:group:room-1:seg-dead", ALICE)
        module = HOME.current_module()
        runner = object.__new__(GatewayRunner)
        source = SessionSource(
            platform=module._finite_platform(),
            chat_id="room-1",
            chat_type="group",
            user_id=BOB,
            thread_id="seg-1",
        )
        context = build_session_context(source, GatewayConfig())
        context.session_key = self.SESSION
        tokens = runner._set_session_env(context)
        marker = module._AUTHENTICATED_FINITE_TURN_USER.set(BOB)
        hook = {"tool_name": "terminal", "tool_call_id": "live"}
        try:
            plugins.invoke_hook("pre_tool_call", **hook)
            gateway = GatewayHarness(finite_home)
            self.assertTrue(await gateway.connect(reconnect=False))
            self.assertEqual(leases_in(finite_home), [lease_name(self.SESSION)])
            plugins.invoke_hook("post_tool_call", **hook)
        finally:
            module._AUTHENTICATED_FINITE_TURN_USER.reset(marker)
            runner._clear_session_env(tokens)
        self.assertEqual(leases_in(finite_home), [])

    async def test_reconnects_and_reloads_keep_live_leases(self):
        finite_home = as_new_gateway_process()
        old = GatewayHarness(finite_home)
        self.assertTrue(await old.connect(reconnect=False))

        # A live Chat turn on the first adapter holds a v1 and v2 lease.
        gate = Path(tempfile.mkdtemp(dir=HOME.scratch)) / "gate"
        MODEL.command = (
            f"touch {shlex.quote(str(gate))}.started; "
            f"while [ ! -e {shlex.quote(str(gate))} ]; do sleep 0.05; done; {LEASE_CHECK}"
        )
        held_turn = asyncio.create_task(old.turn(ALICE, "seg-1", v2=self.V2))
        self.addCleanup(gate.touch)
        while not Path(f"{gate}.started").exists():
            await asyncio.sleep(0.05)
        MODEL.command = LEASE_CHECK
        live = leases_in(finite_home)
        self.assertEqual(live, [lease_name(self.SESSION)] * 2)

        # Another process with this FINITE_HOME holds a call too, such as
        # a CLI started from a turn. Only the completed-cleanup record
        # keeps a later connect from removing it.
        holder = subprocess.Popen(
            [
                sys.executable,
                "-c",
                CHILD_PRELUDE + "from gateway.session_context import set_session_vars\n"
                "plugins.discover_plugins()\n"
                "m = next(m for n, m in sys.modules.items() if n.endswith('finitechat.adapter'))\n"
                "set_session_vars(platform='finitechat', session_key='other', "
                f"user_id={BOB!r})\n"
                f"m._AUTHENTICATED_FINITE_TURN_USER.set({BOB!r})\n"
                "plugins.invoke_hook('pre_tool_call', tool_name='terminal', tool_call_id='c')\n"
                "print('HELD', flush=True)\n"
                "sys.stdin.readline()\n"
                "plugins.invoke_hook('post_tool_call', tool_name='terminal', tool_call_id='c')\n",
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
        )
        self.addCleanup(holder.kill)
        assert holder.stdout is not None
        self.assertEqual(holder.stdout.readline().strip(), "HELD")
        live = sorted([*live, lease_name("other")])
        self.assertEqual(leases_in(finite_home), live)

        # New adapter objects reconnect, a repeated startup connect and a
        # forced plugin reload all keep both calls' leases.
        reconnected = GatewayHarness(finite_home, old.runner)
        self.assertTrue(await reconnected.connect(reconnect=True))
        self.assertEqual(leases_in(finite_home), live)
        repeated = GatewayHarness(finite_home, old.runner)
        self.assertTrue(await repeated.connect(reconnect=False))
        self.assertEqual(leases_in(finite_home), live)
        plugins.discover_plugins(force=True)
        again = GatewayHarness(finite_home, old.runner)
        self.assertTrue(await again.connect(reconnect=True))
        self.assertEqual(leases_in(finite_home), live)

        # The first adapter's held call still finishes with its lease.
        gate.touch()
        self.assertIn("LEASE_PRESENT", await held_turn)
        holder.communicate("\n", timeout=WAIT_SECS)
        self.assertEqual(holder.returncode, 0)
        self.assertEqual(leases_in(finite_home), [])
        self.assertIn("LEASE_PRESENT", await old.turn(ALICE, "seg-2"))
        self.assertEqual(leases_in(finite_home), [])
        for gateway in (old, reconnected, repeated, again):
            await gateway.adapter.cancel_background_tasks()


FBRAIN = os.environ.get("FBRAIN_TEST_BINARY")
BRAIN_SERVER = os.environ.get("FINITE_BRAIN_SERVER_TEST_BINARY")
BECH32 = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"


def npub_hex(npub: str) -> str:
    data = [BECH32.index(c) for c in npub.rsplit("1", 1)[1][:-6]]
    bits = "".join(f"{value:05b}" for value in data)
    return bytes(int(bits[i : i + 8], 2) for i in range(0, 256, 8)).hex()


@unittest.skipUnless(
    FBRAIN and BRAIN_SERVER, "needs FBRAIN_TEST_BINARY and FINITE_BRAIN_SERVER_TEST_BINARY"
)
class OrganizationCreateAfterReloadTests(unittest.IsolatedAsyncioTestCase, LeaseTestCase):
    """fbrain creates an Organization Brain from a Chat turn after a reload."""

    def fbrain(self, home: Path, *args: str) -> dict:
        result = subprocess.run(
            [str(FBRAIN), *args, "--json"],
            env={
                "HOME": str(home),
                "FINITE_HOME": str(home / "finite"),
                "FBRAIN_CONFIG_DIR": str(home / "fbrain-config"),
                "FINITE_BRAIN_SERVER_URL": self.server_url,
                "FINITE_BRAIN_PUBLIC_BASE_URL": self.server_url,
            },
            capture_output=True,
            text=True,
            timeout=WAIT_SECS,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def identity(self, home: Path, secret: str) -> str:
        home.mkdir(parents=True, exist_ok=True)
        # The Agent's identity lives in the module's FINITE_HOME, which an
        # earlier test may have imported already.
        status = self.fbrain(home, "auth", "status")
        if status["state"] == "authenticated":
            return status["npub"]
        (home / "secret").write_text(secret + "\n")
        return self.fbrain(home, "auth", "import", "--file", str(home / "secret"))["npub"]

    def start_server(self, scratch: Path) -> None:
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            address = f"127.0.0.1:{reservation.getsockname()[1]}"
        self.server_url = f"http://{address}"
        self.database = scratch / "brain.sqlite3"
        server = subprocess.Popen(
            [str(BRAIN_SERVER)],
            env={
                "FINITE_BRAIN_ADDR": address,
                "FINITE_BRAIN_PUBLIC_BASE_URL": self.server_url,
                "FINITE_BRAIN_DB": str(self.database),
            },
            stdout=subprocess.DEVNULL,
        )
        self.addCleanup(server.wait, WAIT_SECS)
        self.addCleanup(server.terminate)
        host, port = address.split(":")

        def accepting() -> bool:
            with socket.socket() as probe:
                return probe.connect_ex((host, int(port))) == 0

        wait_until(accepting, "the Brain server to accept connections")

    async def test_chat_turn_creates_organization_with_human_and_agent_admins(self):
        scratch = Path(tempfile.mkdtemp(dir=self.home.scratch))
        self.start_server(scratch)
        human_home = scratch / "human"
        human_npub = self.identity(human_home, "00" * 31 + "04")
        human = npub_hex(human_npub)
        # The Agent signer lives in the Gateway's FINITE_HOME, beside its leases.
        agent_home = scratch / "agent-cli"
        agent_home.mkdir()
        os.symlink(self.home.finite_home, agent_home / "finite")
        agent_npub = self.identity(agent_home, "00" * 31 + "03")

        gateway = GatewayHarness()
        self.assertIn("LEASE_PRESENT", await gateway.turn(human, "seg-org"))
        plugins.discover_plugins(force=True)
        MODEL.command = self.create_organization_command(agent_home)
        result = json.loads(await gateway.turn(human, "seg-org"))
        await gateway.adapter.cancel_background_tasks()
        self.assert_admins(result, {human_home: human_npub, agent_home: agent_npub})

    async def test_first_gateway_start_refuses_a_killed_gateways_lease(self):
        scratch = Path(tempfile.mkdtemp(dir=self.home.scratch))
        self.start_server(scratch)
        human_home = scratch / "human"
        human_npub = self.identity(human_home, "00" * 31 + "04")
        human = npub_hex(human_npub)
        finite_home = as_new_gateway_process()
        agent_home = scratch / "agent-cli"
        agent_home.mkdir()
        os.symlink(finite_home, agent_home / "finite")
        agent_npub = self.identity(agent_home, "00" * 31 + "03")
        # The previous gateway died inside the human's fbrain call.
        kill_gateway_mid_call("agent:main:finitechat:group:room-1:seg-org", human)
        self.assertEqual(len(leases_in(finite_home)), 1)

        gateway = GatewayHarness(finite_home)
        self.assertTrue(await gateway.connect(reconnect=False))
        MODEL.command = self.create_organization_command(agent_home)
        # An internal turn carries the same session and user, but no request.
        refused = json.loads(await gateway.turn(human, "seg-org", internal=True))
        self.assertNotEqual(refused["exit_code"], 0, refused["output"])
        self.assertIn("requester context is unavailable", refused["output"])
        with contextlib.closing(sqlite3.connect(self.database)) as db:
            self.assertEqual(db.execute("SELECT count(*) FROM brain_admins").fetchone(), (0,))

        result = json.loads(await gateway.turn(human, "seg-org"))
        await gateway.adapter.cancel_background_tasks()
        self.assert_admins(result, {human_home: human_npub, agent_home: agent_npub})
        self.assertEqual(leases_in(finite_home), [])

    async def test_stale_session_cannot_create_until_its_lease_is_cleared(self):
        scratch = Path(tempfile.mkdtemp(dir=self.home.scratch))
        self.start_server(scratch)
        human_home = scratch / "human"
        human_npub = self.identity(human_home, "00" * 31 + "04")
        human = npub_hex(human_npub)
        finite_home = as_new_gateway_process()
        agent_home = scratch / "agent-cli"
        agent_home.mkdir()
        os.symlink(finite_home, agent_home / "finite")
        agent_npub = self.identity(agent_home, "00" * 31 + "03")
        admins = {human_home: human_npub, agent_home: agent_npub}
        session = "agent:main:finitechat:group:room-1:seg-org"
        kill_gateway_mid_call(session, human)
        junk = finite_home / "requester-context-v1" / lease_name("unknown")
        junk.write_bytes(b"[]")

        gateway = GatewayHarness(finite_home)
        with unlink_fails(
            *(("requester-context-v1", name) for name in (lease_name(session), junk.name))
        ):
            self.assertTrue(await gateway.connect(reconnect=False))
            MODEL.command = self.create_organization_command(agent_home, "Stale Org")
            # A forced plugin reload beside a malformed leftover keeps the block.
            for reload in (False, True):
                if reload:
                    plugins.discover_plugins(force=True)
                for internal in (True, False):
                    blocked_message(await gateway.turn(human, "seg-org", internal=internal))
            with contextlib.closing(sqlite3.connect(self.database)) as db:
                self.assertEqual(db.execute("SELECT count(*) FROM brain_admins").fetchone(), (0,))
            MODEL.command = self.create_organization_command(agent_home, "Other Org")
            self.assert_admins(json.loads(await gateway.turn(human, "seg-other")), admins)

        # The next terminal call in the same gateway clears the lease.
        MODEL.command = self.create_organization_command(agent_home, "Cleared Org")
        self.assert_admins(json.loads(await gateway.turn(human, "seg-org")), admins)
        refused = json.loads(await gateway.turn(human, "seg-org", internal=True))
        await gateway.adapter.cancel_background_tasks()
        self.assertNotEqual(refused["exit_code"], 0, refused["output"])
        self.assertIn("requester context is unavailable", refused["output"])
        self.assertEqual(leases_in(finite_home), [])

    def create_organization_command(self, agent_home: Path, name: str = "Reload Org") -> str:
        return " ".join(
            shlex.quote(part)
            for part in (
                "env",
                f"HOME={agent_home}",
                f"FBRAIN_CONFIG_DIR={agent_home / 'fbrain-config'}",
                f"FINITE_BRAIN_SERVER_URL={self.server_url}",
                f"FINITE_BRAIN_PUBLIC_BASE_URL={self.server_url}",
                str(FBRAIN),
                "brain",
                "create",
                "organization",
                name,
                "--json",
            )
        )

    def assert_admins(self, result: dict, identities: dict[Path, str]):
        """Each identity (human and Agent) administers the Brain the turn created."""
        self.assertEqual(result["exit_code"], 0, result["output"])
        created = json.loads(result["output"])
        expected = sorted(identities.values())
        self.assertEqual(sorted(created["admins"]), expected)
        with contextlib.closing(sqlite3.connect(self.database)) as db:
            admins = db.execute(
                "SELECT user_id FROM brain_admins WHERE brain_id = ?", (created["brainId"],)
            ).fetchall()
        self.assertEqual(sorted(row[0] for row in admins), expected)
        for home in identities:
            listed = self.fbrain(home, "brain", "list")["brains"]
            self.assertIn(created["brainId"], [brain["brainId"] for brain in listed])


if __name__ == "__main__":
    unittest.main()
