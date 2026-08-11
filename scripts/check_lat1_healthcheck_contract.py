#!/usr/bin/env python3
"""Verify lat1's aggregate healthcheck startup and failure contract."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONFIG = ".#nixosConfigurations.finite-lat-1.config"
PROBED_SERVICE_UNITS = [
    "finite-saas-core.service",
    "podman-finite-saas-dashboard.service",
    "finitechat-server.service",
    "finitechat-hosted-device.service",
    "finite-brain-app.service",
    "finite-saas-sites.service",
    "podman-searxng.service",
    "podman-firecrawl-api.service",
    "prometheus-node-exporter.service",
]


def nix_eval(attribute: str, *, raw: bool = False) -> str:
    command = ["nix", "eval", "--raw" if raw else "--json", f"{CONFIG}.{attribute}"]
    return subprocess.run(
        command,
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def run_synthetic(script: str, *, mode: str) -> subprocess.CompletedProcess[str]:
    if mode == "recover_after_first_pass":
        curl_body = """
  if [ "$curl_calls" -le 10 ]; then
    return 1
  fi
  case " $* " in
    *" -o /dev/null "*) return 0 ;;
  esac
  printf '%s\\n' '{"capabilities":["account_cohort_writes_v1"]}'
"""
    elif mode == "missing_brain_capability":
        curl_body = """
  case " $* " in
    *" -o /dev/null "*) return 0 ;;
  esac
  printf '%s\\n' '{"capabilities":[]}'
"""
    elif mode == "persistent_failure":
        curl_body = "return 1"
    else:
        raise ValueError(f"unknown synthetic healthcheck mode: {mode}")
    harness = f"""
curl_calls=0
curl() {{
  curl_calls=$((curl_calls + 1))
  {curl_body}
}}
sleep() {{ :; }}
trap 'echo curl_calls=$curl_calls' EXIT
eval "$SCRIPT"
"""
    return subprocess.run(
        ["bash", "-c", harness],
        check=False,
        capture_output=True,
        env={**os.environ, "SCRIPT": script},
        text=True,
    )


def main() -> None:
    after = json.loads(nix_eval("systemd.services.finite-healthcheck.after"))
    missing = sorted(set(PROBED_SERVICE_UNITS) - set(after))
    if missing:
        raise SystemExit(f"healthcheck ordering is missing units: {missing}")

    script = nix_eval("systemd.services.finite-healthcheck.script", raw=True)
    timeout = nix_eval(
        "systemd.services.finite-healthcheck.serviceConfig.TimeoutStartSec",
        raw=True,
    )
    if timeout != "2min":
        raise SystemExit(f"healthcheck start timeout is {timeout!r}, expected '2min'")

    recovery = run_synthetic(script, mode="recover_after_first_pass")
    recovery_output = recovery.stdout + recovery.stderr
    if recovery.returncode != 0:
        raise SystemExit(f"transient failure did not recover:\n{recovery_output}")
    for expected in (
        "WAIT finite-saas-core",
        "OK   finite-brain-cohort-writes",
        "OK   node-exporter",
        "curl_calls=27",
    ):
        if expected not in recovery_output:
            raise SystemExit(
                f"transient recovery output is missing {expected!r}:\n{recovery_output}"
            )

    missing_capability = run_synthetic(script, mode="missing_brain_capability")
    missing_capability_output = missing_capability.stdout + missing_capability.stderr
    if missing_capability.returncode != 1:
        raise SystemExit(
            "missing Brain cohort capability did not fail the healthcheck:\n"
            f"{missing_capability_output}"
        )
    for expected in (
        "FAIL finite-brain-cohort-writes",
        "curl_calls=117",
    ):
        if expected not in missing_capability_output:
            raise SystemExit(
                "missing-capability output is missing "
                f"{expected!r}:\n{missing_capability_output}"
            )

    failure = run_synthetic(script, mode="persistent_failure")
    failure_output = failure.stdout + failure.stderr
    if failure.returncode != 1:
        raise SystemExit(
            f"persistent failure exited {failure.returncode}, expected 1:\n"
            f"{failure_output}"
        )
    for expected in (
        "FAIL health endpoints remained unavailable after the bounded startup grace",
        "curl_calls=117",
    ):
        if expected not in failure_output:
            raise SystemExit(
                f"persistent failure output is missing {expected!r}:\n{failure_output}"
            )

    print("lat1 healthcheck contract: ok")


if __name__ == "__main__":
    main()
