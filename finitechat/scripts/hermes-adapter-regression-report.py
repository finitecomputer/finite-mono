#!/usr/bin/env python3
"""Run focused Hermes adapter regressions and emit JSON evidence."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

REQUIRED_REGRESSIONS: dict[str, list[str]] = {
    "plain message mapping": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_poll_event_maps_room_to_chat_and_conversation_to_thread_then_acks",
    ],
    "redelivery dedupe": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_duplicate_redelivery_is_acked_without_second_dispatch",
    ],
    "ack retry without duplicate dispatch": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_ack_failure_retries_without_dispatching_duplicate",
    ],
    "transient poll recovery": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_poll_loop_continues_after_transient_poll_error",
    ],
    "sidecar startup": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_ensure_service_starts_finitechat_serve_and_reads_ready_file",
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_ensure_service_waits_for_health_after_ready_file",
    ],
    "service fallback": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_finitechat_json_falls_back_to_cli_when_service_transport_fails",
    ],
    "service serialization": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_finitechat_json_serializes_cli_access_per_adapter",
    ],
    "media attachments": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_media_send_uses_typed_attachment_payload",
    ],
    "outbound edit route": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_edit_reuses_thread_route_from_original_send",
    ],
    "typing activity": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_typing_activity_uses_ephemeral_bridge_and_clears_same_thread_route",
    ],
    "room filtering": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_room_filter_drops_other_rooms_but_unfiltered_serves_all",
    ],
    "group sender identity": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_group_poll_event_preserves_authenticated_sender_identity",
    ],
    "receipt/control stream filtering": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_stream_loop_skips_typed_receipt_records_without_dispatch_or_ack",
    ],
    "strict inbound stream recovery": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_stream_loop_reconnects_and_catches_up_without_poll_fallback",
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_strict_stream_service_failure_never_falls_back_to_cli",
    ],
}


def flattened_tests() -> list[str]:
    tests: list[str] = []
    for names in REQUIRED_REGRESSIONS.values():
        tests.extend(names)
    return tests


def tail(text: str, lines: int) -> str:
    return "\n".join(text.splitlines()[-lines:])


def build_report(args: argparse.Namespace) -> tuple[int, dict[str, Any]]:
    started = time.monotonic()
    test_names = flattened_tests()
    command = [args.python, "-m", "unittest", "-v", *test_names]
    result = subprocess.run(command, capture_output=True, text=True, timeout=args.timeout)
    passed = result.returncode == 0
    report = {
        "status": "passed" if passed else "failed",
        "generated_at_unix": int(time.time()),
        "elapsed_ms": int((time.monotonic() - started) * 1000),
        "proof_layers": sorted(REQUIRED_REGRESSIONS),
        "regressions": [
            {"name": name, "tests": tests} for name, tests in REQUIRED_REGRESSIONS.items()
        ],
        "test_count": len(test_names),
        "command": command,
        "returncode": result.returncode,
        "stdout_tail": tail(result.stdout, 40),
        "stderr_tail": tail(result.stderr, 80),
    }
    return result.returncode, report


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
