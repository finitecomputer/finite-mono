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
HOSTS = ["finite-lat-1", "finite-lat-3", "finite-lat-4", "finite-lat-5"]

EXPECTED_MAX_SANDBOXES = {
    "finite-lat-1": "12",
    "finite-lat-3": "42",
    # finite-lat-4 mirrors lat3's owner-authorized ceiling; it is admitted
    # drained (FC_RUNNER_DRAIN is operator env, not shared env).
    "finite-lat-4": "42",
    "finite-lat-5": "42",
}

SHARED_ENV_PATH = "/etc/finite/runner-shared.env"
OPERATOR_ENV_PATH = "/etc/finite/runner.env"
OPERATOR_ENV_TEMPLATES = (
    ROOT / "infra/nixos/hosts/finite-lat-3/runner.env.example",
    ROOT / "infra/nixos/hosts/finite-lat-4/runner.env.example",
    ROOT / "infra/nixos/hosts/finite-lat-5/runner.env.example",
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
    host: str,
    attribute: str,
    *,
    raw: bool = False,
    stringify: bool = False,
    base_runner: bool = False,
) -> str:
    config = f".#nixosConfigurations.{host}.config"
    command = ["nix", "eval", "--raw" if raw else "--json"]
    if base_runner:
        # Compare the common Runner role with its optional ingress disabled.
        # The real enabled host is checked separately below, without overrides.
        value = f"(host.extendModules {{ modules = [ ({{ lib, ... }}: {{ finite.hostedHermes.enable = lib.mkForce false; }}) ]; }}).config.{attribute}"
        if stringify:
            value = f"builtins.map toString ({value})"
        command += ["--apply", f"host: {value}", f".#nixosConfigurations.{host}"]
    else:
        if stringify:
            command += ["--apply", "builtins.map toString"]
        command.append(f"{config}.{attribute}")
    return subprocess.run(
        command,
        cwd=ROOT,
        check=True,
        stdout=subprocess.PIPE,
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
        runtime_env = json.loads(envs[host]["FC_RUNNER_RUNTIME_ENV_JSON"])
        if runtime_env.get("FINITE_SITES_API") != "https://finite.site":
            raise SystemExit(
                f"{host}: Runtime fallback must use the qualified Finite Sites origin"
            )
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
            raise SystemExit(
                f"{host}: Finite Private model is not canonical GLM-5.3-Flash"
            )

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
            host,
            "systemd.services.finite-saas-runner.description",
            raw=True,
            base_runner=True,
        ),
        "path": lambda host: json.loads(
            nix_eval(
                host,
                "systemd.services.finite-saas-runner.path",
                stringify=True,
                base_runner=True,
            )
        ),
        "environment": lambda host: json.loads(
            nix_eval(
                host,
                "systemd.services.finite-saas-runner.environment",
                base_runner=True,
            )
        ),
        "serviceConfig": lambda host: json.loads(
            nix_eval(
                host,
                "systemd.services.finite-saas-runner.serviceConfig",
                base_runner=True,
            )
        ),
        "timer": lambda host: json.loads(
            nix_eval(
                host, "systemd.timers.finite-saas-runner.timerConfig", base_runner=True
            )
        ),
        "timerWantedBy": lambda host: json.loads(
            nix_eval(
                host, "systemd.timers.finite-saas-runner.wantedBy", base_runner=True
            )
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


def check_hosted_ingress() -> None:
    expected_origins = {
        f"finite-lat-{number}": f"https://agents-lat{number}.finite.computer"
        for number in (3, 4, 5)
    }
    for host in HOSTS:
        enabled = json.loads(nix_eval(host, "finite.hostedHermes.enable"))
        if enabled != (host in expected_origins):
            raise SystemExit(f"{host}: unexpected hosted Hermes capability")
    core = json.loads(
        nix_eval("finite-lat-2", "systemd.services.finite-saas-core.environment")
    )
    if core.get("FC_CORE_RUNTIME_BIND") != "127.0.0.1:4201":
        raise SystemExit("Core's runtime router must use its own loopback listener")
    origins = json.loads(core["FC_CORE_HOSTED_HERMES_ORIGINS_JSON"])
    if origins != expected_origins:
        raise SystemExit("Core must publish exactly the configured Runner hosts")
    edge = nix_eval(
        "finite-lat-2",
        'services.caddy.virtualHosts."runtime-api.finite.computer".extraConfig',
        raw=True,
    )
    if edge.strip() != "reverse_proxy 127.0.0.1:4201":
        raise SystemExit(
            "The runtime edge must proxy only the dedicated router verbatim"
        )
    for host in expected_origins:
        check_hosted_ingress_host(host, origins)


def check_hosted_ingress_host(host: str, origins: dict[str, str]) -> None:
    environment = json.loads(
        nix_eval(host, "systemd.services.finite-saas-runner.environment")
    )
    service = json.loads(
        nix_eval(host, "systemd.services.finite-saas-runner.serviceConfig")
    )
    expected = {
        "Type": "exec",
        "ExitType": "cgroup",
        "KillMode": "control-group",
        "SendSIGKILL": True,
        "Delegate": False,
        "TimeoutStopSec": "5s",
        "RuntimeMaxSec": "1h",
    }
    for key, value in expected.items():
        if service.get(key) != value:
            raise SystemExit(f"{host} hosted ingress lost its {key} lifetime boundary")
    if not any(
        "hosted-hermes-runner-gate" in entry
        for entry in service.get("ExecStartPre", [])
    ):
        raise SystemExit(f"{host} hosted ingress lost its Runner rollback gate")
    if not environment.get("FC_RUNNER_HOSTED_HERMES_CONFIG"):
        raise SystemExit(f"{host} is missing its hosted ingress manifest")
    proxy = json.loads(
        nix_eval(host, "systemd.services.finite-hosted-hermes.serviceConfig")
    )
    proxy_expected = {
        "Type": "notify",
        "Restart": "no",
        "KillMode": "control-group",
        "SendSIGKILL": True,
        "TimeoutStartSec": "10s",
        "TimeoutStopSec": "5s",
        "User": "root",
        "Group": "root",
        "UMask": "0077",
        "StateDirectory": "finite-hosted-hermes",
        "Environment": [
            "HOME=/var/lib/finite-hosted-hermes",
            "XDG_DATA_HOME=/var/lib/finite-hosted-hermes/data",
            "XDG_CONFIG_HOME=/var/lib/finite-hosted-hermes/config",
        ],
    }
    for key, value in proxy_expected.items():
        if proxy.get(key) != value:
            raise SystemExit(f"{host} dedicated proxy lost its {key} boundary")
    caddy = nix_eval(host, "services.caddy.package.outPath", raw=True)
    if proxy.get("ExecStart") != (
        f"{caddy}/bin/caddy run --config /run/finite-hosted-hermes/caddy.json"
    ):
        raise SystemExit(f"{host} proxy must start from the fresh Runner projection")
    if proxy.get("ExecReload"):
        raise SystemExit(f"{host} proxy must stop before address reuse, never reload")
    for attribute in ("wantedBy", "requiredBy", "upheldBy"):
        if json.loads(
            nix_eval(host, f"systemd.services.finite-hosted-hermes.{attribute}")
        ):
            raise SystemExit(
                f"{host} proxy must not have automatic {attribute} activation"
            )
    # Project only the activation target; complete NixOS socket submodules
    # contain internal values that cannot be serialized to JSON.
    sockets = json.loads(
        subprocess.check_output(
            [
                "nix",
                "eval",
                "--json",
                "--apply",
                'builtins.mapAttrs (_: socket: socket.socketConfig.Service or "")',
                f".#nixosConfigurations.{host}.config.systemd.sockets",
            ],
            cwd=ROOT,
            text=True,
        )
    )
    if any(
        name == "finite-hosted-hermes" or target == "finite-hosted-hermes.service"
        for name, target in sockets.items()
    ):
        raise SystemExit(f"{host} proxy must not use socket activation")
    if (
        environment.get("FC_RUNNER_RUNTIME_CORE_URL")
        != "https://runtime-api.finite.computer"
    ):
        raise SystemExit(f"{host} bootstrap must use the dedicated runtime HTTPS origin")
    public_origin = nix_eval(host, "finite.hostedHermes.publicOrigin", raw=True)
    if public_origin != origins[host]:
        raise SystemExit(f"Core and {host} disagree on the native Hermes origin")
    allowed = json.loads(nix_eval(host, "finite.hostedHermes.allowedOrigins"))
    if allowed != ["https://finite.computer"]:
        raise SystemExit(f"{host} hosted CORS must match the production dashboard exactly")
    if 443 not in json.loads(nix_eval(host, "networking.firewall.allowedTCPPorts")):
        raise SystemExit(f"{host} hosted TLS port is not open")


def main() -> None:
    check_operator_env_templates()
    envs = {
        host: parse_env(
            nix_eval(host, 'environment.etc."finite/runner-shared.env".text', raw=True)
        )
        for host in HOSTS
    }
    check_shared_env(envs)
    core = json.loads(
        nix_eval("finite-lat-2", "systemd.services.finite-saas-core.environment")
    )
    if (
        json.loads(core["FC_CORE_RUNTIME_ENV_JSON"]).get("FINITE_SITES_API")
        != "https://finite.site"
    ):
        raise SystemExit("Core RuntimeSpecs must use the qualified Finite Sites origin")
    phala = json.loads(
        nix_eval(
            "finite-lat-1", "systemd.services.finite-saas-runner-phala.environment"
        )
    )
    if "FC_RUNNER_RUNTIME_ARTIFACT_ID" in phala:
        raise SystemExit(
            "Phala Runtime promotion pin must live in phala-runner.env, not Nix"
        )
    if (
        json.loads(phala["FC_RUNNER_RUNTIME_ENV_JSON"]).get("FINITE_SITES_API")
        != "https://finite.site"
    ):
        raise SystemExit(
            "Phala Runtime fallback must use the qualified Finite Sites origin"
        )
    dashboard = json.loads(
        nix_eval(
            "finite-lat-2",
            "virtualisation.oci-containers.containers.finite-saas-dashboard.environment",
        )
    )
    if dashboard.get("FC_SITES_UPSTREAM_URL") != "https://finite.site":
        raise SystemExit(
            "Dashboard viewing and publishing must use the Finite Sites registry"
        )
    services = json.loads(
        subprocess.run(
            [
                "nix",
                "eval",
                "--json",
                "--apply",
                "builtins.attrNames",
                ".#nixosConfigurations.finite-lat-2.config.systemd.services",
            ],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    if "finite-saas-sites" in services:
        raise SystemExit("The app host must not run a second Sites daemon")
    check_unit_fragments()
    check_hosted_ingress()
    print("Kata Runner host contract: ok")


if __name__ == "__main__":
    main()
