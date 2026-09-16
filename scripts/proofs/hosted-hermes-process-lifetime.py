#!/usr/bin/env python3
"""Destructive only inside a disposable Linux/systemd fixture; never run on a fleet host.

Proves the proposed Runner cgroup boundary with real systemd and containerd.
Does not qualify Kata, production DNS/TLS, or the still-unwired routing lifecycle.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import shlex
import signal
import subprocess
import tempfile
import time


def run(*args, check=True, timeout=60):
    result = subprocess.run([str(arg) for arg in args], text=True,
                            capture_output=True, timeout=timeout)
    if check and result.returncode:
        raise RuntimeError(f"fixture command failed: {args[0]}\n{result.stderr}")
    return result


def eventually(check, timeout=15):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(0.05)
    raise AssertionError("fixture condition did not converge")


def properties(unit):
    result = run("systemctl", "show", unit, "--property=ActiveState,SubState,MainPID,ControlGroup,InvocationID")
    return dict(line.split("=", 1) for line in result.stdout.splitlines())


def alive(pid):
    try:
        # Zombies can no longer mutate anything and may await PID1 reaping.
        return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0] != "Z"
    except FileNotFoundError:
        return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--disposable-fixture", action="store_true", required=True)
    args = parser.parse_args()
    assert os.geteuid() == 0 and Path("/sys/fs/cgroup/cgroup.controllers").is_file()
    tools = args.tools.resolve()
    os.environ["PATH"] = f"{tools / 'bin'}:{os.environ['PATH']}"
    scratch = Path(tempfile.mkdtemp(prefix="fin91-process-", dir="/run"))
    units = []
    loose_pids = []
    nerdctl = None
    evidence = {}

    def service(name, command, *, fenced=True):
        unit = f"{scratch.name}-{name}.service"
        units.append(unit)
        run("systemd-run", f"--unit={unit}", "--property=Type=exec",
            f"--property=ExitType={'cgroup' if fenced else 'main'}",
            f"--property=KillMode={'control-group' if fenced else 'process'}",
            "--property=TimeoutStopSec=5s", f"--setenv=PATH={os.environ['PATH']}",
            *command)
        return unit

    try:
        # Negative control: the old main-process lifetime lets delayed work
        # survive an inactive unit. Use bounded sleep, not any real mutation.
        child_file = scratch / "old-child"
        old = service("old", ["/bin/sh", "-c",
                      f"sleep 60 & echo $! > {child_file}; exit 0"], fenced=False)
        pid = int(eventually(lambda: child_file.read_text() if child_file.exists() else None))
        loose_pids.append(pid)
        eventually(lambda: properties(old)["ActiveState"] == "inactive")
        assert alive(pid), "negative control did not leave the delayed child alive"
        os.kill(pid, signal.SIGKILL)
        eventually(lambda: not alive(pid))
        loose_pids.remove(pid)
        evidence["old_boundary_leaves_orphan"] = True

        # With cgroup lifetime the next timer cycle cannot begin while a child
        # still exists, even after the main process successfully exits.
        child_file = scratch / "new-child"
        unit = service("new", ["/bin/sh", "-c",
                       f"sleep 60 & echo $! > {child_file}; exit 0"])
        pid = int(eventually(lambda: child_file.read_text() if child_file.exists() else None))
        state = eventually(lambda: properties(unit) if properties(unit)["MainPID"] == "0" else None)
        assert alive(pid) and state["ActiveState"] == "active"
        invocation = state["InvocationID"]
        run("systemctl", "start", unit)
        assert properties(unit)["InvocationID"] == invocation
        run("systemctl", "stop", unit)
        eventually(lambda: not alive(pid))
        evidence["child_blocks_next_invocation_and_stop_reaps_it"] = True

        # SIGKILL can bypass Rust Drop; systemd still owns the descendants.
        child_file = scratch / "killed-child"
        unit = service("killed", ["/bin/sh", "-c",
                       f"sleep 60 & echo $! > {child_file}; exec sleep 60"])
        pid = int(eventually(lambda: child_file.read_text() if child_file.exists() else None))
        state = properties(unit)
        os.kill(int(state["MainPID"]), signal.SIGKILL)
        eventually(lambda: properties(unit)["MainPID"] == "0")
        assert not alive(pid) or properties(unit)["ActiveState"] in {"active", "deactivating"}
        run("systemctl", "stop", unit)
        eventually(lambda: not alive(pid))
        evidence["sigkill_cannot_leave_an_untracked_child"] = True

        # Separate containerd service owns the compute. Stopping a Runner unit
        # must not kill the agent process it launched through containerd.
        socket = scratch / "containerd.sock"
        service("containerd", [tools / "bin/containerd", "--address", socket,
                         "--root", scratch / "root", "--state", scratch / "state"])
        eventually(socket.exists)
        nerdctl = [tools / "bin/nerdctl", "--address", socket,
                   "--namespace", "fin91-proof", "--data-root", scratch / "nerdctl",
                   "--cni-path", tools / "lib/cni", "--cni-netconfpath", scratch / "cni"]
        eventually(lambda: run(*nerdctl, "images", "--quiet", check=False).returncode == 0)
        run(*nerdctl, "load", "-i", tools / "share/proof-image.tar.gz", timeout=120)
        network = scratch.name
        run(*nerdctl, "network", "create", "--subnet", f"10.237.{os.getpid() % 250}.0/24", network)
        launch = scratch / "launch.sh"
        launch.write_text("#!/bin/sh\nset -eu\n" + shlex.join(str(arg) for arg in nerdctl) +
                          f" run -d --name agent --network {network} --restart unless-stopped -p 127.0.0.1:30001:8642 "
                          "finite-hosted-lifecycle-proof:fixture\nexec sleep 60\n")
        launch.chmod(0o700)
        unit = service("runner", ["/bin/sh", launch])

        def inspect():
            if properties(unit)["ActiveState"] == "failed":
                raise RuntimeError(run("journalctl", "-u", unit, "--no-pager", "-n", "15").stdout)
            result = run(*nerdctl, "inspect", "agent", check=False)
            return json.loads(result.stdout)[0] if result.returncode == 0 else None

        inspected = eventually(lambda: (record if (record := inspect()) and record["State"]["Status"] == "running" else None), timeout=60)
        agent_pid = inspected["State"]["Pid"]
        assert alive(agent_pid)
        runner_cgroup = properties(unit)["ControlGroup"]
        runner_processes = Path("/sys/fs/cgroup" + runner_cgroup + "/cgroup.procs").read_text().splitlines()
        evidence["runner_processes_after_launch"] = runner_processes
        assert runner_processes == [properties(unit)["MainPID"]], "provider left persistent children in Runner's cgroup"
        assert runner_cgroup not in Path(f"/proc/{agent_pid}/cgroup").read_text()
        run("systemctl", "stop", unit)
        assert inspect()["State"]["Status"] == "running" and alive(agent_pid)
        evidence["stopping_runner_preserves_containerd_agent"] = True
        # Save only non-secret metadata from this synthetic container.
        evidence["running_bindings"] = inspect().get("HostConfig", {}).get("PortBindings")
        evidence["running_saved_ports"] = run(*nerdctl, "port", "agent").stdout
        run(*nerdctl, "stop", "--time", "1", "agent")
        evidence["stopped_bindings"] = inspect().get("HostConfig", {}).get("PortBindings")
        evidence["stopped_saved_ports"] = run(*nerdctl, "port", "agent").stdout
        assert evidence["stopped_bindings"] == {}, "update the inspect-shape negative control"
        assert evidence["stopped_saved_ports"] == evidence["running_saved_ports"]
        assert "8642/tcp -> 127.0.0.1:30001" in evidence["stopped_saved_ports"]
        run(*nerdctl, "start", "agent")
        assert run(*nerdctl, "port", "agent").stdout == evidence["running_saved_ports"]
        assert inspect()["State"]["Status"] == "running"
        evidence["saved_reservation_survives_stop_and_restart"] = True
        evidence["systemd"] = run("systemctl", "--version").stdout.splitlines()[0]
        evidence["containerd"] = run(tools / "bin/containerd", "--version").stdout.strip()
        evidence["nerdctl"] = run(tools / "bin/nerdctl", "--version").stdout.strip()
        evidence["architecture"] = os.uname().machine
        print(json.dumps(evidence, indent=2))
    finally:
        if nerdctl is not None:
            run(*nerdctl, "rm", "--force", "agent", check=False)
            run(*nerdctl, "network", "rm", scratch.name, check=False)
            run(*nerdctl, "network", "rm", "bridge", check=False)
        for unit in reversed(units):
            run("systemctl", "stop", unit, check=False)
            run("systemctl", "reset-failed", unit, check=False)
        for pid in loose_pids:
            if alive(pid):
                os.kill(pid, signal.SIGKILL)
        shutil.rmtree(scratch)


if __name__ == "__main__":
    main()
