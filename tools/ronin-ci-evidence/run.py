"""Offline wrapper contract, with a fixed fake executable and injectable executor."""

import argparse
import json
import math
import os
import selectors
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

from contract import (
    ContractError,
    Validation,
    check_manifest,
    read_json,
    validate_evidence,
)

MAX_FAKE_SECONDS = 2.0
MAX_FAKE_OUTPUT_BYTES = 65536
STATUS_EXITS = {"FIXTURE_PASS": 0, "UNSUPPORTED": 77, "INCONCLUSIVE": 78, "ERROR": 65}


@dataclass(frozen=True)
class FakeBudget:
    """Only this inert executor's wall time and combined output, no runtime policy."""

    seconds: float = MAX_FAKE_SECONDS
    output_bytes: int = MAX_FAKE_OUTPUT_BYTES

    def __post_init__(self):
        if (
            type(self.seconds) not in (int, float)
            or not math.isfinite(self.seconds)
            or not 0 < self.seconds <= MAX_FAKE_SECONDS
        ):
            raise ValueError("fake timeout must be finite and in (0, 2] seconds")
        if (
            type(self.output_bytes) is not int
            or not 1 <= self.output_bytes <= MAX_FAKE_OUTPUT_BYTES
        ):
            raise ValueError("fake combined output budget must be in 1..65536 bytes")


@dataclass(frozen=True)
class Execution:
    returncode: int | None
    stdout: bytes = b""
    stderr: bytes = b""
    timed_out: bool = False
    output_limited: bool = False
    error: str | None = None


def execute_fake(command, budget):
    """Drain both pipes within a byte ceiling; reap the owned fake on every path."""
    started = time.monotonic()
    try:
        process = subprocess.Popen(
            command, stdout=subprocess.PIPE, stderr=subprocess.PIPE
        )
    except OSError as exc:
        return Execution(None, error=str(exc))
    stdout = bytearray()
    stderr = bytearray()
    timed_out = False
    output_limited = False
    error = None
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ, stdout)
            selector.register(process.stderr, selectors.EVENT_READ, stderr)
            while selector.get_map():
                remaining_time = budget.seconds - (time.monotonic() - started)
                if remaining_time <= 0:
                    timed_out = True
                    break
                for key, _ in selector.select(remaining_time):
                    remaining_bytes = budget.output_bytes - len(stdout) - len(stderr)
                    chunk = os.read(key.fd, min(4096, remaining_bytes + 1))
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    key.data.extend(chunk[:remaining_bytes])
                    if len(chunk) > remaining_bytes:
                        output_limited = True
                        break
                if output_limited:
                    break
            if not timed_out and not output_limited:
                try:
                    process.wait(
                        timeout=max(0, budget.seconds - (time.monotonic() - started))
                    )
                except subprocess.TimeoutExpired:
                    timed_out = True
    except OSError as exc:
        error = str(exc)
    finally:
        if process.poll() is None:
            process.kill()
        # Fixed fake_tool has no descendants or privileged resources. wait()
        # records the actual signed returncode, including wrapper termination.
        process.wait()
        process.stdout.close()
        process.stderr.close()
    return Execution(
        process.returncode,
        bytes(stdout),
        bytes(stderr),
        timed_out,
        output_limited,
        error,
    )


def _report(command, budget, execution, validation, status, outer, reasons):
    return {
        "schema_version": 1,
        "kind": "OFFLINE_FIXTURE_RESULT",
        "status": status,
        "fixture_pass": status == "FIXTURE_PASS" and outer == 0,
        "real_mechanism_qualified": False,
        "command": list(command),
        "inner_returncode": execution.returncode,
        "outer_exit_code": outer,
        "timed_out": execution.timed_out,
        "output_limited": execution.output_limited,
        "fake_budget": {
            "seconds": budget.seconds,
            "combined_output_bytes": budget.output_bytes,
        },
        "stdout": execution.stdout.decode("utf-8", errors="replace"),
        "stderr": execution.stderr.decode("utf-8", errors="replace"),
        "evidence_status": validation.status,
        "evidence_reasons": list(validation.reasons),
        "reasons": list(reasons),
    }


def run_fixture(manifest, scenario_path, *, executor=execute_fake, budget=FakeBudget()):
    """Manifest validation precedes execution. Only the fixed fake can be launched."""
    command = (
        sys.executable,
        "-B",
        str(Path(__file__).with_name("fake_tool.py")),
        str(scenario_path),
    )
    try:
        check_manifest(manifest)
    except ContractError as exc:
        invalid = Validation("ERROR", (str(exc),))
        return _report(
            command, budget, Execution(None), invalid, "ERROR", 64, invalid.reasons
        )
    try:
        execution = executor(command, budget)
        if (
            type(execution) is not Execution
            or type(execution.stdout) is not bytes
            or type(execution.stderr) is not bytes
            or type(execution.timed_out) is not bool
            or type(execution.output_limited) is not bool
            or (execution.error is not None and type(execution.error) is not str)
        ):
            raise ValueError("invalid typed executor result")
    except Exception as exc:
        execution = Execution(
            None, error=f"executor exception: {type(exc).__name__}: {exc}"
        )
    validation = validate_evidence(manifest, execution.stdout)
    status = validation.status
    outer = STATUS_EXITS[status]
    reasons = validation.reasons
    # Content validity alone never suppresses a command failure or wrapper guard.
    if execution.error is not None:
        status, outer, reasons = "ERROR", 127, (execution.error,)
    elif execution.timed_out:
        status, outer, reasons = "INCONCLUSIVE", 124, ("fake executor timeout",)
    elif (
        execution.output_limited
        or len(execution.stdout) + len(execution.stderr) > budget.output_bytes
    ):
        status, outer, reasons = "ERROR", 125, ("fake executor output limit exceeded",)
    elif (
        type(execution.returncode) is not int or not -127 <= execution.returncode <= 255
    ):
        status, outer, reasons = "ERROR", 127, ("missing or invalid inner returncode",)
    elif execution.returncode != 0:
        # Preserve positive process status; preserve negative signal status in
        # inner_returncode and expose its conventional 128+signal outer status.
        outer = (
            execution.returncode
            if execution.returncode > 0
            else 128 - execution.returncode
        )
        status, reasons = "ERROR", (f"fake command returned {execution.returncode}",)
    return _report(command, budget, execution, validation, status, outer, reasons)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--scenario", type=Path, required=True)
    parser.add_argument("--fake-timeout-seconds", type=float, default=MAX_FAKE_SECONDS)
    parser.add_argument("--fake-output-bytes", type=int, default=MAX_FAKE_OUTPUT_BYTES)
    args = parser.parse_args()
    try:
        budget = FakeBudget(args.fake_timeout_seconds, args.fake_output_bytes)
        manifest = read_json(args.manifest)
    except (OSError, ContractError, ValueError) as exc:
        report = {
            "kind": "OFFLINE_FIXTURE_RESULT",
            "status": "ERROR",
            "fixture_pass": False,
            "real_mechanism_qualified": False,
            "inner_returncode": None,
            "outer_exit_code": 64,
            "reasons": [str(exc)],
        }
    else:
        report = run_fixture(manifest, args.scenario, budget=budget)
    print(json.dumps(report, sort_keys=True, separators=(",", ":")))
    return report["outer_exit_code"]


if __name__ == "__main__":
    sys.exit(main())
