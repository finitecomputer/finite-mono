"""The opt-in Brain requester-lease diagnostic can never affect chat delivery.

The diagnostic stays off unless the Agent Runtime environment sets
``FINITECHAT_REQUESTER_DIAGNOSTICS=1`` when the plugin registers. When it is
on, a chat turn or tool hook reads the clock and appends one fixed record to
its module's bounded ring: it takes no lock, starts no thread and calls
nothing in ``logging``. One process-wide owner, stable across plugin
rediscovery, runs the only worker and holds the only sink descriptor. The sink
is this user's private regular file in a private directory, refused if it is
a symlink or grants others access, and capped by its actual size.
"""

from __future__ import annotations

import asyncio
import gc
import importlib.util
import json
import logging
import os
import re
import select
import shutil
import stat
import subprocess
import sys
import tempfile
import textwrap
import threading
import time
import types
import unittest
import weakref
from pathlib import Path
from typing import Any, cast
from unittest.mock import patch

REPO_ROOT = Path(__file__).resolve().parents[2]
HARNESS_PATH = REPO_ROOT / "tests" / "hermes" / "test_finite_platform_adapter.py"
PLUGIN_DIR = REPO_ROOT / "integrations" / "hermes" / "finitechat"
GATEWAY_MODULE_NAMES = (
    "gateway",
    "gateway.config",
    "gateway.platforms",
    "gateway.platforms.base",
    "gateway.session_context",
)

# Reuse the canonical adapter test harness by path, as the settlement gate
# suite does. Executing the module only defines its fixtures.
_HARNESS_SPEC = importlib.util.spec_from_file_location(
    "finite_platform_adapter_test_harness", HARNESS_PATH
)
if _HARNESS_SPEC is None or _HARNESS_SPEC.loader is None:
    raise RuntimeError(f"failed to load harness from {HARNESS_PATH}")
harness: Any = importlib.util.module_from_spec(_HARNESS_SPEC)
sys.modules["finite_platform_adapter_test_harness"] = harness
_HARNESS_SPEC.loader.exec_module(harness)

FLAG = "FINITECHAT_REQUESTER_DIAGNOSTICS"
OWNER_KEY = "_finitechat_requester_diagnostics_owner"
MARKER = "[DEBUG-fbrain-requester]"
USER_ID = "ab" * 32
SESSION_KEY = "finitechat:private-room:private-thread"
EMAIL = "sentinel-owner@example.test"
ASSERTION = "sentinel-sites-assertion"
TOOL_CALL_ID = "sentinel-tool-call"
TOOL_ARGUMENT = "fbrain create sentinel-tool-argument"
STAGES = (
    "broker_created|hooks_registered|turn|pre_tool|post_tool|written|write_failed"
    "|removed|remove_failed"
)
GATES = (
    "|authenticated|unauthenticated|accepted|session_api_unavailable|foreign_platform"
    "|missing_session|invalid_user|missing_turn|sender_mismatch|v1|v1_v2|v2"
)
SINK_LINE = re.compile(
    r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z "
    rf"\[DEBUG-fbrain-requester\] stage=(?:{STAGES}) gate=(?:{GATES})"
    r" pid=\d+ marker=[0-9a-f]+$"
)
# How long a stalled sink, handler or thread start holds. Any wait on the
# caller's path would make the measured work take at least this long.
STALL_SECS = 5.0
HERMES_AVAILABLE = importlib.util.find_spec("hermes_cli") is not None


class StallingDiagnosticHandler(logging.Handler):
    """A real handler that stalls only on diagnostic records."""

    def __init__(self, stall_secs: float) -> None:
        super().__init__()
        self.stall_secs = stall_secs
        self.diagnostic_entered = threading.Event()
        self.ordinary_entered = threading.Event()
        self.unstall = threading.Event()

    def handle(self, record: logging.LogRecord) -> bool:
        if MARKER not in record.getMessage():
            self.ordinary_entered.set()
        return bool(super().handle(record))

    def emit(self, record: logging.LogRecord) -> None:
        if MARKER in record.getMessage():
            self.diagnostic_entered.set()
            self.unstall.wait(self.stall_secs)


def stop_owner() -> None:
    """Retire the process-wide owner so the next test starts a fresh one."""
    owner: Any = sys.modules.pop(OWNER_KEY, None)
    if owner is not None:
        owner.stopped = True


class RequesterDiagnosticsTests(unittest.TestCase):
    def setUp(self):
        self.original_gateway_modules = {
            name: sys.modules.get(name) for name in GATEWAY_MODULE_NAMES
        }
        stop_owner()
        self.addCleanup(stop_owner)
        self.module = harness.load_adapter_module()
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.scratch = Path(scratch.name)
        self.finite_home = str(self.scratch / "finite-home")
        self.state_home = str(self.scratch / "state-home")
        os.makedirs(self.state_home)
        self.sink_dir = self.scratch / "requester-diagnostics"
        self.sink = self.sink_dir / "trace.log"
        self.session_context = cast(Any, sys.modules["gateway.session_context"])
        self.session_context.values = {
            "HERMES_SESSION_PLATFORM": "finitechat",
            "HERMES_SESSION_KEY": SESSION_KEY,
            "HERMES_SESSION_USER_ID": USER_ID,
        }

    def tearDown(self):
        for name, module in self.original_gateway_modules.items():
            if module is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = module

    # Fixtures

    def enable(self, sink_dir: Path | None = None) -> Any:
        """Turn the diagnostic on the way the gateway does: at registration."""
        environment = patch.dict(os.environ, {FLAG: "1", "FINITE_HOME": self.finite_home})
        environment.start()
        self.addCleanup(environment.stop)
        self.module.REQUESTER_DIAGNOSTICS_DIR = sink_dir or self.sink_dir
        self.module._REQUESTER_DIAGNOSTICS_REOPEN_SECS = 0.0
        ctx = harness.MockPluginContext()
        self.module.register(ctx)
        return ctx

    def owner(self) -> Any:
        return sys.modules[OWNER_KEY]

    def flush(self) -> None:
        self.assertTrue(self.module._REQUESTER_DIAGNOSTICS.flush(timeout=STALL_SECS))

    def stall_sink_writes(self) -> tuple[threading.Event, threading.Event]:
        """Make the owner's worker block inside its sink write."""
        entered = threading.Event()
        release = threading.Event()
        self.addCleanup(release.set)

        def blocked_write(fd: int, line: bytes, cap: int) -> bool:
            entered.set()
            release.wait(STALL_SECS * 2)
            return False

        self.owner().write_line = blocked_write
        return entered, release

    def private_dir(self, mode: int = 0o700) -> None:
        self.sink_dir.mkdir()
        self.sink_dir.chmod(mode)

    def sink_lines(self) -> list[str]:
        if not self.sink.exists():
            return []
        return self.sink.read_text(encoding="utf-8").splitlines()

    def broker(self, root: Path | None = None):
        return self.module._RequesterContextBroker(
            root or Path(self.finite_home) / "requester-context-v1"
        )

    def adapter(self):
        extra = {"home": self.state_home, "finitechat_bin": "/bin/echo", "room_id": "room-1"}
        home_channel = harness.HomeChannel(
            platform=harness.Platform.FINITECHAT, chat_id="room-1", name="Finite Chat"
        )
        return self.module.FiniteChatAdapter(
            harness.PlatformConfig(extra=extra, home_channel=home_channel)
        )

    def authenticated_event(self):
        return harness.MessageEvent(
            text="create a brain",
            source=types.SimpleNamespace(user_id=USER_ID),
            raw_message={
                "source": {"user_id": USER_ID},
                "requester_email": EMAIL,
                "sites_requester_assertion": ASSERTION,
            },
        )

    def hook(self):
        return {
            "tool_name": "terminal",
            "tool_call_id": TOOL_CALL_ID,
            "args": {"command": TOOL_ARGUMENT},
        }

    def requester_binding(self):
        return (
            self.module._AUTHENTICATED_FINITE_TURN_USER.get(),
            self.module._AUTHENTICATED_FINITE_REQUESTER_CONTEXT.get(),
        )

    def run_turn(self, adapter, event):
        """Run one turn through the stub base.

        Returns the binding the turn saw, then the binding left behind in the
        same task afterwards, where a missed reset would still be visible.
        """
        observed: list[tuple[Any, Any]] = []
        original = harness.BasePlatformAdapter._process_message_background

        async def observe(_self, _event, _session_key):
            observed.append(self.requester_binding())

        async def exercise():
            try:
                await adapter._process_message_background(event, SESSION_KEY)
            finally:
                observed.append(self.requester_binding())

        harness.BasePlatformAdapter._process_message_background = observe
        try:
            asyncio.run(exercise())
        finally:
            harness.BasePlatformAdapter._process_message_background = original
        return observed

    def lease_cycle(self, broker):
        """Lease one authenticated terminal call; return the v1 and v2 payloads."""
        module = self.module
        filename = module._requester_context_filename(SESSION_KEY)
        user_token = module._AUTHENTICATED_FINITE_TURN_USER.set(USER_ID)
        context_token = module._AUTHENTICATED_FINITE_REQUESTER_CONTEXT.set((EMAIL, ASSERTION))
        try:
            broker.before_tool_call(**self.hook())
            v1 = json.loads((broker.root / filename).read_text(encoding="utf-8"))
            v2 = json.loads((broker.root_v2 / filename).read_text(encoding="utf-8"))
            broker.after_tool_call(**self.hook())
        finally:
            module._AUTHENTICATED_FINITE_REQUESTER_CONTEXT.reset(context_token)
            module._AUTHENTICATED_FINITE_TURN_USER.reset(user_token)
        self.assertFalse((broker.root / filename).exists())
        self.assertFalse((broker.root_v2 / filename).exists())
        return v1, v2

    def assert_turn_and_leases_unaffected(self):
        observed = self.run_turn(self.adapter(), self.authenticated_event())
        self.assertEqual(observed, [(USER_ID, (EMAIL, ASSERTION)), (None, None)])
        v1, v2 = self.lease_cycle(self.broker())
        self.assertEqual(v1["requesting_user_id"], USER_ID)
        self.assertEqual(v2["owner_email"], EMAIL)
        self.assertEqual(v2["hosted_requester_assertion"], ASSERTION)

    # Content and gating

    def test_diagnostics_write_fixed_codes_to_their_own_sink_without_requester_data(self):
        captured = StallingDiagnosticHandler(stall_secs=0)
        self.module.logger.addHandler(captured)
        self.addCleanup(self.module.logger.removeHandler, captured)
        with self.assertNoLogs(self.module.logger, level="DEBUG"):
            self.enable()
            broker = self.broker()
            broker.before_tool_call(**self.hook())
            self.assertEqual(list(broker.root.glob("*.json")), [])
            self.lease_cycle(broker)
            self.run_turn(self.adapter(), self.authenticated_event())
            self.flush()

        self.assertFalse(captured.diagnostic_entered.is_set())
        lines = self.sink_lines()
        self.assertTrue(lines)
        for line in lines:
            self.assertRegex(line, SINK_LINE)
        output = "\n".join(lines)
        for expected in (
            "stage=broker_created",
            "stage=hooks_registered",
            "stage=pre_tool gate=missing_turn",
            "stage=pre_tool gate=accepted",
            "stage=written gate=v1_v2",
            "stage=post_tool gate=accepted",
            "stage=removed",
            "stage=turn gate=authenticated",
        ):
            self.assertIn(expected, output)
        for sensitive in (
            SESSION_KEY,
            "private-room",
            USER_ID,
            EMAIL,
            ASSERTION,
            TOOL_CALL_ID,
            TOOL_ARGUMENT,
            "sentinel",
            self.finite_home,
        ):
            self.assertNotIn(sensitive, output)
        self.assertEqual(stat.S_IMODE(self.sink_dir.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(self.sink.stat().st_mode), 0o600)

    def test_diagnostics_distinguish_rejections_without_changing_admission(self):
        self.enable()
        other_user = "cd" * 32
        for platform, session_key, sender, marker, reason in (
            ("telegram", SESSION_KEY, USER_ID, USER_ID, "foreign_platform"),
            ("finitechat", "", USER_ID, USER_ID, "missing_session"),
            ("finitechat", SESSION_KEY, "sentinel-invalid-user", USER_ID, "invalid_user"),
            ("finitechat", SESSION_KEY, USER_ID, None, "missing_turn"),
            ("finitechat", SESSION_KEY, USER_ID, other_user, "sender_mismatch"),
        ):
            with self.subTest(reason=reason):
                self.session_context.values = {
                    "HERMES_SESSION_PLATFORM": platform,
                    "HERMES_SESSION_KEY": session_key,
                    "HERMES_SESSION_USER_ID": sender,
                }
                token = self.module._AUTHENTICATED_FINITE_TURN_USER.set(marker)
                try:
                    result = self.module._active_finite_session(diagnostic_stage="pre_tool")
                finally:
                    self.module._AUTHENTICATED_FINITE_TURN_USER.reset(token)
                self.assertEqual(result, (None, None))
                self.flush()
                output = "\n".join(self.sink_lines())
                self.assertIn(f"stage=pre_tool gate={reason} ", output)
                for sensitive in (SESSION_KEY, "sentinel", USER_ID, other_user):
                    self.assertNotIn(sensitive, output)

    def test_diagnostics_stay_off_unless_the_flag_is_exactly_one(self):
        for value in (None, "", "0", "true", "yes", " 1"):
            stop_owner()
            self.module = harness.load_adapter_module()
            # Loading the module installs a fresh session context stub.
            self.session_context = cast(Any, sys.modules["gateway.session_context"])
            self.session_context.values = {
                "HERMES_SESSION_PLATFORM": "finitechat",
                "HERMES_SESSION_KEY": SESSION_KEY,
                "HERMES_SESSION_USER_ID": USER_ID,
            }
            environment = {"FINITE_HOME": self.finite_home}
            if value is not None:
                environment[FLAG] = value
            with self.subTest(value=value), patch.dict(os.environ, environment):
                if value is None:
                    os.environ.pop(FLAG, None)
                self.module.REQUESTER_DIAGNOSTICS_DIR = self.sink_dir
                ctx = harness.MockPluginContext()
                self.module.register(ctx)
                self.assertEqual(set(ctx.registered_hooks), {"pre_tool_call", "post_tool_call"})
                self.assert_turn_and_leases_unaffected()
                self.module._requester_diagnostic("turn", "authenticated")
                self.assertNotIn(OWNER_KEY, sys.modules)
                self.assertEqual(len(self.module._REQUESTER_DIAGNOSTICS._ring), 0)
                self.assertFalse(self.sink_dir.exists())

    # The producer's whole path

    def test_producer_reads_the_clock_and_appends_nothing_else(self):
        self.enable()
        self.flush()
        calls: list[str] = []

        def profile(frame, event, arg):
            if event == "call":
                calls.append(frame.f_code.co_name)
            elif event == "c_call":
                calls.append("C:" + getattr(arg, "__qualname__", getattr(arg, "__name__", "")))

        sys.setprofile(profile)
        try:
            self.module._requester_diagnostic("turn", "authenticated")
        finally:
            sys.setprofile(None)
        self.assertEqual(
            [call for call in calls if call != "C:setprofile"],
            ["_requester_diagnostic", "emit", "C:time", "C:deque.append"],
        )

    def test_hook_gate_path_takes_no_lock_starts_no_thread_and_never_logs(self):
        self.enable()
        self.flush()
        forbidden_modules = {"logging", "threading", "queue"}
        seen: list[str] = []

        def profile(frame, event, arg):
            if event == "call":
                module = frame.f_globals.get("__name__", "")
                if module.split(".")[0] in forbidden_modules:
                    seen.append(f"python {module}.{frame.f_code.co_name}")
            elif event == "c_call":
                name = getattr(arg, "__qualname__", getattr(arg, "__name__", ""))
                if "acquire" in name or "start_new_thread" in name or "start_joinable" in name:
                    seen.append(f"builtin {name}")

        sys.setprofile(profile)
        try:
            self.module._active_finite_session(diagnostic_stage="pre_tool")
        finally:
            sys.setprofile(None)
        self.assertEqual(seen, [])

    def test_stalled_logging_handler_never_blocks_a_broker_warning_under_the_lock(self):
        # Ported from the second review: a real handler stalls only on
        # diagnostic records while a broker error warning is logged under the
        # broker lock. The warning and the hook must not wait on it.
        handler = StallingDiagnosticHandler(stall_secs=STALL_SECS)
        logger = self.module.logger
        saved = (logger.handlers[:], logger.propagate, logger.level)
        logger.handlers = [handler]
        logger.propagate = False
        logger.setLevel(logging.INFO)
        self.addCleanup(handler.unstall.set)

        def restore() -> None:
            logger.handlers, logger.propagate, _ = saved
            logger.setLevel(saved[2])

        self.addCleanup(restore)
        self.enable()
        self.module._requester_diagnostic("turn", "authenticated")
        # Give a logging-based diagnostic time to reach the handler.
        handler.diagnostic_entered.wait(1.0)

        broker = self.broker(self.scratch / "unwritable" / "requester-context-v1")
        hook_done = threading.Event()

        def run_hook() -> None:
            token = self.module._AUTHENTICATED_FINITE_TURN_USER.set(USER_ID)
            try:
                with patch.object(self.module.Path, "mkdir", side_effect=OSError("disk error")):
                    broker.before_tool_call(tool_name="terminal", tool_call_id="x")
            finally:
                self.module._AUTHENTICATED_FINITE_TURN_USER.reset(token)
                hook_done.set()

        caller = threading.Thread(target=run_hook)
        caller.start()
        try:
            self.assertTrue(hook_done.wait(STALL_SECS / 2), "the broker hook waited on logging")
            self.assertTrue(handler.ordinary_entered.is_set())
        finally:
            handler.unstall.set()
            caller.join(STALL_SECS * 2)

    def test_turn_emission_never_waits_for_worker_start(self):
        # Ported from the second review: the worker's thread start is paused
        # while a turn emits. The turn must not wait for it.
        entered = threading.Event()
        release = threading.Event()
        self.addCleanup(release.set)
        original_thread = threading.Thread

        class SlowBootstrap(original_thread):
            # Thread.start() returns only after the new thread has
            # bootstrapped; this makes that wait as long as the test needs.
            def start(self) -> None:
                entered.set()
                release.wait(STALL_SECS)
                super().start()

        environment = patch.dict(os.environ, {FLAG: "1", "FINITE_HOME": self.finite_home})
        environment.start()
        self.addCleanup(environment.stop)
        self.module.REQUESTER_DIAGNOSTICS_DIR = self.sink_dir
        diagnostics = self.module._RequesterDiagnostics()
        self.module._REQUESTER_DIAGNOSTICS = diagnostics
        turn_done = threading.Event()

        def turn() -> None:
            self.run_turn(self.adapter(), self.authenticated_event())
            turn_done.set()

        with patch.object(self.module.threading, "Thread", SlowBootstrap):
            # Startup is allowed to wait for the thread; the turn is not.
            original_thread(target=diagnostics.start, daemon=True).start()
            self.assertTrue(entered.wait(STALL_SECS), "the worker start was never reached")
            turn_thread = original_thread(target=turn, daemon=True)
            turn_thread.start()
            try:
                self.assertTrue(
                    turn_done.wait(STALL_SECS / 2), "the turn waited for the worker start"
                )
            finally:
                release.set()
                turn_thread.join(STALL_SECS * 2)
        self.assertTrue(diagnostics.flush(timeout=STALL_SECS))
        self.assertIn("stage=turn gate=authenticated", "\n".join(self.sink_lines()))

    @unittest.skipUnless(hasattr(os, "fork"), "requires fork")
    def test_fork_leaves_the_child_disabled_and_its_emission_prompt(self):
        # Ported from the second review: fork while the producer is in use.
        # The child must never wait inside emit and must have it off.
        self.enable()
        diagnostics = self.module._REQUESTER_DIAGNOSTICS
        read_fd, write_fd = os.pipe()
        child = os.fork()
        if child == 0:
            try:
                os.close(read_fd)
                os.write(write_fd, b"entered\n")
                self.module._requester_diagnostic("turn", "authenticated")
                os.write(write_fd, b"enabled\n" if diagnostics._enabled else b"disabled\n")
                os.write(write_fd, b"done\n")
            finally:
                os._exit(0)
        os.close(write_fd)

        received = b""
        deadline = time.monotonic() + STALL_SECS / 2
        while b"done" not in received and time.monotonic() < deadline:
            readable, _, _ = select.select([read_fd], [], [], 0.05)
            if readable:
                chunk = os.read(read_fd, 1024)
                if not chunk:
                    break
                received += chunk
        os.close(read_fd)
        pid, _ = os.waitpid(child, os.WNOHANG)
        if not pid:
            os.kill(child, 9)
            os.waitpid(child, 0)
        self.assertIn(b"entered", received)
        self.assertIn(b"done", received, "the child waited inside emit")
        self.assertIn(b"disabled", received)

    # One owner per process

    def test_owner_does_not_keep_a_dead_module_instance_alive(self):
        self.enable()
        first = weakref.ref(self.module._REQUESTER_DIAGNOSTICS)
        # A second module instance, as plugin rediscovery creates, replaces
        # the first; nothing else refers to the first any more.
        self.module = harness.load_adapter_module()
        self.module.REQUESTER_DIAGNOSTICS_DIR = self.sink_dir
        self.module.register(harness.MockPluginContext())
        # The worker may hold the first instance for one iteration.
        deadline = time.monotonic() + STALL_SECS
        while first() is not None and time.monotonic() < deadline:
            gc.collect()
            time.sleep(0.05)
        self.assertIsNone(first())
        self.module._requester_diagnostic("turn", "authenticated")
        self.flush()
        live = [ref for ref, _ in self.owner().entries if ref() is not None]
        self.assertEqual(len(live), 1)

    @unittest.skipUnless(HERMES_AVAILABLE, "needs the pinned Hermes plugin manager")
    def test_forced_rediscovery_keeps_one_worker_one_sink_and_every_marker(self):
        # Ported from the second review's reload probe: the pinned plugin
        # manager's real forced discovery, three times, in a fresh process.
        home = self.scratch / "hermes-home"
        plugin = home / "plugins" / "finitechat"
        shutil.copytree(PLUGIN_DIR, plugin, ignore=shutil.ignore_patterns("__pycache__"))
        adapter = plugin / "adapter.py"
        source = adapter.read_text(encoding="utf-8")
        default = '"/tmp/finitechat-requester-diagnostics'
        self.assertIn(default, source)
        adapter.write_text(
            source.replace(default, f'"{self.scratch}/finitechat-requester-diagnostics'),
            encoding="utf-8",
        )
        (home / "config.yaml").write_text(
            json.dumps({"plugins": {"enabled": ["finitechat"]}}), encoding="utf-8"
        )
        script = textwrap.dedent(
            f"""
            import json, os, sys, threading, time
            from hermes_cli.plugins import get_plugin_manager

            adapter_file = os.path.realpath({str(adapter)!r})

            def adapter_module():
                return [
                    module for module in list(sys.modules.values())
                    if os.path.realpath(getattr(module, "__file__", None) or "") == adapter_file
                ][0]

            manager = get_plugin_manager()
            manager.discover_and_load(force=False)
            first = adapter_module()
            manager.discover_and_load(force=True)
            manager.discover_and_load(force=True)
            last = adapter_module()
            first._requester_diagnostic("turn", "authenticated")
            last._requester_diagnostic("turn", "unauthenticated")
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline and (first._REQUESTER_DIAGNOSTICS._ring
                                                  or last._REQUESTER_DIAGNOSTICS._ring):
                time.sleep(0.05)
            time.sleep(0.5)
            sinks = []
            for root, _, names in os.walk({str(self.scratch)!r}):
                sinks += [os.path.join(root, n) for n in names
                          if "finitechat-requester-diagnostics" in os.path.join(root, n)]
            inodes = {{os.stat(path).st_ino for path in sinks}}
            descriptors = 0
            for entry in os.listdir("/dev/fd"):
                try:
                    if os.fstat(int(entry)).st_ino in inodes:
                        descriptors += 1
                except OSError:
                    pass
            workers = [t for t in threading.enumerate()
                       if t.name == "finitechat-requester-diagnostics"]
            lines = []
            for path in sinks:
                lines += open(path, encoding="utf-8").read().splitlines()
            print(json.dumps({{
                "same_module": first is last,
                "workers": len(workers),
                "descriptors": descriptors,
                "sinks": len(sinks),
                "lines": lines,
            }}), flush=True)
            """
        )
        environment = dict(
            os.environ,
            HOME=str(self.scratch),
            HERMES_HOME=str(home),
            FINITE_HOME=str(home / "agent"),
            FINITECHAT_HOME=str(home / "agent"),
        )
        environment[FLAG] = "1"
        result = subprocess.run(
            [sys.executable, "-B", "-c", script],
            capture_output=True,
            text=True,
            env=environment,
            timeout=300,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr[-4000:])
        report = json.loads(result.stdout.strip().splitlines()[-1])
        self.assertFalse(report["same_module"])
        self.assertEqual(report["workers"], 1, report)
        self.assertEqual(report["descriptors"], 1, report)
        self.assertEqual(report["sinks"], 1, report)
        markers = {}
        for line in report["lines"]:
            match = re.search(r"stage=turn gate=(\w+) pid=\d+ marker=([0-9a-f]+)$", line)
            if match:
                markers[match.group(1)] = match.group(2)
        self.assertIn("authenticated", markers, report)
        self.assertIn("unauthenticated", markers, report)
        self.assertNotEqual(markers["authenticated"], markers["unauthenticated"])

    # The sink refuses anything that is not its own private regular file

    def test_symlinked_trace_file_is_refused_before_the_first_open(self):
        victim = self.scratch / "durable-victim"
        victim.write_bytes(b"DURABLE\n")
        self.private_dir()
        self.sink.symlink_to(victim)
        self.enable()
        self.assert_turn_and_leases_unaffected()
        self.flush()
        self.assertEqual(victim.read_bytes(), b"DURABLE\n")
        self.assertTrue(self.sink.is_symlink())

    def test_symlink_put_in_place_before_a_reopen_is_refused(self):
        self.enable()
        self.module._requester_diagnostic("turn", "authenticated")
        self.flush()
        self.assertEqual(len(self.sink_lines()), 3)  # broker, hooks, turn
        victim = self.scratch / "durable-victim"
        victim.write_bytes(b"DURABLE\n")
        self.sink.unlink()
        self.sink.symlink_to(victim)
        owner = self.owner()
        real_write = owner.write_line
        failures = iter([OSError("synthetic write failure")])

        def failing_once(fd: int, line: bytes, cap: int) -> bool:
            for failure in failures:
                raise failure
            return real_write(fd, line, cap)

        owner.write_line = failing_once
        # The first record meets the write failure and closes the sink; the
        # rest need a reopen, which meets the symlink.
        for _ in range(3):
            self.module._requester_diagnostic("turn", "authenticated")
            self.flush()
        self.assertEqual(victim.read_bytes(), b"DURABLE\n")

    def test_existing_trace_file_readable_by_others_is_refused(self):
        self.private_dir()
        self.sink.write_bytes(b"EXISTING\n")
        self.sink.chmod(0o644)
        self.enable()
        self.module._requester_diagnostic("turn", "authenticated")
        self.flush()
        self.assertEqual(self.sink.read_bytes(), b"EXISTING\n")
        self.assertEqual(stat.S_IMODE(self.sink.stat().st_mode), 0o644)

    def test_existing_directory_with_a_wider_mode_is_refused_and_left_alone(self):
        self.private_dir(mode=0o755)
        self.enable()
        self.module._requester_diagnostic("turn", "authenticated")
        self.flush()
        self.assertFalse(self.sink.exists())
        self.assertEqual(stat.S_IMODE(self.sink_dir.stat().st_mode), 0o755)

    def test_symlinked_directory_is_refused(self):
        elsewhere = self.scratch / "elsewhere"
        elsewhere.mkdir(mode=0o700)
        self.sink_dir.symlink_to(elsewhere)
        self.enable()
        self.module._requester_diagnostic("turn", "authenticated")
        self.flush()
        self.assertEqual(list(elsewhere.iterdir()), [])

    def test_hard_linked_trace_file_is_refused(self):
        victim = self.scratch / "durable-victim"
        victim.write_bytes(b"DURABLE\n")
        victim.chmod(0o600)
        self.private_dir()
        os.link(victim, self.sink)
        self.enable()
        self.module._requester_diagnostic("turn", "authenticated")
        self.flush()
        self.assertEqual(victim.read_bytes(), b"DURABLE\n")
        self.assertEqual(self.sink.stat().st_nlink, 2)

    def test_directory_widened_between_the_check_and_the_open_is_refused(self):
        self.private_dir()
        sink_dir = str(self.sink_dir)
        real_lstat = os.lstat

        def widen_after_the_check(path: Any, *args: Any, **kwargs: Any) -> os.stat_result:
            listed = real_lstat(path, *args, **kwargs)
            if os.fspath(path) == sink_dir:
                os.chmod(sink_dir, 0o755)
            return listed

        with patch.object(os, "lstat", widen_after_the_check):
            fd = self.module._requester_sink_open(sink_dir, "trace.log")
        if fd is not None:
            os.close(fd)
        self.assertIsNone(fd)
        self.assertFalse(self.sink.exists())
        self.assertEqual(stat.S_IMODE(self.sink_dir.stat().st_mode), 0o755)

    # The cap is a property of the file

    def fill_limit(self) -> int:
        """A small cap that a handful of records reaches."""
        self.module._REQUESTER_DIAGNOSTICS_MAX_BYTES = 1000
        return 1000

    def test_full_existing_file_accepts_nothing(self):
        cap = self.fill_limit()
        self.private_dir()
        self.sink.write_bytes(b"x" * cap)
        self.sink.chmod(0o600)
        self.enable()
        self.module._requester_diagnostic("turn", "authenticated")
        self.flush()
        self.assertEqual(self.sink.stat().st_size, cap)

    def test_last_line_stays_within_the_cap(self):
        cap = self.fill_limit()
        self.enable()
        for _ in range(40):
            self.module._requester_diagnostic("turn", "authenticated")
        self.flush()
        data = self.sink.read_bytes()
        self.assertLessEqual(len(data), cap)
        self.assertTrue(data.endswith(b"\n"))
        longest = max(len(line) + 1 for line in data.splitlines())
        self.assertGreater(len(data), cap - longest)

    def test_restart_gets_no_new_allowance(self):
        cap = self.fill_limit()
        self.enable()
        for _ in range(40):
            self.module._requester_diagnostic("turn", "authenticated")
        self.flush()
        before = self.sink.read_bytes()
        # A new process owner and module instance, as after a gateway restart.
        stop_owner()
        self.module = harness.load_adapter_module()
        self.module._REQUESTER_DIAGNOSTICS_MAX_BYTES = cap
        self.module.REQUESTER_DIAGNOSTICS_DIR = self.sink_dir
        self.module.register(harness.MockPluginContext())
        for _ in range(40):
            self.module._requester_diagnostic("turn", "authenticated")
        self.flush()
        after = self.sink.read_bytes()
        self.assertTrue(after.startswith(before))
        self.assertLessEqual(len(after), cap)

    # Stalled or failing sinks

    def test_stalled_sink_never_delays_a_turn_or_a_lease_write(self):
        self.enable()
        entered, _ = self.stall_sink_writes()
        self.module._requester_diagnostic("turn", "authenticated")
        self.assertTrue(entered.wait(STALL_SECS), "the worker never reached its sink")
        started = time.monotonic()
        self.assert_turn_and_leases_unaffected()
        limit = self.module._REQUESTER_DIAGNOSTICS_QUEUE_LIMIT
        for index in range(limit * 2):
            self.module._requester_diagnostic("turn", f"g{index}")
        elapsed = time.monotonic() - started
        self.assertLess(elapsed, STALL_SECS / 2)
        ring = self.module._REQUESTER_DIAGNOSTICS._ring
        # The ring kept the newest records and overwrote the oldest.
        self.assertEqual(len(ring), limit)
        self.assertEqual(ring[-1][2], f"g{limit * 2 - 1}")

    def test_stalled_sink_never_blocks_the_event_loop_during_a_turn(self):
        self.enable()
        entered, _ = self.stall_sink_writes()
        self.module._requester_diagnostic("turn", "authenticated")
        self.assertTrue(entered.wait(STALL_SECS), "the worker never reached its sink")
        adapter = self.adapter()
        event = self.authenticated_event()
        original = harness.BasePlatformAdapter._process_message_background

        async def slow_turn(_self, _event, _session_key):
            await asyncio.sleep(0.2)

        async def exercise():
            beats = 0

            async def heartbeat():
                nonlocal beats
                while True:
                    beats += 1
                    await asyncio.sleep(0.01)

            beat_task = asyncio.create_task(heartbeat())
            started = time.monotonic()
            await adapter._process_message_background(event, SESSION_KEY)
            elapsed = time.monotonic() - started
            beat_task.cancel()
            return beats, elapsed

        harness.BasePlatformAdapter._process_message_background = slow_turn
        try:
            beats, elapsed = asyncio.run(exercise())
        finally:
            harness.BasePlatformAdapter._process_message_background = original
        self.assertLess(elapsed, STALL_SECS / 2)
        self.assertGreater(beats, 5)

    def test_unwritable_sink_never_fails_a_turn_or_a_lease_write(self):
        self.enable(sink_dir=self.scratch / "missing-parent" / "requester-diagnostics")
        self.assert_turn_and_leases_unaffected()
        # The worker drops what it cannot write and keeps draining.
        self.flush()
        self.assert_turn_and_leases_unaffected()

    def test_failure_to_start_the_worker_never_fails_registration_a_turn_or_a_lease(self):
        class UnstartableThread(threading.Thread):
            def start(self):
                raise RuntimeError("can't start new thread")

        with patch.object(self.module.threading, "Thread", UnstartableThread):
            ctx = self.enable()
        self.assertEqual(set(ctx.registered_hooks), {"pre_tool_call", "post_tool_call"})
        self.assertEqual(len(ctx.registered), 1)
        self.assert_turn_and_leases_unaffected()

    def test_stalled_sink_and_handler_never_block_logging_shutdown_or_exit(self):
        # A worker stuck in its sink must leave logging.shutdown() and
        # interpreter exit unaffected. A logging-based diagnostic would hold
        # the stalled handler's lock, which logging.shutdown() also takes.
        script = textwrap.dedent(
            f"""
            import importlib.util, logging, os, sys, threading, time
            from pathlib import Path
            spec = importlib.util.spec_from_file_location("harness", {str(HARNESS_PATH)!r})
            harness = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(harness)
            module = harness.load_adapter_module()

            class Stalling(logging.Handler):
                def emit(self, record):
                    if {MARKER!r} in record.getMessage():
                        threading.Event().wait({STALL_SECS * 6})

            module.logger.handlers = [Stalling()]
            module.logger.propagate = False
            module.logger.setLevel(logging.INFO)
            os.environ[{FLAG!r}] = "1"
            os.environ["FINITE_HOME"] = {self.finite_home!r}
            module.REQUESTER_DIAGNOSTICS_DIR = Path({str(self.sink_dir)!r})
            module.register(harness.MockPluginContext())
            entered = threading.Event()

            def stuck(fd, line, cap):
                entered.set()
                threading.Event().wait({STALL_SECS * 6})

            owner = sys.modules.get({OWNER_KEY!r})
            if owner is not None:
                owner.write_line = stuck
            module._requester_diagnostic("turn", "authenticated")
            entered.wait(5)
            started = time.monotonic()
            logging.shutdown()
            print("shutdown_secs=%.3f" % (time.monotonic() - started), flush=True)
            """
        )
        result = subprocess.run(
            [sys.executable, "-B", "-c", script],
            capture_output=True,
            text=True,
            timeout=STALL_SECS * 24,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        match = re.search(r"shutdown_secs=([0-9.]+)", result.stdout)
        self.assertIsNotNone(match, result.stdout + result.stderr)
        assert match is not None
        self.assertLess(float(match.group(1)), STALL_SECS / 2)

    def test_diagnostic_clock_failure_does_not_interrupt_the_turn(self):
        self.enable()
        with patch.object(self.module.time, "time", side_effect=OSError("clock unavailable")):
            observed = self.run_turn(self.adapter(), self.authenticated_event())
        self.assertEqual(observed, [(USER_ID, (EMAIL, ASSERTION)), (None, None)])

    def test_emission_failure_leaves_turn_and_hooks_unchanged_when_on_or_off(self):
        for enabled in (False, True):
            with self.subTest(enabled=enabled):
                if enabled:
                    self.enable()
                with patch.object(
                    self.module._REQUESTER_DIAGNOSTICS,
                    "emit",
                    side_effect=RuntimeError("producer unavailable"),
                ):
                    self.assert_turn_and_leases_unaffected()


if __name__ == "__main__":
    unittest.main()
