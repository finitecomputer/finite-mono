"""Time real service builds on one disposable runner per packaging engine."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("engine", choices=["crane", "crate2nix"])
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    evidence = output / "sources"
    started = time.monotonic()
    subprocess.run(
        [sys.executable, str(HERE / "probe.py"), "--output", str(evidence)], check=True
    )
    setup_s = round(time.monotonic() - started, 3)
    report = {
        "engine": args.engine,
        "setup_and_invalidation_probe_s": setup_s,
        "source_revision": json.loads((evidence / "report.json").read_text())[
            "revision"
        ],
        "runner": os.environ.get("RUNNER_NAME"),
        "system": subprocess.check_output(
            ["nix", "eval", "--impure", "--raw", "--expr", "builtins.currentSystem"],
            text=True,
        ).strip(),
        "substituters": "https://cache.nixos.org",
        "nix_cores": 8,
        "nix_max_jobs": 8,
        "packages": ["finitesitesd", "finite-saas-core"],
        "cases": [],
    }
    failed = False
    for case in ["baseline", "identical", "sites-source", "sites-dependency"]:
        print(f"Building {args.engine}: {case}", flush=True)
        command = [
            "nix-build",
            str(HERE / "benchmark.nix"),
            "--no-out-link",
            "--argstr",
            "source",
            str(evidence / case),
            "--argstr",
            "engine",
            args.engine,
            "--option",
            "builders",
            "",
            "--option",
            "substituters",
            "https://cache.nixos.org",
            "--cores",
            "8",
            "--max-jobs",
            "8",
        ]
        started = time.monotonic()
        with (
            (output / f"{case}.stdout").open("w") as stdout,
            (output / f"{case}.log").open("w") as stderr,
        ):
            result = subprocess.run(command, stdout=stdout, stderr=stderr)
        row = {
            "case": case,
            "elapsed_s": round(time.monotonic() - started, 3),
            "exit_code": result.returncode,
        }
        report["cases"].append(row)
        (output / "timings.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(row), flush=True)
        if result.returncode:
            print((output / f"{case}.log").read_text()[-12000:], file=sys.stderr)
            failed = True
            break
    summary = [
        f"## {args.engine}: service packaging benchmark",
        "",
        "Services: `finitesitesd` and unrelated `finite-saas-core`.",
        "",
        "| Case | Elapsed | Result |",
        "| --- | ---: | --- |",
    ]
    for row in report["cases"]:
        summary.append(
            f"| {row['case']} | {row['elapsed_s']:.1f}s | {'passed' if row['exit_code'] == 0 else 'FAILED'} |"
        )
    summary.extend(
        [
            "",
            f"Snapshot generation and invalidation probes: {setup_s:.1f}s (reported separately).",
            "",
            "Initial build allows public Nix cache substitution but excludes Finite's production cache.",
            "Subsequent cases reuse this job's local store. Identical is a warm local rebuild, not a fresh-runner cache test.",
            "Each case starts from the baseline source snapshot. No candidate is uploaded to the production cache.",
            "Both engines use the production packaging compiler; their default codegen/profile behavior may differ.",
            "This measures packaging, not service correctness or end-to-end CI speed.",
        ]
    )
    markdown = "\n".join(summary) + "\n"
    (output / "summary.md").write_text(markdown)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a") as destination:
            destination.write(markdown)
    print(markdown)
    raise SystemExit(1 if failed else 0)


if __name__ == "__main__":
    main()
