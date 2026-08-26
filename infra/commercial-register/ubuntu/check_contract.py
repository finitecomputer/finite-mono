#!/usr/bin/env python3
"""Values-free deployment contract for the commercial register."""

from __future__ import annotations

import json
import re
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
COMPOSE = HERE / "compose.yaml"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def parse_env(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw in read(path).splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        require(separator == "=", f"invalid assignment in {path}: {raw!r}")
        values[key] = value
    return values


def main() -> None:
    versions = parse_env(HERE / "versions.env")
    compose = read(COMPOSE)
    package = json.loads(read(ROOT / "commercial-register/package.json"))

    require(versions["TWENTY_VERSION"] == "2.35.0", "Twenty version drifted")
    require(
        package["devDependencies"]["twenty-sdk"] == versions["TWENTY_VERSION"],
        "Twenty SDK and server image versions must move together",
    )
    for name in ("TWENTY_IMAGE", "POSTGRES_IMAGE", "REDIS_IMAGE"):
        image = versions[name]
        require(
            re.fullmatch(r"[a-z0-9./-]+@sha256:[0-9a-f]{64}", image) is not None,
            f"{name} is not digest-pinned",
        )
        require(image in compose, f"compose.yaml does not use {name}")

    require("127.0.0.1:3020:3000" in compose, "Twenty must bind only to loopback")
    require("LOGIC_FUNCTION_TYPE: DISABLED" in compose, "logic functions must stay disabled")
    require("CODE_INTERPRETER_TYPE: DISABLED" in compose, "code interpreter must stay disabled")
    require("/etc/finite/commercial-register/twenty.env" in compose, "Twenty env file missing")
    require("/etc/finite/commercial-register/postgres.env" in compose, "Postgres env file missing")

    caddy = read(ROOT / "infra/monitoring/ubuntu/Caddyfile")
    prometheus = read(ROOT / "infra/monitoring/ubuntu/prometheus.yml")
    require("crm.finite.computer" in caddy, "Caddy CRM origin missing")
    require("reverse_proxy 127.0.0.1:3020" in caddy, "Caddy CRM upstream drifted")
    require("job_name: crm.finite.computer" in prometheus, "CRM blackbox probe missing")
    require("https://crm.finite.computer/healthz" in prometheus, "CRM health target drifted")

    expected = {
        "finite-commercial-register.service",
        "finite-commercial-register-backup.service",
        "finite-commercial-register-backup.timer",
        "finite-commercial-register-health.service",
        "finite-commercial-register-health.timer",
    }
    require(
        {path.name for path in (HERE / "systemd").iterdir()} == expected,
        "commercial-register systemd unit set drifted",
    )
    for script in [HERE / "deploy", *(HERE / "scripts").iterdir()]:
        result = subprocess.run(["bash", "-n", str(script)], capture_output=True, text=True)
        require(result.returncode == 0, f"{script} failed bash -n: {result.stderr}")

    deploy = read(HERE / "deploy")
    for required in (
        "--activate",
        "SERVER_URL=http://localhost:3020",
        "scripts/finite-status",
        "finite-status-before.json",
        "finite-status-after.json",
        "finite-commercial-register-backup.service",
        "/opt/finite-commercial-register/bin/restore-check",
    ):
        require(required in deploy, f"deploy missing {required!r}")

    publish = read(HERE / "scripts/publish-url")
    for required in (
        "--activate",
        "SERVER_URL=https://crm.finite.computer",
        "finite-status-before.json",
        "finite-status-after.json",
        "/opt/finite-commercial-register/bin/backup",
        "/opt/finite-commercial-register/bin/restore-check",
    ):
        require(required in publish, f"publish-url missing {required!r}")

    monitoring_deploy = read(ROOT / "infra/monitoring/ubuntu/deploy")
    require(
        'https://crm.finite.computer/healthz "public commercial register health"'
        in monitoring_deploy,
        "monitoring edge deploy does not prove public CRM health",
    )

    forbidden = re.compile(r"(?i)(password|secret|token|api[_-]?key)=([^$\n{][^\n]*)")
    for path in [COMPOSE, HERE / "versions.env", *list((HERE / "systemd").iterdir())]:
        require(forbidden.search(read(path)) is None, f"possible secret value in {path}")

    print("commercial-register production contract OK")


if __name__ == "__main__":
    main()
