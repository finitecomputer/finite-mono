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

BASE_REGRESSIONS: dict[str, list[str]] = {
    "plain message mapping": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_poll_event_maps_room_to_chat_and_conversation_to_thread_then_acks",
    ],
    "redelivery dedupe": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_duplicate_redelivery_is_acked_without_second_dispatch",
    ],
    "ack retry without duplicate dispatch": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_ack_failure_retries_without_dispatching_duplicate",
    ],
    "durable busy-text admission": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_busy_text_waits_unacked_then_admits_in_inbox_order",
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_deferred_text_survives_adapter_restart_until_admission",
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_controls_bypass_busy_text_admission_gate",
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_active_session_does_not_block_another_session",
    ],
    "Hermes clarification routing": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_clarification_uses_hermes_prompt_on_exact_ordinary_message_route",
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_clarification_without_exact_route_fails_without_home_fallback",
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
    "adapter state compatibility": [
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_unversioned_adapter_state_is_adopted_without_losing_data",
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_new_adapter_state_initialization_is_atomic",
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_future_adapter_state_is_preserved_and_rejected",
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_unversioned_unknown_schema_object_is_not_adopted",
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_malformed_adapter_state_preserves_explicit_sends",
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_transient_state_failure_recovers_without_restart",
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_failed_dedup_write_retries_without_redispatch",
        "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_persisted_dedup_eviction_is_deterministic_when_timestamps_tie",
    ],
}

DURABILITY_SCENARIOS: dict[str, dict[str, Any]] = {
    "restart after route learning preserves reply scope": {
        "tests": [
            "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_reply_route_survives_adapter_restart",
        ],
        "failure_point": "adapter restarts after learning an inbound Topic/Chat route and before sending the reply",
        "restart_boundary": "after inbound dispatch and ack; before outbound reply",
        "asserted_observations": {
            "dispatch_count": 1,
            "ack_attempt_count": 1,
            "successful_ack_count": 1,
            "turn_completion_count": 1,
            "route_before_restart": {
                "conversation_id": "topic-build",
                "segment_id": "chat-build-1",
            },
            "route_after_restart": {
                "conversation_id": "topic-build",
                "segment_id": "chat-build-1",
            },
        },
    },
    "unknown reply route warns before Home fallback": {
        "tests": [
            "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_unscoped_fallback_logs_warning_with_route_key",
        ],
        "failure_point": "an outbound reply names a route key that the adapter cannot resolve",
        "restart_boundary": None,
        "asserted_observations": {
            "dispatch_count": 0,
            "ack_attempt_count": 0,
            "successful_ack_count": 0,
            "turn_completion_count": 0,
            "route_before_restart": None,
            "route_after_restart": None,
            "warning_contains_room_and_route_key": True,
        },
    },
    "intentional unscoped Home send stays quiet": {
        "tests": [
            "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_unscoped_home_send_without_route_key_stays_silent",
        ],
        "failure_point": "an intentional Home send has no Topic/Chat route key",
        "restart_boundary": None,
        "asserted_observations": {
            "dispatch_count": 0,
            "ack_attempt_count": 0,
            "successful_ack_count": 0,
            "turn_completion_count": 0,
            "route_before_restart": None,
            "route_after_restart": None,
            "warning_count": 0,
        },
    },
    "in-flight turn retains inbox ownership until completion": {
        "tests": [
            "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_turn_completion_hook_owns_the_ack",
        ],
        "failure_point": "the inbox redelivers while the original turn is still running",
        "restart_boundary": None,
        "asserted_observations": {
            "dispatch_count": 1,
            "ack_attempt_count": 1,
            "successful_ack_count": 1,
            "turn_completion_count": 1,
            "route_before_restart": None,
            "route_after_restart": None,
        },
    },
    "pre-completion handler failure leaves event for redelivery": {
        "tests": [
            "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_processing_failure_before_completion_leaves_event_unacked",
        ],
        "failure_point": "message handling raises before the terminal completion hook",
        "restart_boundary": None,
        "asserted_observations": {
            "dispatch_count": 2,
            "ack_attempt_count": 1,
            "successful_ack_count": 1,
            "turn_completion_count": 1,
            "route_before_restart": None,
            "route_after_restart": None,
        },
    },
    "terminal failure acks completed turn": {
        "tests": [
            "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_failed_turn_is_acked_after_the_turn_ran",
        ],
        "failure_point": "the turn completes with a terminal failure after producing its failure response",
        "restart_boundary": None,
        "asserted_observations": {
            "dispatch_count": 1,
            "ack_attempt_count": 1,
            "successful_ack_count": 1,
            "turn_completion_count": 1,
            "route_before_restart": None,
            "route_after_restart": None,
        },
    },
    "cancelled turn leaves event for redelivery": {
        "tests": [
            "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_cancelled_turn_stays_unacked_and_redelivery_reprocesses",
        ],
        "failure_point": "the terminal completion hook reports an in-flight turn as cancelled",
        "restart_boundary": None,
        "asserted_observations": {
            "dispatch_count": 2,
            "ack_attempt_count": 0,
            "successful_ack_count": 0,
            "turn_completion_count": 1,
            "route_before_restart": None,
            "route_after_restart": None,
        },
    },
    "restart after processing before ack suppresses duplicate turn": {
        "tests": [
            "tests.hermes.test_finite_platform_adapter.FinitePlatformAdapterTests.test_persisted_dedup_acks_redelivery_without_reprocessing_after_restart",
        ],
        "failure_point": "the process restarts after durable turn completion but before its ack lands",
        "restart_boundary": "after persisted completion and failed ack; before inbox redelivery",
        "asserted_observations": {
            "dispatch_count": 1,
            "ack_attempt_count": 2,
            "successful_ack_count": 1,
            "turn_completion_count": 1,
            "route_before_restart": None,
            "route_after_restart": None,
        },
    },
    "pinned Hermes owner task retains ack until completion": {
        "tests": [
            "tests.hermes.test_pinned_hermes_sender_context.PinnedHermesQueueAdmissionTests.test_real_018_owner_task_blocks_ack_until_followup_turn_completes",
        ],
        "failure_point": "a queued message is admitted after the prior pinned Hermes owner releases",
        "restart_boundary": None,
        "asserted_observations": {
            "dispatch_count": 1,
            "ack_attempt_count": 1,
            "successful_ack_count": 1,
            "turn_completion_count": 1,
            "route_before_restart": None,
            "route_after_restart": None,
        },
    },
}

REQUIRED_REGRESSIONS: dict[str, list[str]] = {
    **BASE_REGRESSIONS,
    **{name: list(scenario["tests"]) for name, scenario in DURABILITY_SCENARIOS.items()},
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
        "durability_scenarios": [
            {
                "name": name,
                "status": aggregate_status(list(scenario["tests"]), observed),
                "failure_point": scenario["failure_point"],
                "restart_boundary": scenario["restart_boundary"],
                "asserted_observations": scenario["asserted_observations"],
                "tests": scenario["tests"],
            }
            for name, scenario in DURABILITY_SCENARIOS.items()
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
