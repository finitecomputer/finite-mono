#!/usr/bin/env python3
"""Verify all Kata Runner hosts share one runner-role declaration.

finite-lat-1, finite-lat-3, and finite-lat-4 import
infra/nixos/modules/kata-runner-host.nix,
which renders the shared non-secret environment to
/etc/finite/runner-shared.env and owns the finite-saas-runner unit shape. Host
configs pass only the declared per-host inputs. This guard evaluates both
nixosConfigurations and fails on any runner-role drift outside that declared
per-host set, so a future hand-edit to one host breaks CI instead of
production. (finite-lat-2 is the app-plane replacement host and deliberately
runs no runner; the runner lane moves to finite-lat-4.)
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
HOSTS = ["finite-lat-1", "finite-lat-3", "finite-lat-4"]

EXPECTED_MAX_SANDBOXES = {
    "finite-lat-1": "12",
    "finite-lat-3": "42",
    # finite-lat-4 mirrors lat3's owner-authorized ceiling; it is admitted
    # drained (FC_RUNNER_DRAIN is operator env, not shared env).
    "finite-lat-4": "42",
}

SHARED_ENV_PATH = "/etc/finite/runner-shared.env"
OPERATOR_ENV_PATH = "/etc/finite/runner.env"
OPERATOR_ENV_TEMPLATES = (
    ROOT / "infra/nixos/hosts/finite-lat-3/runner.env.example",
    ROOT / "infra/nixos/hosts/finite-lat-4/runner.env.example",
)

# Keys the host configs set through finite.kataRunnerHost.*. Everything else
# in the rendered shared env must be identical across hosts.
PER_HOST_KEYS = {
    "FC_CORE_URL",
    "FC_RUNNER_ID",
    "FC_RUNNER_KATA_HOST_ADDRESS",
    "FC_RUNNER_MAX_SANDBOXES",
    "FC_RUNNER_SOURCE_HOST_ID",
    "FC_RUNNER_WORK_ROOT",
}

# The shared default whose hand-set drift halted a rollout; pinned by the
# shared module and asserted here.
SHARED_STOP_TIMEOUT = "180"
CANONICAL_FINITE_PRIVATE_BASE_URL = (
    "https://finite-private.finite.containers.tinfoil.dev/v1"
)
CANONICAL_FINITE_PRIVATE_MODEL = "glm-5-3-flash"

# The promoted Runtime artifact is an operator-managed pin in runner.env
# (infra/runbooks/runtime-image.md). A rendered default was only ever a stale
# shadow of it, so the shared env must not carry one.
OPERATOR_ONLY_KEYS = {"FC_RUNNER_RUNTIME_ARTIFACT_ID"}
NIX_OWNED_FINITE_PRIVATE_KEYS = {
    "FC_RUNNER_FINITE_PRIVATE_BASE_URL",
    "FC_RUNNER_FINITE_PRIVATE_MODEL",
}

# systemd.services.finite-saas-runner.environment key that is legitimately
# per-host (loopback Authority on the Core host, overlay proxy on a remote
# Runner). It must exist on both hosts but may differ.
PER_HOST_UNIT_ENV_KEYS = {"FINITE_IDENTITY_AUTHORITY"}


def nix_eval(
    host: str, attribute: str, *, raw: bool = False, stringify: bool = False
) -> str:
    config = f".#nixosConfigurations.{host}.config"
    command = ["nix", "eval", "--raw" if raw else "--json"]
    if stringify:
        command += ["--apply", "builtins.map toString"]
    command.append(f"{config}.{attribute}")
    return subprocess.run(
        command,
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def parse_env(text: str) -> dict[str, str]:
    env = {}
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator:
            raise SystemExit(f"unparseable runner-shared.env line: {line!r}")
        env[key] = value
    return env


def check_shared_env(envs: dict[str, dict[str, str]]) -> None:
    reference, *others = HOSTS
    for host in HOSTS:
        if envs[host].get("FC_RUNNER_MAX_SANDBOXES") != EXPECTED_MAX_SANDBOXES[host]:
            raise SystemExit(
                f"{host}: FC_RUNNER_MAX_SANDBOXES is "
                f"{envs[host].get('FC_RUNNER_MAX_SANDBOXES')!r}, expected "
                f"{EXPECTED_MAX_SANDBOXES[host]!r}"
            )
        rendered_operator_keys = sorted(OPERATOR_ONLY_KEYS & set(envs[host]))
        if rendered_operator_keys:
            raise SystemExit(
                f"{host}: {rendered_operator_keys} rendered into "
                f"{SHARED_ENV_PATH}; the Runtime artifact pin lives only in "
                f"{OPERATOR_ENV_PATH}"
            )
        if envs[host].get("FC_RUNNER_KATA_STOP_TIMEOUT_SECS") != SHARED_STOP_TIMEOUT:
            raise SystemExit(
                f"{host}: FC_RUNNER_KATA_STOP_TIMEOUT_SECS is "
                f"{envs[host].get('FC_RUNNER_KATA_STOP_TIMEOUT_SECS')!r}, "
                f"expected the shared {SHARED_STOP_TIMEOUT!r}"
            )
        if (
            envs[host].get("FC_RUNNER_FINITE_PRIVATE_BASE_URL")
            != CANONICAL_FINITE_PRIVATE_BASE_URL
        ):
            raise SystemExit(
                f"{host}: Finite Private base URL is not the live finite-private route"
            )
        if (
            envs[host].get("FC_RUNNER_FINITE_PRIVATE_MODEL")
            != CANONICAL_FINITE_PRIVATE_MODEL
        ):
            raise SystemExit(f"{host}: Finite Private model is not canonical GLM-5.3-Flash")

    for host in others:
        keys_a, keys_b = set(envs[reference]), set(envs[host])
        undeclared_key_drift = (keys_a ^ keys_b) - PER_HOST_KEYS
        if undeclared_key_drift:
            raise SystemExit(
                f"runner env keys drifted outside the per-host set "
                f"{reference} vs {host}: {sorted(undeclared_key_drift)}"
            )

        shared_keys = (keys_a & keys_b) - PER_HOST_KEYS
        drifted = sorted(
            key for key in shared_keys if envs[reference][key] != envs[host][key]
        )
        if drifted:
            raise SystemExit(
                f"shared runner env values drifted {reference} vs {host}: "
                f"{drifted}; fix them in infra/nixos/modules/kata-runner-host.nix"
            )

        per_host_present = (keys_a | keys_b) & PER_HOST_KEYS
        identical = sorted(
            key
            for key in per_host_present
            if envs[reference].get(key) == envs[host].get(key)
        )
        if identical:
            raise SystemExit(
                f"per-host runner env keys no longer differ {reference} vs "
                f"{host}: {identical}; move them to the shared module"
            )


def check_operator_env_templates() -> None:
    for template in OPERATOR_ENV_TEMPLATES:
        keys = set(parse_env(template.read_text(encoding="utf-8")))
        shadowed = sorted(keys & NIX_OWNED_FINITE_PRIVATE_KEYS)
        if shadowed:
            raise SystemExit(
                f"{template.relative_to(ROOT)} shadows Nix-owned Runner keys "
                f"{shadowed}; keep the Finite Private route/model only in "
                "infra/nixos/modules/kata-runner-host.nix"
            )


def check_unit_fragments() -> None:
    # Evaluated leaf attributes; whole-service eval touches options with no
    # defined value. requires/after/wants and unitConfig legitimately differ:
    # the Core host wires the Runner behind finite-identity.service and
    # finite-saas-core.service, and lat3 gates creation on the operator file.
    # The module-owned ordering entries are asserted separately below.
    comparable_attributes = {
        "description": lambda host: nix_eval(
            host, "systemd.services.finite-saas-runner.description", raw=True
        ),
        "path": lambda host: json.loads(
            nix_eval(host, "systemd.services.finite-saas-runner.path", stringify=True)
        ),
        "environment": lambda host: json.loads(
            nix_eval(host, "systemd.services.finite-saas-runner.environment")
        ),
        "serviceConfig": lambda host: json.loads(
            nix_eval(host, "systemd.services.finite-saas-runner.serviceConfig")
        ),
        "timer": lambda host: json.loads(
            nix_eval(host, "systemd.timers.finite-saas-runner.timerConfig")
        ),
        "timerWantedBy": lambda host: json.loads(
            nix_eval(host, "systemd.timers.finite-saas-runner.wantedBy")
        ),
    }

    services = {}
    fragments = {}
    for host in HOSTS:
        service = {
            name: evaluate(host) for name, evaluate in comparable_attributes.items()
        }
        services[host] = service
        fragment = dict(service)
        environment = dict(fragment["environment"])
        for key in PER_HOST_UNIT_ENV_KEYS:
            environment.pop(key, None)
        fragment["environment"] = environment
        service_config = dict(fragment["serviceConfig"])
        service_config.pop("EnvironmentFile", None)
        fragment["serviceConfig"] = service_config
        fragments[host] = fragment

    reference, *others = HOSTS
    for host in others:
        if fragments[reference] != fragments[host]:
            diff = sorted(
                name
                for name in fragments[reference]
                if fragments[reference][name] != fragments[host][name]
            )
            raise SystemExit(
                f"finite-saas-runner module-owned shape drifted {reference} "
                f"vs {host} in {diff}; role changes belong in "
                "infra/nixos/modules/finite-saas-runner.nix"
            )

    for host in HOSTS:
        service = services[host]
        environment_files = service["serviceConfig"].get("EnvironmentFile", [])
        if environment_files[:2] != [SHARED_ENV_PATH, OPERATOR_ENV_PATH]:
            raise SystemExit(
                f"{host}: EnvironmentFile order is {environment_files}; the "
                f"shared {SHARED_ENV_PATH} must load before the "
                f"operator-managed {OPERATOR_ENV_PATH}"
            )
        for key in PER_HOST_UNIT_ENV_KEYS:
            if key not in service["environment"]:
                raise SystemExit(f"{host}: unit environment is missing {key}")
        for attribute in ("requires", "after"):
            entries = json.loads(
                nix_eval(host, f"systemd.services.finite-saas-runner.{attribute}")
            )
            if "containerd.service" not in entries:
                raise SystemExit(
                    f"{host}: lost the module-owned containerd.service {attribute} entry"
                )


def main() -> None:
    check_operator_env_templates()
    envs = {
        host: parse_env(
            nix_eval(host, 'environment.etc."finite/runner-shared.env".text', raw=True)
        )
        for host in HOSTS
    }
    check_shared_env(envs)
    check_unit_fragments()
    print("Kata Runner host contract: ok")


if __name__ == "__main__":
    main()
