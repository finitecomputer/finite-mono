#!/usr/bin/env python3
"""Compare repeated Finite Private load-canary service metrics."""

from __future__ import annotations

import argparse
import json
import math
import re
import statistics
import sys
from dataclasses import dataclass
from pathlib import Path


COMPLETION_PATTERN = re.compile(
    r"^completion_seconds\s+.*\bp95=(?P<value>[0-9]+(?:\.[0-9]+)?)\b"
)
GENERATION_PATTERN = re.compile(
    r"^generation_tokens_per_second\s+"
    r".*\bper_request_p50=(?P<generation>[0-9]+(?:\.[0-9]+)?)\b"
    r".*\baggregate=(?P<aggregate>[0-9]+(?:\.[0-9]+)?)\b"
)
EXPECTED_RUNS = 3
MINIMUM_GENERATION_RATIO = 0.90
MAXIMUM_COMPLETION_RATIO = 1.25


class InputError(ValueError):
    pass


@dataclass(frozen=True)
class LoadSamples:
    completion_p95: tuple[float, ...]
    per_request_generation_p50: tuple[float, ...]
    aggregate: tuple[float, ...]


def _finite_positive(value: str, *, metric: str, path: Path) -> float:
    parsed = float(value)
    if not math.isfinite(parsed) or parsed <= 0:
        raise InputError(f"{path}: {metric} must be finite and positive")
    return parsed


def parse_load_log(path: Path, *, expected_runs: int) -> LoadSamples:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise InputError(f"could not read {path}: {error}") from error

    completion_p95: list[float] = []
    generation_p50: list[float] = []
    aggregate: list[float] = []
    for line in lines:
        completion_match = COMPLETION_PATTERN.match(line)
        if completion_match:
            completion_p95.append(
                _finite_positive(
                    completion_match.group("value"),
                    metric="completion p95",
                    path=path,
                )
            )
            continue
        generation_match = GENERATION_PATTERN.match(line)
        if generation_match:
            generation_p50.append(
                _finite_positive(
                    generation_match.group("generation"),
                    metric="per-request generation p50",
                    path=path,
                )
            )
            aggregate.append(
                _finite_positive(
                    generation_match.group("aggregate"),
                    metric="aggregate generation rate",
                    path=path,
                )
            )

    counts = (len(completion_p95), len(generation_p50), len(aggregate))
    if counts != (expected_runs, expected_runs, expected_runs):
        raise InputError(
            f"{path}: expected exactly {expected_runs} completion and generation "
            f"samples, found completion={counts[0]} generation={counts[1]} "
            f"aggregate={counts[2]}"
        )
    return LoadSamples(
        completion_p95=tuple(completion_p95),
        per_request_generation_p50=tuple(generation_p50),
        aggregate=tuple(aggregate),
    )


def compare(
    baseline: LoadSamples,
    candidate: LoadSamples,
) -> dict[str, object]:
    baseline_generation = statistics.median(baseline.per_request_generation_p50)
    candidate_generation = statistics.median(candidate.per_request_generation_p50)
    baseline_completion = statistics.median(baseline.completion_p95)
    candidate_completion = statistics.median(candidate.completion_p95)
    baseline_aggregate = statistics.median(baseline.aggregate)
    candidate_aggregate = statistics.median(candidate.aggregate)
    generation_ratio = candidate_generation / baseline_generation
    completion_ratio = candidate_completion / baseline_completion

    violations: list[str] = []
    if generation_ratio < MINIMUM_GENERATION_RATIO:
        violations.append("per_request_generation_rate")
    if completion_ratio > MAXIMUM_COMPLETION_RATIO:
        violations.append("completion_p95")

    return {
        "schema": "finite-private-load-comparison-v1",
        "passed": not violations,
        "per_request_generation_p50": {
            "baseline_median": baseline_generation,
            "candidate_median": candidate_generation,
            "candidate_to_baseline_ratio": round(generation_ratio, 6),
            "minimum_ratio": MINIMUM_GENERATION_RATIO,
        },
        "completion_p95": {
            "baseline_median": baseline_completion,
            "candidate_median": candidate_completion,
            "candidate_to_baseline_ratio": round(completion_ratio, 6),
            "maximum_ratio": MAXIMUM_COMPLETION_RATIO,
        },
        "aggregate": {
            "baseline_median": baseline_aggregate,
            "candidate_median": candidate_aggregate,
            "role": "diagnostic_only",
        },
        "violations": violations,
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Gate DeepSeek scheduler performance on repeated service-level metrics; "
            "public-edge aggregate throughput is reported but is not an acceptance gate."
        )
    )
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    arguments = parser.parse_args()

    try:
        baseline = parse_load_log(arguments.baseline, expected_runs=EXPECTED_RUNS)
        candidate = parse_load_log(arguments.candidate, expected_runs=EXPECTED_RUNS)
    except InputError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2

    report = compare(baseline, candidate)
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
