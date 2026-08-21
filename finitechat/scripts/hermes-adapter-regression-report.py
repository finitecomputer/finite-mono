#!/usr/bin/env python3
"""Run the Hermes-internal adapter regressions and emit JSON evidence.

Inbox in-flight state, reply/edit route resolution, and delivered-event dedup
now live in the Rust sidecar (ownership audit O1/O2) and are proven by the
Rust integration/unit tests, not here. This gate covers only the behaviours
that remain the Python adapter's responsibility: busy-session admission,
clarification routing, transport/service plumbing, media mapping, activity,
room filtering, sender identity, and the strict inbound stream.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

ADAPTER_TESTS = "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests"

REQUIRED_REGRESSIONS: dict[str, list[str]] = {
    "plain message mapping": [
        f"{ADAPTER_TESTS}.test_poll_event_maps_room_to_chat_and_conversation_to_thread_then_acks",
    ],
    "durable busy-text admission": [
        f"{ADAPTER_TESTS}.test_busy_text_waits_unacked_then_admits_in_inbox_order",
        f"{ADAPTER_TESTS}.test_deferred_text_survives_adapter_restart_until_admission",
        f"{ADAPTER_TESTS}.test_controls_bypass_busy_text_admission_gate",
        f"{ADAPTER_TESTS}.test_active_session_does_not_block_another_session",
    ],
    "Hermes clarification routing": [
        f"{ADAPTER_TESTS}.test_clarification_uses_hermes_prompt_on_exact_ordinary_message_route",
        f"{ADAPTER_TESTS}.test_clarification_without_exact_route_fails_without_home_fallback",
    ],
    "transient poll recovery": [
        f"{ADAPTER_TESTS}.test_poll_loop_continues_after_transient_poll_error",
    ],
    "sidecar startup": [
        f"{ADAPTER_TESTS}.test_ensure_service_starts_finitechat_serve_and_reads_ready_file",
        f"{ADAPTER_TESTS}.test_ensure_service_waits_for_health_after_ready_file",
    ],
    "service fallback": [
        f"{ADAPTER_TESTS}.test_finitechat_json_falls_back_to_cli_when_service_transport_fails",
    ],
    "service serialization": [
        f"{ADAPTER_TESTS}.test_finitechat_json_serializes_cli_access_per_adapter",
    ],
    "media attachments": [
        f"{ADAPTER_TESTS}.test_media_send_uses_typed_attachment_payload",
    ],
    "typing activity": [
        f"{ADAPTER_TESTS}.test_typing_activity_uses_ephemeral_bridge_and_clears_same_thread_route",
    ],
    "room filtering": [
        f"{ADAPTER_TESTS}.test_room_filter_drops_other_rooms_but_unfiltered_serves_all",
    ],
    "group sender identity": [
        f"{ADAPTER_TESTS}.test_group_poll_event_preserves_authenticated_sender_identity",
    ],
    "receipt/control stream filtering": [
        f"{ADAPTER_TESTS}.test_stream_loop_skips_typed_receipt_records_without_dispatch_or_ack",
    ],
    "strict inbound stream recovery": [
        f"{ADAPTER_TESTS}.test_stream_loop_reconnects_and_catches_up_without_poll_fallback",
        f"{ADAPTER_TESTS}.test_strict_stream_service_failure_never_falls_back_to_cli",
    ],
}


def flattened_tests() -> list[str]:
    tests: list[str] = []
    for names in REQUIRED_REGRESSIONS.values():
        tests.extend(names)
    return tests


def tail(text: str, lines: int) -> str:
    return "\n".join(text.splitlines()[-lines:])


def output_text(value: str | bytes | None) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return value


def observed_test_results(output: str, test_names: list[str]) -> dict[str, str]:
    lines = output.splitlines()
    starts: dict[str, int] = {}
    for test_name in test_names:
        starts[test_name] = next(
            (index for index, line in enumerate(lines) if test_name in line),
            -1,
        )

    results: dict[str, str] = {}
    for test_name in test_names:
        start = starts[test_name]
        if start < 0:
            results[test_name] = "missing"
            continue

        later_starts = [index for index in starts.values() if index > start]
        end = min(later_starts) if later_starts else len(lines)
        block = [line.strip() for line in lines[start:end]]

        if any(line == "ok" or line.endswith(" ... ok") for line in block):
            results[test_name] = "passed"
        elif any(line.startswith("skipped ") or " ... skipped" in line for line in block):
            results[test_name] = "skipped"
        elif any(
            line in {"FAIL", "ERROR"} or line.endswith(" ... FAIL") or line.endswith(" ... ERROR")
            for line in block
        ):
            results[test_name] = "failed"
        else:
            results[test_name] = "missing"
    return results


def aggregate_status(tests: list[str], results: dict[str, str]) -> str:
    statuses = {results[test] for test in tests}
    if statuses == {"passed"}:
        return "passed"
    for status in ("failed", "skipped", "missing"):
        if status in statuses:
            return status
    return "failed"


def build_report(args: argparse.Namespace) -> tuple[int, dict[str, Any]]:
    started = time.monotonic()
    test_names = flattened_tests()
    command = [args.python, "-m", "unittest", "-v", *test_names]
    timed_out = False
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=args.timeout)
    except subprocess.TimeoutExpired as error:
        timed_out = True
        result = subprocess.CompletedProcess(
            command,
            124,
            stdout=output_text(error.stdout),
            stderr=(
                output_text(error.stderr)
                + f"\nreliability gate timed out after {args.timeout} seconds"
            ),
        )
    observed = observed_test_results(f"{result.stdout}\n{result.stderr}", test_names)
    missing_tests = [name for name in test_names if observed[name] == "missing"]
    skipped_tests = [name for name in test_names if observed[name] == "skipped"]
    failed_tests = [name for name in test_names if observed[name] == "failed"]
    passed = result.returncode == 0 and not missing_tests and not skipped_tests and not failed_tests
    gate_exit_code = 0 if passed else result.returncode or 1
    regression_statuses = {
        name: aggregate_status(tests, observed) for name, tests in REQUIRED_REGRESSIONS.items()
    }
    report = {
        "status": "passed" if passed else "failed",
        "generated_at_unix": int(time.time()),
        "elapsed_ms": int((time.monotonic() - started) * 1000),
        "scope": (
            "Hermes-internal adapter regressions only; inbox lease/ack/release, "
            "reply/edit route resolution, and dedup are proven by the Rust sidecar tests"
        ),
        "required_proof_layers": sorted(REQUIRED_REGRESSIONS),
        "proof_layers": sorted(
            name for name, status in regression_statuses.items() if status == "passed"
        ),
        "regressions": [
            {
                "name": name,
                "status": regression_statuses[name],
                "tests": tests,
            }
            for name, tests in REQUIRED_REGRESSIONS.items()
        ],
        "test_count": len(test_names),
        "observed_test_count": sum(status != "missing" for status in observed.values()),
        "passed_test_count": sum(status == "passed" for status in observed.values()),
        "missing_tests": missing_tests,
        "skipped_tests": skipped_tests,
        "failed_tests": failed_tests,
        "test_results": [{"test": name, "status": observed[name]} for name in test_names],
        "command": command,
        "returncode": result.returncode,
        "gate_exit_code": gate_exit_code,
        "timed_out": timed_out,
        "stdout_tail": tail(result.stdout, 40),
        "stderr_tail": tail(result.stderr, 80),
    }
    return gate_exit_code, report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", default="target/hermes-adapter-regressions/report.json")
    parser.add_argument("--python", default=sys.executable)
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()

    status, report = build_report(args)
    report_path = Path(args.report)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(report, indent=2) + "\n"
    report_path.write_text(text, encoding="utf-8")
    print(text, end="")
    return status


if __name__ == "__main__":
    sys.exit(main())
