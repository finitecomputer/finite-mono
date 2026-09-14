#!/usr/bin/env python3
"""Project local Kata ownership into a credential-free, short-lived route table.

Only this root helper can inspect containerd. The public proxy cannot access the
socket or durable Agent state. No Core requests or overlay addresses are used.
"""

import ipaddress
import json
import os
import re
import subprocess
import tempfile
import time
from pathlib import Path

LABEL = "computer.finite.v2."
NETWORKS = tuple(ipaddress.ip_network(value) for value in ("10.4.0.0/24", "10.89.0.0/16"))


def routes_from_containers(containers, work_root, host_id):
    root = Path(work_root) / "kata"
    candidates = {}
    for container in containers:
        try:
            labels = container["Config"]["Labels"]
            name = container["Name"].removeprefix("/")
            if (
                labels.get(LABEL + "runtime") != "true"
                or labels.get(LABEL + "source_host_id") != host_id
                or labels.get(LABEL + "source_machine_id") != name
                or not labels.get(LABEL + "project_id")
                or labels.get(LABEL + "recovery_request_id")
                or container["State"]["Status"] != "running"
            ):
                continue
            mounts = [m for m in container["Mounts"] if m["Destination"] == "/data"]
            if len(mounts) != 1 or mounts[0].get("RW") is not True:
                continue
            state = Path(mounts[0]["Source"])
            runtime_id = state.name
            if state.parent != root or not re.fullmatch(r"runtime_[0-9a-f]{20}", runtime_id):
                continue
            # Kata/nerdctl may expose guest interfaces as unknown-eth0 rather
            # than the CNI network name. Match only the runner's local subnets
            # and require exactly one address, never the first interface.
            addresses = [
                ipaddress.ip_address(network["IPAddress"])
                for network in container["NetworkSettings"]["Networks"].values()
                if network.get("IPAddress")
            ]
            addresses = [
                address for address in addresses if any(address in network for network in NETWORKS)
            ]
            if len(addresses) != 1:
                continue
            address = addresses[0]
            # This file contains public coordinates only, never the secret key.
            with (state / "agent/config.json").open() as config_file:
                account_id = json.loads(config_file.read(8192))["account_id"]
            if not re.fullmatch(r"[0-9a-f]{64}", account_id):
                continue
            candidates.setdefault(runtime_id, []).append(
                {
                    "address": str(address),
                    "account_id": account_id,
                }
            )
        except (KeyError, TypeError, ValueError, OSError):
            continue
    # Ambiguous ownership never grants authority to select a container.
    return {runtime: matches[0] for runtime, matches in candidates.items() if len(matches) == 1}


def refresh(nerdctl, work_root, host_id, destination):
    def run(*args):
        return subprocess.run(
            [nerdctl, "--namespace", "finite", *args],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout

    ids = run("ps", "--filter", "label=" + LABEL + "runtime=true", "--format", "{{.ID}}").split()
    containers = json.loads(run("inspect", *ids)) if ids else []
    routes = routes_from_containers(containers, work_root, host_id)
    destination = Path(destination)
    with tempfile.NamedTemporaryFile(mode="w", dir=destination.parent, delete=False) as output:
        try:
            os.fchmod(output.fileno(), 0o644)
            json.dump({"generated_at": time.time(), "routes": routes}, output)
            output.flush()
            os.fsync(output.fileno())
            os.replace(output.name, destination)
        finally:
            Path(output.name).unlink(missing_ok=True)


if __name__ == "__main__":
    while True:
        try:
            refresh(
                os.environ["NERDCTL"],
                os.environ["FINITE_RUNNER_WORK_ROOT"],
                os.environ["FINITE_SOURCE_HOST_ID"],
                os.environ["FINITE_GATEWAY_ROUTES"],
            )
        except (OSError, ValueError, subprocess.SubprocessError):
            # Existing projection expires; never log inspect output or credentials.
            print("Gateway inventory refresh failed", flush=True)
        time.sleep(2)
