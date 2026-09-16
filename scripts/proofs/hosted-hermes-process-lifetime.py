#!/usr/bin/env python3
"""Destructive only inside a disposable Linux/systemd fixture; never run on a fleet host.

Proves the actual Runner lifecycle with real systemd, Caddy and containerd.
Does not qualify x86 Kata, public DNS/TLS, or the full Core lease/guest protocols.
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
import http.server
import threading
import ssl
import urllib.request
import urllib.error
import uuid


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


def prove_runner(binary, tools, scratch, nerdctl, network):
    runner = "finite-saas-runner.service"
    proxy = "finite-hosted-hermes.service"
    state = Path("/run/finite-hosted-hermes")
    unit_root = Path("/run/systemd/system")
    for unit in (runner, proxy):
        assert run("systemctl", "show", unit, "--property=LoadState", "--value").stdout.strip() == "not-found", "fixture must not have real Runner/Caddy units"
    assert not state.exists(), "fixture must not have real hosted state"
    state.mkdir(mode=0o700)
    work = scratch / "work"
    work.mkdir()
    targets = []
    token = uuid.uuid4().hex
    evidence = {}

    class Core(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            assert self.path == "/api/core/v1/hosted-hermes-route-targets"
            assert self.headers["Authorization"] == "Bearer " + token
            body = json.dumps(targets).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        def log_message(self, *_args):
            pass

    core = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Core)
    threading.Thread(target=core.serve_forever, daemon=True).start()
    # Use the same provider, CNI and containerd, but the production namespace.
    native = list(nerdctl)
    ns_index = native.index("--namespace")
    del native[ns_index:ns_index + 2]
    command = shlex.join(str(arg) for arg in native)
    wrapper = scratch / "nerdctl-proof"
    # For each remove, record the *old* proxy PID captured by the harness.
    # It must have exited even if a successor is already serving other agents.
    wrapper.write_text("#!/bin/sh\nset -eu\n" + f"""
if test "$3" = rm; then
  test -f {state}/mutation-in-progress
  if test -f {work}/old-proxy; then
    old=$(cat {work}/old-proxy)
    if test "$old" != 0 && kill -0 "$old" 2>/dev/null; then exit 91; fi
  fi
  touch {work}/remove-was-fenced
  if test -f {work}/delay-remove; then
    touch {work}/at-remove
    sleep 3
  fi
fi
exec {command} "$@"
""")
    wrapper.chmod(0o700)
    job_file = scratch / "job.json"
    gate = Path(__file__).resolve().parents[2] / "infra/nixos/modules/hosted-hermes-runner-gate.sh"
    common = f"Environment=PATH={os.environ['PATH']}\n"
    (unit_root / proxy).write_text(f"""[Service]
Type=notify
ExecStart={tools}/bin/caddy run --config {state}/caddy.json
Restart=no
KillMode=control-group
SendSIGKILL=yes
TimeoutStartSec=10s
TimeoutStopSec=5s
Environment=HOME={scratch}/caddy
Environment=XDG_DATA_HOME={scratch}/caddy/data
Environment=XDG_CONFIG_HOME={scratch}/caddy/config
{common}
""")
    runner_text = f"""[Service]
Type=exec
ExitType=cgroup
KillMode=control-group
SendSIGKILL=yes
Delegate=no
TimeoutStopSec=5s
RuntimeMaxSec=120s
ExecStartPre=/bin/sh {gate} {binary}
ExecStart={binary} --ignored --exact kata::hosted_hermes_proof::systemd_lifecycle --nocapture --test-threads=1
Environment=FIN91_PROOF_JOB={job_file}
{common}
"""
    (unit_root / runner).write_text(runner_text)
    run("systemctl", "daemon-reload")

    def target(name):
        return dict(runtimeId=name, projectId="proof-project", sourceMachineId=name, generation=1)

    def start_job(action, name="agent-a", runtime=None):
        job_file.write_text(json.dumps(dict(action=action, name=name, runtime=runtime or name,
            root=str(work), nerdctl=str(wrapper), core_url=f"http://127.0.0.1:{core.server_port}",
            core_token=token, network=network)))
        job_file.chmod(0o600)
        run("systemctl", "reset-failed", runner, check=False)
        run("systemctl", "start", runner)

    def finish_job():
        def finished():
            status = properties(runner)["ActiveState"]
            if status == "failed":
                raise AssertionError(run("journalctl", "-u", runner, "--no-pager", "-n", "35").stdout)
            return status == "inactive"
        eventually(finished, timeout=100)

    def job(action, name="agent-a", runtime=None):
        start_job(action, name, runtime)
        finish_job()

    def request(name):
        try:
            with urllib.request.urlopen(f"https://localhost:34443/runtimes/{name}/api/status",
                    context=ssl._create_unverified_context(), timeout=1) as response:
                return response.status, response.read().decode()
        except urllib.error.HTTPError as error:
            return error.code, ""
        except (urllib.error.URLError, TimeoutError):
            return 0, ""

    def port(name):
        return run(*native, "--namespace", "finite", "port", name).stdout.strip()

    def inspect(name):
        return json.loads(run(*native, "--namespace", "finite", "inspect", name).stdout)[0]

    def old_proxy():
        (work / "old-proxy").write_text(properties(proxy)["MainPID"])

    try:
        # Images are namespace-scoped; saved reservations are host-wide.
        run(*native, "--namespace", "finite", "load", "-i", tools / "share/proof-image.tar.gz", timeout=120)
        network += "-finite"
        run(*native, "--namespace", "finite", "network", "create", "--subnet", f"10.238.{os.getpid() % 250}.0/24", network)
        run(*native, "--namespace", "k8s.io", "namespace", "create", "k8s.io", check=False)
        targets[:] = [target("agent-a"), target("agent-b")]
        job("create")
        eventually(lambda: request("agent-a") == (200, "agent-a"))
        assert port("agent-a").endswith(":30000")
        pid = properties(proxy)["MainPID"]
        job("reconcile")
        assert properties(proxy)["MainPID"] == pid, "no-op ticks must not interrupt connections"
        job("binding")
        job("stop")
        assert port("agent-a").endswith(":30000")
        job("create", "agent-b")
        assert port("agent-b").endswith(":30001")
        job("start")
        eventually(lambda: request("agent-a") == (200, "agent-a"))
        evidence["stopped_reservation_and_restart"] = True

        # Removal must kill the old proxy before actual provider release. Other
        # agents are served by the successor even while the provider is delayed.
        job("stop")
        old_proxy()
        (work / "delay-remove").touch()
        start_job("remove")
        eventually(lambda: (work / "at-remove").exists())
        assert request("agent-b") == (200, "agent-b")
        assert request("agent-a")[0] == 404
        finish_job()
        (work / "delay-remove").unlink()
        (work / "at-remove").unlink()
        assert (work / "remove-was-fenced").exists()
        targets[:] = [target("agent-b"), target("agent-c")]
        job("create", "agent-c")
        assert port("agent-c").endswith(":30000")
        assert request("agent-a")[0] == 404
        assert request("agent-c") == (200, "agent-c")
        evidence["exit_before_reuse_and_other_agent_survives_slow_remove"] = True

        # Automatic containerd restart retains identity, saved address and data.
        previous = inspect("agent-b")["State"]["Pid"]
        os.kill(previous, signal.SIGKILL)
        eventually(lambda: inspect("agent-b")["State"]["Pid"] not in (0, previous), timeout=30)
        job("reconcile")
        assert port("agent-b").endswith(":30001")
        eventually(lambda: request("agent-b") == (200, "agent-b"))
        evidence["automatic_restart_preserves_binding_and_data"] = True

        # Applied disable is represented by Core removing all eligible targets.
        targets[:] = []
        job("reconcile")
        assert properties(proxy)["MainPID"] == "0"
        assert inspect("agent-b")["State"]["Status"] == "running"
        assert (work / "kata/agent-b/api/status").read_text() == "agent-b"
        targets[:] = [target("agent-b"), target("agent-c")]
        job("reconcile")
        evidence["disable_terminates_proxy_without_compute_or_data_loss"] = True

        # Failed command fences the rest of its invocation. A fresh quiescent
        # invocation recovers by killing the proxy and rebuilding authority.
        job("failed-mutation", "agent-b")
        assert (state / "mutation-in-progress").exists()
        job("reconcile")
        assert not (state / "mutation-in-progress").exists()
        evidence["failed_operation_recovery"] = True

        job("stop", "agent-c")
        old_proxy()
        (work / "delay-remove").touch()
        start_job("interrupted-remove", "agent-c")
        eventually(lambda: (work / "at-remove").exists())
        before = eventually(lambda: (p if (p := properties(runner))["MainPID"] == "0" else None))
        assert before["ActiveState"] == "active"
        assert (state / "mutation-in-progress").exists()
        run("systemctl", "start", runner)
        assert properties(runner)["InvocationID"] == before["InvocationID"]
        finish_job()
        (work / "delay-remove").unlink()
        targets[:] = [target("agent-b"), target("agent-d")]
        job("create", "agent-d")
        assert port("agent-d").endswith(":30000")
        assert request("agent-c")[0] == 404
        assert request("agent-d") == (200, "agent-d")
        evidence["interrupted_remove_blocks_concurrent_invocation_then_recovers"] = True

        # Run the exact deployment gate with a different, old Runner binary.
        # It has no lifecycle support: the gate must stop ingress *before* it.
        old = scratch / "old-runner"
        old.write_text("#!/bin/sh\ntest \"$(systemctl show finite-hosted-hermes.service --property=MainPID --value)\" = 0\n")
        old.chmod(0o700)
        rollback = runner_text.replace(str(binary), str(old))
        # Remove Rust test arguments from the old binary's ExecStart line.
        rollback = "\n".join(f"ExecStart={old}" if line.startswith("ExecStart=") else line for line in rollback.splitlines()) + "\n"
        (unit_root / runner).write_text(rollback)
        run("systemctl", "daemon-reload")
        run("systemctl", "start", runner)
        finish_job()
        assert properties(proxy)["MainPID"] == "0"
        assert inspect("agent-b")["State"]["Status"] == "running"
        (unit_root / runner).write_text(runner_text)
        run("systemctl", "daemon-reload")
        job("reconcile")
        assert request("agent-b") == (200, "agent-b")
        evidence["binary_rollback_stops_ingress_and_upgrade_reprojects"] = True

        # Upgrade/recovery share these actual adapter methods: keep the old
        # stopped reservation while preparing an unpublished helper, remove the
        # old canonical record, then promote only the correctly owned helper.
        job("stop", "agent-b")
        job("create", "agent-b-candidate", "agent-b")
        assert port("agent-b").endswith(":30001")
        assert port("agent-b-candidate").endswith(":30002")
        assert request("agent-b")[0] == 404
        assert request("agent-b-candidate")[0] == 404
        old_proxy()
        job("remove", "agent-b")
        job("rename", "agent-b-candidate", "agent-b")
        assert request("agent-b") == (200, "agent-b")
        assert port("agent-b").endswith(":30002")
        assert (work / "kata/agent-b/api/status").read_text() == "agent-b"
        evidence["replacement_helper_is_unpublished_until_canonical_promotion"] = True
        return evidence
    finally:
        core.shutdown()
        core.server_close()
        for unit in (runner, proxy):
            run("systemctl", "stop", unit, check=False)
            run("systemctl", "reset-failed", unit, check=False)
            (unit_root / unit).unlink(missing_ok=True)
        run("systemctl", "daemon-reload")
        ids = run(*native, "--namespace", "finite", "ps", "--all", "--quiet", check=False).stdout.split()
        if ids:
            run(*native, "--namespace", "finite", "rm", "--force", *ids, check=False)
        run(*native, "--namespace", "finite", "network", "rm", network, check=False)
        run(*native, "--namespace", "finite", "network", "rm", "bridge", check=False)
        shutil.rmtree(state)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--runner-test-binary", type=Path, required=True)
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
        run(*nerdctl, "rm", "--force", "agent")
        evidence["runner_lifecycle"] = prove_runner(args.runner_test_binary.resolve(), tools, scratch, nerdctl, network)
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
