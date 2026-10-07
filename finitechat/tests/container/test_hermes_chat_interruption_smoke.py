"""Contract checks for the deterministic real-Hermes interruption smoke."""

from __future__ import annotations

import copy
import importlib.util
import io
import json
import os
import re
import subprocess
import tarfile
import tempfile
import time
import unittest
import urllib.request
from pathlib import Path
from typing import ClassVar
from unittest import mock

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPO_ROOT / "scripts" / "hermes-chat-interruption-docker-smoke.py"
WORKFLOW_PATH = REPO_ROOT.parent / ".github" / "workflows" / "hermes-runtime-smoke.yml"
SIDECAR_INBOX_PATH = REPO_ROOT / "crates" / "finitechat-cli" / "src" / "hermes.rs"

spec = importlib.util.spec_from_file_location("hermes_chat_interruption_smoke", SCRIPT_PATH)
assert spec is not None and spec.loader is not None
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


class HermesChatInterruptionSmokeTest(unittest.TestCase):
    def test_fake_provider_stall_barrier_is_explicit(self) -> None:
        state = smoke.FakeModelState()
        request, stall = state.record(
            {
                "stream": True,
                "messages": [
                    {"role": "user", "content": "FINITE_INTERRUPT_STALL:sigkill keep open"}
                ],
            }
        )

        self.assertEqual(stall, "sigkill")
        self.assertNotIn("sigkill", state.seen)
        state.mark_seen("sigkill", request)
        state.wait_seen("sigkill", timeout=0.1)
        self.assertEqual(state.require_stream_in_flight("sigkill")["request_index"], 0)

    def test_barrier_rejects_missing_nonstream_finished_released_and_timed_out(self) -> None:
        for mutation in ("missing", "nonstream", "finished", "released", "timed_out", "duplicate"):
            with self.subTest(mutation=mutation):
                state = smoke.FakeModelState()
                payload = {
                    "stream": True,
                    "messages": [{"role": "user", "content": "FINITE_INTERRUPT_STALL:sigkill"}],
                }
                request, _ = state.record(payload)
                if mutation != "missing":
                    state.mark_seen("sigkill", request)
                if mutation == "nonstream":
                    request["stream"] = False
                elif mutation == "finished":
                    state.finish(request, "streamed")
                elif mutation == "released":
                    state.release("sigkill")
                elif mutation == "timed_out":
                    state.wait_released("sigkill", request, timeout=0)
                elif mutation == "duplicate":
                    duplicate, _ = state.record(payload)
                    state.mark_seen("sigkill", duplicate)
                with self.assertRaises(smoke.SmokeFailure):
                    state.require_stream_in_flight("sigkill")

    def test_http_barrier_distinguishes_history_requests_from_the_open_stream(self) -> None:
        state = smoke.FakeModelState()
        server = smoke.start_fake_model(state, 0)
        url = f"http://127.0.0.1:{server.server_address[1]}/v1/chat/completions"

        def request(stream: bool) -> urllib.request.Request:
            return urllib.request.Request(
                url,
                data=json.dumps(
                    {
                        "stream": stream,
                        "messages": [{"role": "user", "content": "FINITE_INTERRUPT_STALL:sigkill"}],
                    }
                ).encode(),
                headers={"Content-Type": "application/json"},
            )

        try:
            with urllib.request.urlopen(request(False), timeout=2) as response:
                response.read()
            self.assertNotIn("sigkill", state.seen)
            with self.assertRaises(smoke.SmokeFailure):
                state.require_stream_in_flight("sigkill")
            with urllib.request.urlopen(request(True), timeout=2) as response:
                self.assertEqual(response.headers["Content-Type"], "text/event-stream")
                state.wait_seen("sigkill", timeout=2)
                barrier = state.require_stream_in_flight("sigkill")
                self.assertEqual(barrier["request_index"], 1)
                self.assertLessEqual(barrier["entered_at_ms"], barrier["checked_at_ms"])
                state.release("sigkill")
                self.assertIn(b"data: [DONE]", response.read())
            with self.assertRaises(smoke.SmokeFailure):
                state.require_stream_in_flight("sigkill")
        finally:
            state.release("sigkill")
            server.shutdown()
            server.server_close()

    def test_fake_provider_returns_the_requested_fresh_reply(self) -> None:
        self.assertEqual(
            smoke.expected_reply(
                {"messages": [{"role": "user", "content": "Reply with exactly: fresh chat two ok"}]}
            ),
            "fresh chat two ok",
        )

    def test_canonical_hermes_version_is_parsed_from_runtime_output(self) -> None:
        self.assertEqual(smoke.parse_hermes_version("Hermes Agent v0.18.2 (2026.7.7.2)"), "0.18.2")

    def test_matrix_keeps_the_three_bounded_cases(self) -> None:
        source = SCRIPT_PATH.read_text(encoding="utf-8")
        self.assertEqual(
            smoke.GRACEFUL_STOP_CASES,
            ["graceful-stop", "graceful-stop-repeat-2", "graceful-stop-repeat-3"],
        )
        self.assertIn(
            "for graceful_case in GRACEFUL_STOP_CASES:\n"
            "            interrupt(graceful_case, kill=False, restore=False)",
            source,
        )
        self.assertIn('interrupt("sigkill", kill=True, restore=False)', source)
        self.assertIn('interrupt("empty-target-restore", kill=False, restore=True)', source)
        self.assertIn(
            '"interruption_boundary": "SSE headers flushed before first data frame"', source
        )
        self.assertIn("wait_durable_inbox_event(name, queued_message_id)", source)
        self.assertIn('"first_next_ordinary_turn": True', source)
        self.assertIn('"model_handoffs": queued_handoffs', source)
        self.assertIn("expected SIGKILL exit 137", source)
        self.assertIn("escalated to SIGKILL instead of stopping gracefully", source)
        self.assertIn('"production_kata_task_and_stable_manifest_gate": False', source)

    def test_provider_summary_keeps_markers_and_outcomes_without_prompt_bodies(self) -> None:
        state = smoke.FakeModelState()
        stalled, _ = state.record(
            {
                "stream": True,
                "messages": [
                    {"role": "system", "content": "private system prompt"},
                    {"role": "user", "content": "FINITE_INTERRUPT_STALL:graceful-stop keep open"},
                ],
            }
        )
        queued, stall = state.record(
            {
                "stream": True,
                "messages": [
                    {"role": "user", "content": "Reply with exactly: graceful-stop queued ok"}
                ],
            }
        )
        self.assertIsNone(stall)
        state.finish(queued, "client_disconnected")

        summary = state.summary()

        self.assertEqual(summary["count"], 2)
        first, second = summary["requests"]
        self.assertEqual((first["stall"], first["outcome"]), ("graceful-stop", "pending"))
        self.assertEqual(second["latest_user_reply_marker"], "graceful-stop queued ok")
        self.assertEqual(second["outcome"], "client_disconnected")
        self.assertIsInstance(second["finished_at_ms"], int)
        self.assertNotIn("private system prompt", repr(summary))
        self.assertNotIn("keep open", repr(summary))
        self.assertEqual(stalled["outcome"], "pending")

    def test_provider_summary_is_bounded_to_the_latest_requests(self) -> None:
        state = smoke.FakeModelState()
        for index in range(5):
            state.observe(
                {"messages": [{"role": "user", "content": f"Reply with exactly: n{index}"}]}
            )

        summary = state.summary(limit=2)

        self.assertEqual(summary["count"], 5)
        self.assertEqual([request["index"] for request in summary["requests"]], [3, 4])

    def test_inbox_summary_reports_lease_state_without_event_payload(self) -> None:
        inbox = {
            "events": [
                {
                    "key": "room-a:7:msg-7",
                    "room_id": "room-a",
                    "seq": 7,
                    "message_id": "msg-7",
                    "created_at_ms": 900,
                    "event": {"text": "secret message body"},
                    "lease": {"state": "leased", "lease_id": "lease-1", "leased_at_ms": 1_000},
                },
                {
                    "key": "room-a:8:msg-8",
                    "seq": 8,
                    "message_id": "msg-8",
                    "event": {"text": "another body"},
                },
            ],
            "cursors": {"room-a": 8},
            "acked": [{"key": "room-a:6:msg-6", "acked_at_ms": 800}],
        }

        summary = smoke.summarize_hermes_inbox(inbox, now_ms=4_000)

        leased, legacy = summary["events"]
        self.assertEqual(leased["lease_state"], "leased")
        self.assertEqual(leased["lease_id"], "lease-1")
        self.assertEqual(leased["lease_age_ms"], 3_000)
        self.assertEqual(legacy["lease_state"], "pending")
        self.assertIsNone(legacy["lease_age_ms"])
        self.assertEqual(summary["cursors"], {"room-a": 8})
        self.assertEqual(summary["acked_count"], 1)
        self.assertNotIn("body", repr(summary))

    def test_inbox_summary_is_bounded(self) -> None:
        inbox = {"events": [{"key": f"k{index}"} for index in range(10)]}

        summary = smoke.summarize_hermes_inbox(inbox, now_ms=0, limit=3)

        self.assertEqual(summary["event_count"], 10)
        self.assertEqual([event["key"] for event in summary["events"]], ["k7", "k8", "k9"])

    def test_bounded_text_keeps_the_tail(self) -> None:
        self.assertEqual(smoke.bounded_text("short", 10), "short")
        bounded = smoke.bounded_text("a" * 20 + "TAIL", 4)
        self.assertTrue(bounded.endswith("TAIL"))
        self.assertLess(len(bounded), 24)

    def test_stopped_inbox_read_never_creates_a_missing_volume(self) -> None:
        missing = subprocess.CompletedProcess([], 1, "", "no such volume")
        with (
            mock.patch.object(smoke.smoke, "run", return_value=missing) as run,
            self.assertRaises(smoke.SmokeFailure),
        ):
            smoke.read_hermes_inbox(
                image="image", container="agent", home_volume="home", live=False
            )
        self.assertEqual(run.call_count, 1)
        self.assertEqual(run.call_args.args[0][:3], ["docker", "volume", "inspect"])

    def test_stopped_inbox_read_mounts_the_home_volume_read_only(self) -> None:
        ok = subprocess.CompletedProcess([], 0, '{"events":[]}', "")
        with mock.patch.object(smoke.smoke, "run", return_value=ok) as run:
            raw = smoke.read_hermes_inbox(
                image="image", container="agent", home_volume="home", live=False
            )
        self.assertEqual(raw, '{"events":[]}')
        command = run.call_args.args[0]
        self.assertIn("type=volume,src=home,dst=/home/node,readonly", command)
        self.assertEqual(command[-1], "/home/node/.finitechat/agent/hermes-inbox.json")

    def test_diagnostic_capture_records_read_failures_without_raising(self) -> None:
        state = smoke.FakeModelState()
        state.observe({"messages": [{"role": "user", "content": "Reply with exactly: x"}]})
        timeout = subprocess.TimeoutExpired(["docker"], 30)
        with (
            mock.patch.object(smoke.smoke, "run", side_effect=timeout),
            mock.patch.object(smoke.smoke, "agent_log_tail", side_effect=RuntimeError("gone")),
        ):
            snapshot = smoke.capture_diagnostics(
                "on_failure",
                image="image",
                container="agent",
                home_volume="home",
                live=True,
                model_state=state,
            )

        self.assertEqual(snapshot["point"], "on_failure")
        self.assertIn("TimeoutExpired", snapshot["hermes_inbox_error"])
        self.assertEqual(snapshot["agent_log_error"], "RuntimeError: gone")
        self.assertEqual(snapshot["provider"]["count"], 1)

    def test_diagnostic_capture_bounds_the_agent_log_tail(self) -> None:
        inbox = subprocess.CompletedProcess([], 0, '{"events":[]}', "")
        with (
            mock.patch.object(smoke.smoke, "run", return_value=inbox),
            mock.patch.object(smoke.smoke, "agent_log_tail", return_value="x" * 50_000) as tail,
        ):
            snapshot = smoke.capture_diagnostics(
                "before_stop",
                image="image",
                container="agent",
                home_volume="home",
                live=True,
                model_state=smoke.FakeModelState(),
            )

        self.assertEqual(tail.call_args.kwargs["lines"], smoke.DIAGNOSTIC_LOG_LINES)
        self.assertLessEqual(len(snapshot["agent_log_tail"]), smoke.DIAGNOSTIC_TEXT_LIMIT + 20)
        self.assertEqual(snapshot["hermes_inbox"]["event_count"], 0)

    def test_failure_report_records_stage_and_diagnostics_before_reraising(self) -> None:
        source = SCRIPT_PATH.read_text(encoding="utf-8")
        failure = source[source.index('report["status"] = "failed"') :]
        failure = failure[: failure.index("finally:")]
        self.assertIn('report["failure_stage"] = report.get("stage")', failure)
        self.assertIn('diagnose("on_failure", live=False)', failure)
        self.assertTrue(failure.rstrip().endswith("raise"))
        for point in ("before_stop", "before_container_removal", "after_restart"):
            self.assertIn(f'diagnose("{point}"', source)

    # Shapes below follow runtime-diagnostics-37421176063: the not-awaited
    # shutdown left both the active (seq 4) and queued (seq 6) entries leased.
    @staticmethod
    def inbox_fixture(now_ms: int = 10_000_000) -> dict:
        def event(seq: int, message_id: str, lease: dict | None) -> dict:
            entry = {
                "key": f"room-a\x1f{seq}\x1f{message_id}",
                "room_id": "room-a",
                "seq": seq,
                "message_id": message_id,
                "created_at_ms": now_ms - 5_000,
                "event": {"text": f"body {seq}"},
            }
            if lease is not None:
                entry["lease"] = lease
            return entry

        return {
            "events": [
                event(3, "older", {"state": "pending"}),
                event(
                    4,
                    "active",
                    {"state": "leased", "lease_id": "l-4", "leased_at_ms": now_ms - 3_747},
                ),
                event(
                    6, "queued", {"state": "leased", "lease_id": "l-6", "leased_at_ms": now_ms - 91}
                ),
                event(7, "other", {"state": "leased", "lease_id": "l-7", "leased_at_ms": now_ms}),
            ],
            "cursors": {"room-a": 7},
            "acked": [{"key": "room-a\x1f2\x1fdone", "acked_at_ms": now_ms - 9_000}],
        }

    IDS: ClassVar[dict[str, str]] = {"active": "active", "queued": "queued"}

    def released(self, active: str = "pending") -> dict:
        inbox = self.inbox_fixture()
        inbox["events"] = [e for e in inbox["events"] if e["message_id"] != "other"]
        for entry in inbox["events"]:
            if entry["message_id"] in ("active", "queued"):
                entry["lease"] = {"state": "pending"}
        if active == "acked":
            inbox["events"] = [e for e in inbox["events"] if e["message_id"] != "active"]
            inbox["acked"].append({"key": "room-a\x1f4\x1factive", "acked_at_ms": 1})
        return inbox

    def test_graceful_check_fails_on_the_observed_not_awaited_inbox(self) -> None:
        with self.assertRaisesRegex(smoke.SmokeFailure, "leased before restart: seq=4"):
            smoke.require_graceful_inbox_released(self.inbox_fixture(), message_ids=self.IDS)

    def test_graceful_check_fails_on_any_leased_entry(self) -> None:
        inbox = self.released()
        inbox["events"][0]["lease"] = {"state": "leased", "lease_id": "x", "leased_at_ms": 1}
        with self.assertRaisesRegex(smoke.SmokeFailure, "seq=3"):
            smoke.require_graceful_inbox_released(inbox, message_ids=self.IDS)

    def test_graceful_check_requires_both_unfinished_turns_to_be_released(self) -> None:
        self.assertEqual(
            smoke.require_graceful_inbox_released(self.released(), message_ids=self.IDS),
            {"active": "pending", "queued": "pending"},
        )
        with self.assertRaisesRegex(smoke.SmokeFailure, "stalled active turn"):
            smoke.require_graceful_inbox_released(self.released("acked"), message_ids=self.IDS)

    def test_graceful_check_rejects_a_settled_or_missing_queued_follow_up(self) -> None:
        settled = self.released()
        settled["events"] = [e for e in settled["events"] if e["message_id"] != "queued"]
        missing = copy.deepcopy(settled)
        settled["acked"].append({"key": "room-a\x1f6\x1fqueued", "acked_at_ms": 1})
        with self.assertRaisesRegex(smoke.SmokeFailure, "settled before it ran"):
            smoke.require_graceful_inbox_released(settled, message_ids=self.IDS)
        with self.assertRaisesRegex(smoke.SmokeFailure, "vanished"):
            smoke.require_graceful_inbox_released(missing, message_ids=self.IDS)

    def test_expiry_fixture_backdates_only_the_two_known_leases(self) -> None:
        original = self.inbox_fixture()
        pristine = copy.deepcopy(original)

        simulated, records = smoke.simulate_lease_expiry(
            original, message_ids=self.IDS, now_ms=10_000_000
        )

        self.assertEqual(original, pristine, "the input inbox must not be mutated")
        shift = smoke.PRODUCTION_LEASE_TTL_MS + smoke.SIMULATED_EXPIRY_MARGIN_MS
        expected = copy.deepcopy(pristine)
        expected["events"][1]["lease"]["leased_at_ms"] -= shift
        expected["events"][2]["lease"]["leased_at_ms"] -= shift
        self.assertEqual(json.dumps(simulated), json.dumps(expected))
        self.assertEqual(
            [(r["role"], r["seq"], r["lease_id"], r["original_leased_at_ms"]) for r in records],
            [("active", 4, "l-4", 10_000_000 - 3_747), ("queued", 6, "l-6", 10_000_000 - 91)],
        )
        for record in records:
            self.assertEqual(
                record["original_leased_at_ms"] - record["simulated_leased_at_ms"], shift
            )
        # The sidecar's TTL rule now sees both as expired and the third as live.
        now = 10_000_000
        ages = [now - e["lease"]["leased_at_ms"] for e in simulated["events"][1:]]
        self.assertGreaterEqual(ages[0], smoke.PRODUCTION_LEASE_TTL_MS)
        self.assertGreaterEqual(ages[1], smoke.PRODUCTION_LEASE_TTL_MS)
        self.assertLess(ages[2], smoke.PRODUCTION_LEASE_TTL_MS)

    def test_expiry_fixture_refuses_anything_but_two_fresh_leases(self) -> None:
        now = 10_000_000
        cases = {
            "not leased": lambda i: i["events"][1].update(lease={"state": "pending"}),
            "not a fresh": lambda i: i["events"][1]["lease"].update(
                leased_at_ms=now - smoke.PRODUCTION_LEASE_TTL_MS
            ),
            "found 2 times": lambda i: i["events"].append(copy.deepcopy(i["events"][2])),
            "found 0 times": lambda i: i["events"].pop(2),
        }
        for message, mutate in cases.items():
            inbox = self.inbox_fixture(now)
            mutate(inbox)
            with self.subTest(message), self.assertRaisesRegex(smoke.SmokeFailure, message):
                smoke.simulate_lease_expiry(inbox, message_ids=self.IDS, now_ms=now)
        with self.assertRaisesRegex(smoke.SmokeFailure, "exactly two distinct"):
            smoke.simulate_lease_expiry(
                self.inbox_fixture(now),
                message_ids={"active": "active", "queued": "active"},
                now_ms=now,
            )

    def test_backdate_verification_rejects_any_other_change(self) -> None:
        original = self.inbox_fixture()
        simulated, records = smoke.simulate_lease_expiry(
            original, message_ids=self.IDS, now_ms=10_000_000
        )
        for mutate in (
            lambda i: i["events"][3]["lease"].update(leased_at_ms=0),
            lambda i: i["events"].reverse(),
            lambda i: i["cursors"].update({"room-a": 8}),
            lambda i: i["events"][1]["lease"].update(lease_id="other"),
        ):
            changed = copy.deepcopy(simulated)
            mutate(changed)
            with self.assertRaises(smoke.SmokeFailure):
                smoke.require_only_leases_backdated(original, changed, records)

    def test_turn_lease_ttl_matches_pinned_hermes(self) -> None:
        spec = importlib.util.find_spec("run_agent")
        assert spec is not None and spec.origin is not None
        source = Path(spec.origin).read_text(encoding="utf-8")
        ttls = re.findall(r"^\s+_lease_ttl = (\d+(?:\.\d+)?)$", source, re.MULTILINE)
        self.assertEqual([float(ttl) for ttl in ttls], [smoke.HERMES_TURN_LEASE_TTL_SECS])
        refresh = re.findall(r'"_session_turn_lease_refresh_interval", (\d+(?:\.\d+)?)\)', source)
        self.assertEqual(len(refresh), 1)
        self.assertLess(float(refresh[0]), smoke.HERMES_TURN_LEASE_TTL_SECS)

    def test_pinned_turn_lease_of_a_killed_recycled_pid_holds_until_the_smoke_deadline(
        self,
    ) -> None:
        with (
            tempfile.TemporaryDirectory() as home,
            mock.patch.dict(os.environ, {"HERMES_HOME": home}),
        ):
            import hermes_state

            db = hermes_state.SessionDB(Path(home) / "state.db")
            try:
                db.create_session("room", "finitechat")
                exited_at = time.time()
                # This process's own PID stands in for the killed gateway's
                # PID recycled in the new container, so Hermes counts it alive.
                killed = f"pid={os.getpid()}:turn=killed:platform=finitechat"
                self.assertTrue(
                    db.try_acquire_session_turn_lease(
                        "room", killed, ttl_seconds=smoke.HERMES_TURN_LEASE_TTL_SECS
                    )
                )
                restarted = f"pid={os.getpid()}:turn=restarted:platform=finitechat"
                deadline = (
                    exited_at
                    + smoke.HERMES_TURN_LEASE_TTL_SECS
                    + smoke.HERMES_TURN_LEASE_MARGIN_SECS
                )
                for at, acquired in (
                    (exited_at + 90, False),  # the old queued-reply timeout
                    (exited_at + smoke.HERMES_TURN_LEASE_TTL_SECS - 1, False),
                    (deadline, True),
                ):
                    with (
                        self.subTest(after_exit=at - exited_at),
                        mock.patch.object(hermes_state.time, "time", return_value=at),
                    ):
                        self.assertIs(
                            db.try_acquire_session_turn_lease("room", restarted), acquired
                        )
            finally:
                db.close()

    def test_stopped_turn_leases_are_read_from_a_copy_including_the_wal(self) -> None:
        with (
            tempfile.TemporaryDirectory() as home,
            mock.patch.dict(os.environ, {"HERMES_HOME": home}),
        ):
            import hermes_state

            db = hermes_state.SessionDB(Path(home) / "state.db")
            try:
                db.create_session("room", "finitechat")
                holder = "pid=27:turn=killed:platform=finitechat"
                self.assertTrue(db.try_acquire_session_turn_lease("room", holder))
                self.assertTrue((Path(home) / "state.db-wal").stat().st_size > 0)
                archive = io.BytesIO()
                with tarfile.open(fileobj=archive, mode="w") as tar:
                    for name in ("state.db", "state.db-wal"):
                        tar.add(Path(home) / name, arcname=name)
                ok = subprocess.CompletedProcess([], 0, "", "")
                copied = subprocess.CompletedProcess([], 0, archive.getvalue(), b"")
                with (
                    mock.patch.object(smoke.smoke, "run", return_value=ok),
                    mock.patch.object(smoke.subprocess, "run", return_value=copied) as run,
                ):
                    rows = smoke.read_stopped_turn_leases(image="image", home_volume="home")
            finally:
                db.close()
        self.assertEqual([row["holder"] for row in rows], [holder])
        command = run.call_args.args[0]
        self.assertIn("type=volume,src=home,dst=/home/node,readonly", command)
        self.assertIn("--network", command)
        self.assertIn("state.db-wal", command[-1])
        self.assertNotIn("state.db-shm", command[-1])

    def test_turn_lease_wait_runs_on_monotonic_time_from_the_proven_exit(self) -> None:
        class Clock:
            def __init__(self, now: float) -> None:
                self.now = now
                self.slept: list[float] = []

            def monotonic(self) -> float:
                return self.now

            def sleep(self, seconds: float) -> None:
                self.slept.append(seconds)
                self.now += seconds

        for now, sleeps in ((108.0, [307.0]), (500.0, [])):
            with self.subTest(now=now):
                clock = Clock(now)
                waited = smoke.wait_turn_lease_expiry(
                    100.0, monotonic=clock.monotonic, sleep=clock.sleep
                )
                self.assertEqual(clock.slept, sleeps)
                self.assertGreaterEqual(
                    waited["since_exit_secs"],
                    smoke.HERMES_TURN_LEASE_TTL_SECS + smoke.HERMES_TURN_LEASE_MARGIN_SECS,
                )

    def test_turn_lease_check_rejects_missing_changed_extended_and_live_leases(self) -> None:
        exited = 1_000.0
        lease = {
            "conversation_id": "room",
            "holder": "pid=27:turn=killed:platform=finitechat",
            "acquired_at": exited - 10,
            "expires_at": exited + smoke.HERMES_TURN_LEASE_TTL_SECS - 5,
        }
        restart = exited + smoke.HERMES_TURN_LEASE_TTL_SECS + smoke.HERMES_TURN_LEASE_MARGIN_SECS
        records = smoke.require_turn_leases_lapsed(
            [lease], [dict(lease)], exited_at=exited, now=restart
        )
        self.assertEqual(records[0]["expired_secs_before_restart"], 20.0)
        refreshed = {**lease, "expires_at": exited + smoke.HERMES_TURN_LEASE_TTL_SECS + 1}
        for name, before, after, now in (
            ("missing", [], [], restart),
            ("changed", [lease], [refreshed], restart),
            ("refreshed_after_exit", [refreshed], [refreshed], restart),
            ("still_live", [lease], [lease], lease["expires_at"]),
        ):
            with self.subTest(name), self.assertRaises(smoke.SmokeFailure):
                smoke.require_turn_leases_lapsed(before, after, exited_at=exited, now=now)

    def test_sigkill_waits_out_the_turn_lease_before_restart(self) -> None:
        source = SCRIPT_PATH.read_text(encoding="utf-8")
        order = [
            "exited_monotonic, exited_at = time.monotonic(), time.time()",
            'set_stage("remove_container", case)',
            'set_stage("simulate_lease_expiry", case)',
            'set_stage("wait_hermes_turn_lease_expiry", case)',
            'set_stage("restart_agent", case)',
        ]
        positions = [source.index(step) for step in order]
        self.assertEqual(positions, sorted(positions))
        self.assertEqual(source.count("wait_stopped_turn_lease_expiry("), 2)

    def test_production_lease_ttl_matches_the_sidecar(self) -> None:
        source = SIDECAR_INBOX_PATH.read_text(encoding="utf-8")
        match = re.search(
            r"const DEFAULT_HERMES_INBOX_LEASE_TTL_MILLIS: u64 = (\d+) \* (\d+) \* (\d+);", source
        )
        assert match is not None
        a, b, c = (int(group) for group in match.groups())
        self.assertEqual(smoke.PRODUCTION_LEASE_TTL_MS, a * b * c)
        self.assertIn(f'"{smoke.LEASE_TTL_ENV}"', source)

    def test_smoke_never_shortens_the_lease_ttl(self) -> None:
        source = SCRIPT_PATH.read_text(encoding="utf-8")
        self.assertEqual(source.count("LEASE_TTL_ENV"), 3)  # definition, guard, guard message
        self.assertIn('["docker", "exec", name, "printenv", LEASE_TTL_ENV]', source)

    def test_stopped_inbox_write_is_compare_and_swap_on_an_isolated_container(self) -> None:
        ok = subprocess.CompletedProcess([], 0, "", "")
        with mock.patch.object(smoke.smoke, "run", return_value=ok) as run:
            smoke.write_stopped_inbox(
                image="image",
                home_volume="home",
                expected_sha256="abc",
                fixture=Path("/tmp/fixture.json"),
            )
        command = run.call_args.args[0]
        self.assertEqual(command[command.index("--network") + 1], "none")
        self.assertIn("EXPECTED_SHA256=abc", command)
        self.assertIn(
            "type=bind,src=/tmp/fixture.json,dst=/fixture/hermes-inbox.json,readonly", command
        )
        self.assertIn('test "$(sha256sum "$f" | cut -d" " -f1)" = "$EXPECTED_SHA256"', command[-1])

    def test_case_flow_orders_the_inbox_contract_checks(self) -> None:
        source = SCRIPT_PATH.read_text(encoding="utf-8")
        flow = source[
            source.index("def interrupt_case(") : source.index("def simulate_stopped_lease_expiry(")
        ]
        stopped = flow.index("require_graceful_inbox_released(")
        removed = flow.index("smoke.docker_container_rm(name)")
        expiry = flow.index("simulate_stopped_lease_expiry(case_name")
        restart = flow.index("health = start_agent()")
        self.assertLess(flow.index('smoke.run(["docker", "kill", "--signal", "KILL"'), stopped)
        self.assertLess(stopped, removed)
        self.assertLess(removed, expiry)
        self.assertLess(expiry, restart)
        self.assertIn('if not kill:\n            set_stage("check_stopped_inbox_released"', flow)
        self.assertIn('if kill:\n            set_stage("simulate_lease_expiry"', flow)
        self.assertLess(restart, flow.index("require_restart_order("))
        self.assertLess(flow.index('case["fresh_turns"]'), flow.index("wait_inbox_settled("))
        self.assertLess(
            flow.index("wait_inbox_settled("), flow.index("require_acks_after_handoff(")
        )

    def test_settled_inbox_requires_both_acked_and_nothing_leased(self) -> None:
        inbox = self.released()
        inbox["events"] = [e for e in inbox["events"] if e["message_id"] not in self.IDS.values()]
        inbox["acked"] += [
            {"key": "room-a\x1f4\x1factive", "acked_at_ms": 1},
            {"key": "room-a\x1f6\x1fqueued", "acked_at_ms": 1},
        ]
        self.assertEqual(
            smoke.require_settled_out_of_inbox(inbox, message_ids=self.IDS)["leased_entries"], 0
        )
        with self.assertRaisesRegex(smoke.SmokeFailure, "queued still in inbox"):
            smoke.require_settled_out_of_inbox(self.released(), message_ids=self.IDS)
        leased = copy.deepcopy(inbox)
        leased["events"][0]["lease"] = {"state": "leased", "lease_id": "x", "leased_at_ms": 1}
        with self.assertRaisesRegex(smoke.SmokeFailure, "leased seqs"):
            smoke.require_settled_out_of_inbox(leased, message_ids=self.IDS)

    def test_restart_order_requires_one_queued_handoff_after_the_rerun(self) -> None:
        def req(text: str) -> dict:
            return {"latest_user_text": text}

        active = req("FINITE_INTERRUPT_STALL:c keep this turn open")
        queued = req("Reply with exactly: c queued follow-up ok")
        check = lambda requests: smoke.require_restart_order(  # noqa: E731
            requests, active_marker="FINITE_INTERRUPT_STALL:c", queued_expected="c queued"
        )
        self.assertEqual(check([active, queued]), {"active_reruns": 1, "queued_handoffs": 1})
        self.assertEqual(check([queued]), {"active_reruns": 0, "queued_handoffs": 1})
        for requests, message in (
            ([active, queued, queued], "2 times"),
            ([active], "0 times"),
            ([queued, active], "before the interrupted"),
            ([active, active, queued], "reran 2 times"),
        ):
            with self.subTest(message), self.assertRaisesRegex(smoke.SmokeFailure, message):
                check(requests)

    def test_acks_must_follow_the_model_handoff_after_restart(self) -> None:
        def req(text: str, received_at_ms: int) -> dict:
            return {"latest_user_text": text, "received_at_ms": received_at_ms}

        def inbox(active_ack: int, queued_ack: int) -> dict:
            return {
                "events": [],
                "acked": [
                    {"key": "room-a\x1f4\x1factive", "acked_at_ms": active_ack},
                    {"key": "room-a\x1f6\x1fqueued", "acked_at_ms": queued_ack},
                ],
            }

        markers = {"active": "FINITE_INTERRUPT_STALL:c", "queued": "c queued follow-up ok"}
        requests = [
            req("FINITE_INTERRUPT_STALL:c keep this turn open", 1_000),
            req("Reply with exactly: c queued follow-up ok", 1_100),
        ]
        check = lambda requests, inbox: smoke.require_acks_after_handoff(  # noqa: E731
            requests, inbox, message_ids=self.IDS, markers=markers
        )
        self.assertEqual(check(requests, inbox(1_050, 1_150)), {"active": 50, "queued": 50})
        # Run 37428809877: both acked about 2.2s before the first model request.
        with self.assertRaisesRegex(smoke.SmokeFailure, "active turn was acked 2260 ms before"):
            check(
                [req(r["latest_user_text"], r["received_at_ms"] + 2_260) for r in requests],
                inbox(1_000, 1_036),
            )
        with self.assertRaisesRegex(smoke.SmokeFailure, "active turn was acked without reaching"):
            check(requests[1:], inbox(1_050, 1_150))
        with self.assertRaisesRegex(smoke.SmokeFailure, "queued turn has no ack time"):
            check(requests, {"acked": inbox(1_050, 1_150)["acked"][:1]})

    def test_dispatch_workflow_runs_and_uploads_the_matrix(self) -> None:
        workflow = WORKFLOW_PATH.read_text(encoding="utf-8")
        self.assertIn("chat_interruption_smoke:", workflow)
        self.assertIn("../#finitechat-server", workflow)
        self.assertIn("FINITECHAT_SERVER_BIN=", workflow)
        self.assertIn("scripts/hermes-chat-interruption-docker-smoke.py", workflow)
        self.assertIn("target/hermes-chat-interruption-docker-smoke/report.json", workflow)


if __name__ == "__main__":
    unittest.main()
