#!/usr/bin/env python3
"""Run the real closure deploy scripts against synthetic host state.

The ssh shim runs each remote body locally (bash for the activation body,
python3 for the configuration comparison helper). Host commands (systemctl,
nix-store, nix-env, readlink, sleep) are shims that record their calls and
model the systemd and profile state the scripts read. No network, host
service, system profile or real Nix store is touched.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
COMPARE = ROOT / "infra/nixos/scripts/compare-activation-config"
REV = "a" * 40
RUNNER_STORE = "/nix/store/" + "r" * 32 + "-finite-saas-runner-0.1.0"
DASHBOARD_PIN = (ROOT / "infra/nixos/modules/dashboard.nix").read_text(encoding="utf-8")

# Tonight's dry activation on lat3, lat4 and lat5: a package change touches
# system-path and the tmpfiles configuration, so both units are named.
REFERENCE_DRY_OUTPUT = (
    "would stop the following units: systemd-tmpfiles-resetup.service\n"
    "would activate the configuration...\n"
    "would reload the following units: dbus-broker.service\n"
    "would start the following units: systemd-tmpfiles-resetup.service\n"
)


def write_shim(bin_dir: Path, name: str, body: str) -> None:
    path = bin_dir / name
    path.write_text("#!/usr/bin/env bash\n" + body, encoding="utf-8")
    path.chmod(0o755)


def manifest(host: str) -> dict:
    number = host[-1]
    payload = {
        "schema": f"finite.{host}.nixos-closure.v{1 if host == 'lat3' else 2}",
        "host": f"finite-lat-{number}",
        "repository": "finitecomputer/finite-mono",
        "rev": REV,
        "system": "/nix/store/" + "b" * 32 + f"-nixos-system-finite-lat-{number}-26.05.test",
        "cache": "nix-cache",
    }
    if host != "lat3":
        payload["disko"] = "/nix/store/" + "c" * 32 + "-disko"
        payload["kexec"] = "/nix/store/" + "d" * 32 + "-kexec-tarball"
    return payload


SSH_SHIM = """import os, subprocess, sys
args = sys.argv[1:]
if any("dry-activate" in arg for arg in args):
    sys.stdout.write(open(os.environ["TEST_DRY_OUTPUT"]).read())
    sys.exit(0)
body = sys.stdin.read()
store = os.environ["TEST_STORE_ROOT"]
remote_args = args[args.index("-") + 1:] if "-" in args else args[args.index("--") + 1:] if "--" in args else []
remote_args = [arg.replace("/nix/store/", store + "/") for arg in remote_args]
if "python3" in args:
    if os.environ.get("TEST_COMPARE_EXIT"):
        print("python3: command not found", file=sys.stderr)
        sys.exit(int(os.environ["TEST_COMPARE_EXIT"]))
    remote_args = ["--current", os.environ["TEST_CURRENT"], "--store-dir", store, *remote_args]
    sys.exit(subprocess.run([sys.executable, "-", *remote_args], input=body, text=True).returncode)
if "previous_system=" not in body:
    sys.exit(0)  # the host-local monitoring secrets preflight
body = body.replace("/nix/store/", store + "/")
body = body.replace("/run/finite-deploy-runner-pause", os.environ["TEST_PAUSE_MARKER"])
body = body.replace("/run/systemd/system", os.environ["TEST_RUNTIME_UNITS"])
if os.environ.get("TEST_SHORT_DEADLINE") == "1":
    body = body.replace("SECONDS + 600", "SECONDS + 1")
sys.exit(subprocess.run(["bash", "-s", "--", *remote_args], input=body, text=True).returncode)
"""

# Runner model. Before the switch, ActiveState follows the sequence in
# $STATE/runner. After it, TEST_RUNNER_AFTER_SWITCH (if set) holds the state;
# otherwise each invocation after the first finishes as TEST_CYCLE says
# (success, fail or none), after TEST_CYCLE_ACTIVE_READS reads of "active".
# The paused timer never starts. TEST_TIMER_QUERY_FAIL (always, after-switch
# or after-rollback) makes timer queries fail; TEST_TIMER_STOP (fail-after-switch,
# or noop-before-switch) makes stopping the timer fail or do nothing.
SYSTEMCTL_SHIM = r"""
echo "systemctl $*" >> "$CALL_LOG"
unit="${!#}"
invocations() { cat "$STATE/invocations"; }
timer_query_fails() {
  case "${TEST_TIMER_QUERY_FAIL:-}" in
    always) return 0 ;;
    after-switch) [[ -e "$STATE/switched" ]] ;;
    after-rollback) [[ -e "$STATE/rollback-started" ]] ;;
    *) return 1 ;;
  esac
}
runner_state() {
  local state reads
  if [[ -e "$STATE/switched" ]]; then
    reads="$(cat "$STATE/cycle-active-reads" 2>/dev/null || echo "${TEST_CYCLE_ACTIVE_READS:-0}")"
    if [[ -n "${TEST_RUNNER_AFTER_SWITCH:-}" ]]; then
      echo "$TEST_RUNNER_AFTER_SWITCH"
    elif [[ "$(invocations)" -ge 2 && "$reads" -gt 0 ]]; then
      echo $((reads - 1)) > "$STATE/cycle-active-reads"
      echo active
    elif [[ "$(invocations)" -ge 2 && "${TEST_CYCLE:-success}" == fail ]]; then
      echo failed
    else
      echo inactive
    fi
    return
  fi
  state="$(head -n 1 "$STATE/runner")"
  if [[ "$(wc -l < "$STATE/runner")" -gt 1 ]]; then
    tail -n +2 "$STATE/runner" > "$STATE/runner.next"
    mv "$STATE/runner.next" "$STATE/runner"
  fi
  printf '%s\n' "$state"
}
case "$1" in
  is-active)
    case "$unit" in
      finite-saas-runner.timer)
        if timer_query_fails; then exit 4; fi
        [[ "$(cat "$STATE/timer")" == active ]] ;;
      *) exit 0 ;;
    esac
    ;;
  stop)
    if [[ "$unit" == finite-saas-runner.timer ]]; then
      case "${TEST_TIMER_STOP:-}" in
        fail-after-switch) [[ ! -e "$STATE/switched" ]] || exit 1 ;;
        noop-before-switch) [[ -e "$STATE/switched" ]] || exit 0 ;;
      esac
      echo inactive > "$STATE/timer"
    fi
    exit 0
    ;;
  start)
    if [[ "$unit" == finite-saas-runner.timer ]]; then
      if [[ -n "${TEST_FAIL_TIMER_RESTORE:-}" && -e "$STATE/rollback-started" ]]; then exit 1; fi
      # A failed start condition skips the start; systemctl still exits 0.
      [[ -e "$TEST_PAUSE_MARKER" ]] || echo active > "$STATE/timer"
    fi
    exit 0
    ;;
  daemon-reload) exit 0 ;;
  show)
    case "$*" in
      *finite-saas-runner.timer*)
        if timer_query_fails; then echo "Failed to connect to bus" >&2; exit 1; fi
        cat "$STATE/timer" ;;
      *finite-saas-runner.service*InvocationID*|*InvocationID*finite-saas-runner.service*) echo "id-$(invocations)" ;;
      *finite-saas-runner.service*Result*|*Result*finite-saas-runner.service*)
        if [[ "$(invocations)" -ge 2 && "${TEST_CYCLE:-success}" == fail ]]; then echo exit-code; else echo success; fi
        ;;
      *finite-saas-runner.service*) runner_state ;;
      *containerd.service*) echo 111 ;;
      *systemd-networkd.service*)
        if [[ -e "$STATE/switched" ]]; then echo "${NETWORKD_PID_AFTER:-222}"; else echo 222; fi
        ;;
      *) exit 91 ;;
    esac
    ;;
  cat) printf '[Service]\nExecStart=%s/bin/finite-saas-runner run-once\n' "$RUNNER_STORE" ;;
  --failed) if [[ -e "$STATE/switched" ]]; then printf '%b' "${FAILED_UNITS:-}"; fi ;;
  *) exit 90 ;;
esac
"""

READLINK_SHIM = r"""
target="${!#}"
case "$target" in
  /run/current-system) cat "$STATE/current" ;;
  /nix/var/nix/profiles/system) cat "$STATE/profile" ;;
  *) echo "$target" ;;
esac
"""

NIX_ENV_SHIM = r"""
echo "nix-env $*" >> "$CALL_LOG"
target="${!#}"
if [[ "$target" == "$TEST_CURRENT" ]]; then
  touch "$STATE/rollback-started"
  [[ -z "${TEST_FAIL_PROFILE_RESET:-}" ]] || exit 1
fi
echo "$target" > "$STATE/profile"
"""

NIX_STORE_SHIM = r"""
echo "nix-store $*" >> "$CALL_LOG"
case "$1" in
  --check-validity) exit 0 ;;
  -qR)
    # The Runner path first. With TEST_CLOSURE_FILLER set, a listing far
    # larger than a pipe buffer follows: a reader that stops at the first
    # match leaves this writer on EPIPE.
    printf '%s\n' "$RUNNER_STORE"
    if [[ "${TEST_CLOSURE_FILLER:-0}" -gt 0 ]]; then
      yes /nix/store/zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz-filler | head -n "$TEST_CLOSURE_FILLER"
    fi
    ;;
  *) exit 92 ;;
esac
"""

# Each poll interval after the switch lets the running timer start one more
# Runner invocation.
SLEEP_SHIM = r"""
if [[ -e "$STATE/switched" && "$(cat "$STATE/timer")" == active && "${TEST_CYCLE:-success}" != none ]]; then
  echo $(( $(cat "$STATE/invocations") + 1 )) > "$STATE/invocations"
fi
exec /bin/sleep 0.05
"""

# The candidate's switch: record the Runner state it crossed and whether the
# pause was held, then start the timer the way switch-to-configuration does
# when timers.target wants it. The timer's first run starts inside the switch.
SWITCH_SHIM = r"""
paused=no
[[ ! -e "$TEST_PAUSE_MARKER" ]] || paused=yes
echo "switch $* runner=$(head -n 1 "$STATE/runner") paused=$paused" >> "$CALL_LOG"
touch "$STATE/switched"
echo "$TEST_SYSTEM" > "$STATE/current"
if [[ -e "$TEST_SYSTEM/etc/systemd/system/timers.target.wants/finite-saas-runner.timer" && "$paused" == no ]]; then
  echo active > "$STATE/timer"
  echo 1 > "$STATE/invocations"
  echo "switch started the timer" >> "$CALL_LOG"
fi
exit "${SWITCH_STATUS:-0}"
"""

TOUCH_SHIM = r"""
if [[ "${!#}" == "$TEST_PAUSE_MARKER" ]]; then
  [[ -z "${TEST_FAIL_MARKER:-}" ]] || exit 1
  /usr/bin/touch "$@"
  if [[ -n "${TEST_TERM_AFTER_MARKER:-}" ]]; then kill -TERM "$PPID"; fi
  exit 0
fi
exec /usr/bin/touch "$@"
"""

PREVIOUS_SWITCH_SHIM = r"""
echo "rollback-switch $*" >> "$CALL_LOG"
[[ -z "${TEST_FAIL_OLD_SWITCH:-}" ]] || exit 1
echo "$TEST_CURRENT" > "$STATE/current"
# With TEST_FAIL_TIMER_RESTORE the timer cannot start at all, here or later.
[[ -e "$TEST_PAUSE_MARKER" || -n "${TEST_FAIL_TIMER_RESTORE:-}" ]] || echo active > "$STATE/timer"
"""


class DeployHarness:
    def __init__(self, test: unittest.TestCase, host: str) -> None:
        self.test = test
        self.host = host
        self.script = ROOT / f"scripts/deploy-{host}-closure-cache"
        self._tmp = tempfile.TemporaryDirectory()
        test.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.artifact = self.root / "artifact"
        (self.artifact / "nix-cache").mkdir(parents=True)
        (self.artifact / "manifest.json").write_text(json.dumps(manifest(host)) + "\n")
        (self.artifact / "nix-cache/nix-cache-info").write_text("StoreDir: /nix/store\n")
        self.store = self.root / "store"
        self.system = Path(manifest(host)["system"].replace("/nix/store", str(self.store)))
        self.current = self.store / ("e" * 32 + f"-nixos-system-finite-lat-{host[-1]}-26.05.old")
        self.state = self.root / "state"
        self.state.mkdir()
        self.runtime_units = self.root / "run-systemd-system"
        self.runtime_units.mkdir()
        self.pause_marker = self.root / "run-finite-deploy-runner-pause"
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.calls = self.root / "calls"
        self.calls.touch()
        self.dry_output = self.root / "dry-output"
        self.dry_output.write_text("would restart the following units: finite-saas-runner.service\n")
        for system in (self.system, self.current):
            (system / "bin").mkdir(parents=True)
            (system / "etc/systemd/system").mkdir(parents=True)
            (system / "etc/systemd/system/finite-saas-runner.timer").write_text("[Timer]\n")
        write_shim(self.system / "bin", "switch-to-configuration", SWITCH_SHIM)
        write_shim(self.current / "bin", "switch-to-configuration", PREVIOUS_SWITCH_SHIM)
        self.set_timer_wanted(True)
        self.set_timer("active")
        self.set_runner_states("inactive")
        (self.state / "current").write_text(f"{self.current}\n")
        (self.state / "profile").write_text(f"{self.current}\n")
        (self.state / "invocations").write_text("0\n")
        (self.bin / "ssh").write_text("#!" + sys.executable + "\n" + SSH_SHIM)
        (self.bin / "ssh").chmod(0o755)
        write_shim(self.bin, "systemctl", SYSTEMCTL_SHIM)
        write_shim(self.bin, "readlink", READLINK_SHIM)
        write_shim(self.bin, "nix-store", NIX_STORE_SHIM)
        write_shim(self.bin, "nix-env", NIX_ENV_SHIM)
        write_shim(self.bin, "nix", "exit 0\n")
        write_shim(self.bin, "sleep", SLEEP_SHIM)
        write_shim(self.bin, "touch", TOUCH_SHIM)
        write_shim(
            self.bin,
            "git",
            'if [[ "$1" == show ]]; then cat "$TEST_DASHBOARD_NIX"; fi\nexit 0\n',
        )
        self.dashboard_nix = self.root / "dashboard.nix"
        self.dashboard_nix.write_text(DASHBOARD_PIN)
        self.env: dict[str, str] = {}

    def set_timer_wanted(self, wanted: bool) -> None:
        wants = self.system / "etc/systemd/system/timers.target.wants"
        link = wants / "finite-saas-runner.timer"
        if wanted:
            wants.mkdir(exist_ok=True)
            link.symlink_to("../finite-saas-runner.timer")
        elif link.is_symlink():
            link.unlink()

    def set_timer(self, state: str) -> None:
        (self.state / "timer").write_text(state + "\n")

    def timer(self) -> str:
        return (self.state / "timer").read_text().strip()

    def link(self, name: str) -> str:
        return (self.state / name).read_text().strip()

    def set_runner_states(self, *states: str) -> None:
        (self.state / "runner").write_text("".join(s + "\n" for s in states))

    def write_tree(self, system: Path, files: dict[str, str]) -> None:
        for rel, text in files.items():
            path = system / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)

    def run(self, mode: str, *flags: str) -> subprocess.CompletedProcess[str]:
        env = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "CALL_LOG": str(self.calls),
            "STATE": str(self.state),
            "TEST_STORE_ROOT": str(self.store),
            "TEST_SYSTEM": str(self.system),
            "TEST_CURRENT": str(self.current),
            "TEST_DRY_OUTPUT": str(self.dry_output),
            "TEST_DASHBOARD_NIX": str(self.dashboard_nix),
            "TEST_PAUSE_MARKER": str(self.pause_marker),
            "TEST_RUNTIME_UNITS": str(self.runtime_units),
            "RUNNER_STORE": RUNNER_STORE,
            **self.env,
        }
        return subprocess.run(
            [str(self.script), mode, *flags, str(self.artifact)],
            cwd=ROOT,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )

    def call_lines(self) -> list[str]:
        return self.calls.read_text().splitlines()

    def pause_left_behind(self) -> list[str]:
        left = [str(p) for p in self.runtime_units.rglob("*finite-deploy-runner-pause*")]
        if self.pause_marker.exists():
            left.append(str(self.pause_marker))
        return left


OTHER_FAILED_UNITS = (
    "finite-saas-runner.service loaded failed failed Finite SaaS Runner\\n"
    "containerd.service loaded failed failed containerd\\n"
)


class RunnerActivationTests(unittest.TestCase):
    """Activation behavior shared by the three Runner hosts."""

    host = ""

    def harness(self) -> DeployHarness:
        return DeployHarness(self, self.host)

    def assert_deployed(self, h: DeployHarness, result: subprocess.CompletedProcess[str]) -> None:
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("==> DEPLOYED system=", result.stdout)
        self.assertEqual(h.link("current"), str(h.system))
        self.assertEqual(h.link("profile"), str(h.system))
        self.assertEqual(h.pause_left_behind(), [])

    def assert_rollback_verified(self, h: DeployHarness, result: subprocess.CompletedProcess[str]) -> None:
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("==> DEPLOYED", result.stdout)
        self.assertIn("rolling back", result.stderr)
        self.assertIn("ROLLBACK VERIFIED", result.stderr)
        self.assertEqual(h.link("current"), str(h.current))
        self.assertEqual(h.link("profile"), str(h.current))
        self.assertEqual(h.pause_left_behind(), [])

    # Quiescence

    def test_switch_waits_while_the_runner_one_shot_is_activating(self) -> None:
        h = self.harness()
        h.set_runner_states("activating", "activating", "deactivating", "inactive")
        result = h.run("--activate")
        self.assert_deployed(h, result)
        self.assertIn("switch switch runner=inactive paused=no", h.call_lines())

    def test_switch_waits_while_an_exec_cgroup_runner_reads_active(self) -> None:
        # The effective unit on today's hosts is Type=exec with ExitType=cgroup:
        # it reads active while any process in its cgroup runs.
        h = self.harness()
        h.set_runner_states("active", "active", "deactivating", "inactive")
        result = h.run("--activate")
        self.assert_deployed(h, result)
        self.assertIn("switch switch runner=inactive paused=no", h.call_lines())

    def test_a_failed_runner_counts_as_quiescent(self) -> None:
        h = self.harness()
        h.set_runner_states("activating", "failed")
        result = h.run("--activate")
        self.assert_deployed(h, result)
        self.assertIn("switch switch runner=failed paused=no", h.call_lines())

    def test_the_effective_runner_unit_reads_inactive_only_with_an_empty_cgroup(self) -> None:
        # The quiescence wait relies on these settings; revisit it if they change.
        host = (ROOT / f"infra/nixos/hosts/finite-lat-{self.host[-1]}/default.nix").read_text()
        module = (ROOT / "infra/nixos/modules/hosted-hermes.nix").read_text()
        self.assertRegex(host, r"finite\.hostedHermes = \{\n    enable = true;")
        for setting in ('Type = lib.mkForce "exec";', 'ExitType = "cgroup";', 'KillMode = lib.mkForce "control-group";'):
            self.assertIn(setting, module)

    def test_quiescence_timeout_restores_the_timer_without_switching(self) -> None:
        h = self.harness()
        h.set_runner_states("activating")
        h.env["TEST_SHORT_DEADLINE"] = "1"
        result = h.run("--activate")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("still activating", result.stderr)
        calls = h.call_lines()
        self.assertFalse(any(line.startswith(("switch ", "nix-env ", "rollback-switch")) for line in calls), calls)
        self.assertEqual(h.timer(), "active")

    def test_an_active_state_that_never_ends_refuses_without_switching(self) -> None:
        # A RemainAfterExit unit would read active with no process left. The
        # wait cannot tell that from a running cycle, so it refuses.
        h = self.harness()
        h.set_runner_states("active")
        h.env["TEST_SHORT_DEADLINE"] = "1"
        result = h.run("--activate")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("still active", result.stderr)
        self.assertFalse(any(line.startswith(("switch ", "nix-env ")) for line in h.call_lines()))
        self.assertEqual(h.timer(), "active")

    # Timer ownership

    def test_a_timer_stopped_before_the_script_is_refused(self) -> None:
        h = self.harness()
        h.set_timer("inactive")
        result = h.run("--activate")
        self.assertEqual(result.returncode, 78, result.stderr)
        self.assertIn("finite-saas-runner.timer is already stopped", result.stderr)
        self.assertIn("--keep-runner-paused", result.stderr)
        calls = h.call_lines()
        self.assertFalse(any(line.startswith(("switch ", "nix-env ")) for line in calls), calls)
        self.assertEqual(h.timer(), "inactive")

    def test_the_timer_runs_after_the_switch_when_the_candidate_enables_it(self) -> None:
        h = self.harness()
        result = h.run("--activate")
        self.assert_deployed(h, result)
        self.assertEqual(h.timer(), "active")

    def test_a_candidate_that_disables_the_timer_needs_keep_runner_paused(self) -> None:
        h = self.harness()
        h.set_timer_wanted(False)
        result = h.run("--activate")
        self.assertEqual(result.returncode, 78, result.stderr)
        self.assertIn("does not enable finite-saas-runner.timer", result.stderr)
        self.assertIn("--keep-runner-paused", result.stderr)
        calls = h.call_lines()
        self.assertFalse(any(line.startswith(("switch ", "nix-env ")) for line in calls), calls)
        self.assertNotIn("systemctl stop finite-saas-runner.timer", calls)
        self.assertEqual(h.timer(), "active")

    def test_a_candidate_that_disables_the_timer_deploys_with_keep_runner_paused(self) -> None:
        h = self.harness()
        h.set_timer_wanted(False)
        result = h.run("--activate", "--keep-runner-paused")
        self.assert_deployed(h, result)
        self.assertEqual(h.timer(), "inactive")
        self.assertIn("candidate Runner was not exercised", result.stdout)

    # Unknown timer state never counts as stopped

    def test_an_unreadable_timer_before_the_switch_is_refused(self) -> None:
        h = self.harness()
        h.env["TEST_TIMER_QUERY_FAIL"] = "always"
        result = h.run("--activate")
        self.assertEqual(result.returncode, 78, result.stderr)
        self.assertIn("cannot read the state of finite-saas-runner.timer", result.stderr)
        calls = h.call_lines()
        self.assertNotIn("systemctl stop finite-saas-runner.timer", calls)
        self.assertFalse(any(line.startswith(("switch ", "nix-env ")) for line in calls), calls)

    def test_a_timer_still_active_after_stop_is_not_crossed(self) -> None:
        h = self.harness()
        h.env["TEST_TIMER_STOP"] = "noop-before-switch"
        result = h.run("--activate")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("finite-saas-runner.timer is active after stop", result.stderr)
        self.assertFalse(any(line.startswith(("switch ", "nix-env ")) for line in h.call_lines()))
        self.assertEqual(h.timer(), "active")

    # Pause installation and cleanup

    def test_a_failed_pause_marker_leaves_no_runtime_files(self) -> None:
        h = self.harness()
        h.set_timer("inactive")
        h.env["TEST_FAIL_MARKER"] = "1"
        result = h.run("--activate", "--keep-runner-paused")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(line.startswith(("switch ", "nix-env ")) for line in h.call_lines()))
        self.assertEqual(h.pause_left_behind(), [])
        self.assertEqual(h.timer(), "inactive")

    def test_an_interrupted_pause_installation_is_cleaned_up(self) -> None:
        h = self.harness()
        h.set_timer("inactive")
        h.env["TEST_TERM_AFTER_MARKER"] = "1"
        result = h.run("--activate", "--keep-runner-paused")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("release it by hand with: rm -f", result.stdout)
        self.assertFalse(any(line.startswith(("switch ", "nix-env ")) for line in h.call_lines()))
        self.assertEqual(h.pause_left_behind(), [])

    def test_leftover_pause_files_are_refused(self) -> None:
        h = self.harness()
        leftover = h.runtime_units / "finite-saas-runner.timer.d/50-finite-deploy-runner-pause.conf"
        leftover.parent.mkdir()
        leftover.write_text("[Unit]\n")
        result = h.run("--activate")
        self.assertEqual(result.returncode, 78, result.stderr)
        self.assertIn("a Runner pause from an earlier run is still installed", result.stderr)
        self.assertIn("50-finite-deploy-runner-pause.conf", result.stderr)
        calls = h.call_lines()
        self.assertNotIn("systemctl stop finite-saas-runner.timer", calls)
        self.assertFalse(any(line.startswith(("switch ", "nix-env ")) for line in calls), calls)

    @unittest.skipIf(os.geteuid() == 0, "root searches mode 0 directories")
    def test_an_unsearchable_runtime_unit_directory_is_refused(self) -> None:
        h = self.harness()
        h.runtime_units.chmod(0)
        self.addCleanup(h.runtime_units.chmod, 0o755)
        result = h.run("--activate")
        self.assertEqual(result.returncode, 78, result.stderr)
        self.assertIn("cannot check for an earlier Runner pause", result.stderr)
        self.assertFalse(any(line.startswith(("switch ", "nix-env ")) for line in h.call_lines()))

    def test_keep_runner_paused_holds_the_pause_through_the_switch(self) -> None:
        h = self.harness()
        h.set_timer("inactive")
        result = h.run("--activate", "--keep-runner-paused")
        self.assert_deployed(h, result)
        calls = h.call_lines()
        self.assertIn("switch switch runner=inactive paused=yes", calls)
        self.assertNotIn("switch started the timer", calls)
        self.assertEqual(h.timer(), "inactive")
        self.assertEqual(h.link("invocations"), "0")
        self.assertIn("candidate Runner was not exercised", result.stdout)

    def test_keep_runner_paused_stops_a_running_timer_and_keeps_it_stopped(self) -> None:
        h = self.harness()
        result = h.run("--activate", "--keep-runner-paused")
        self.assert_deployed(h, result)
        self.assertIn("switch switch runner=inactive paused=yes", h.call_lines())
        self.assertEqual(h.timer(), "inactive")

    def test_keep_runner_paused_rollback_restores_the_stopped_timer(self) -> None:
        h = self.harness()
        h.set_timer("inactive")
        h.env["SWITCH_STATUS"] = "4"
        h.env["FAILED_UNITS"] = OTHER_FAILED_UNITS
        result = h.run("--activate", "--keep-runner-paused")
        self.assert_rollback_verified(h, result)
        self.assertEqual(h.timer(), "inactive")

    # Membership

    def test_membership_check_reads_the_whole_closure_listing(self) -> None:
        h = self.harness()
        h.env["TEST_CLOSURE_FILLER"] = "50000"
        result = h.run("--activate")
        self.assert_deployed(h, result)
        self.assertNotIn("rolling back", result.stderr)

    # Candidate Runner cycle

    def test_success_requires_a_successful_candidate_runner_cycle(self) -> None:
        h = self.harness()
        result = h.run("--activate")
        self.assert_deployed(h, result)
        self.assertIn("candidate Runner cycle id-2 finished with Result=success", result.stdout)

    def test_a_running_candidate_cycle_is_awaited(self) -> None:
        h = self.harness()
        h.env["TEST_CYCLE_ACTIVE_READS"] = "3"
        result = h.run("--activate")
        self.assert_deployed(h, result)
        self.assertIn("finished with Result=success", result.stdout)

    def test_a_failing_candidate_runner_cycle_rolls_back(self) -> None:
        h = self.harness()
        h.env["TEST_CYCLE"] = "fail"
        result = h.run("--activate")
        self.assert_rollback_verified(h, result)
        self.assertIn("Result=exit-code", result.stderr)
        self.assertEqual(h.timer(), "active")

    def test_a_runner_left_failed_by_the_switch_rolls_back(self) -> None:
        # The switch exits 4 with only the Runner failed, and the candidate
        # Runner keeps failing: a broken binary is not a deploy.
        h = self.harness()
        h.env["SWITCH_STATUS"] = "4"
        h.env["FAILED_UNITS"] = "finite-saas-runner.service loaded failed failed Finite SaaS Runner\\n"
        h.env["TEST_CYCLE"] = "fail"
        result = h.run("--activate")
        self.assert_rollback_verified(h, result)

    def assert_nonzero_switch_rolls_back(self, h: DeployHarness, *flags: str) -> None:
        # The failed-unit list does not say why the switch failed, so any
        # nonzero switch is a failed deploy, even with only the Runner listed.
        h.env["SWITCH_STATUS"] = "2"
        h.env["FAILED_UNITS"] = "finite-saas-runner.service loaded failed failed Finite SaaS Runner\\n"
        result = h.run("--activate", *flags)
        self.assert_rollback_verified(h, result)
        self.assertIn("switch-to-configuration exited 2; failed units: finite-saas-runner.service", result.stderr)
        self.assertNotIn("candidate Runner cycle decides", result.stdout)

    def test_a_nonzero_switch_with_only_the_runner_failed_rolls_back(self) -> None:
        h = self.harness()
        self.assert_nonzero_switch_rolls_back(h)
        self.assertEqual(h.timer(), "active")

    def test_a_nonzero_switch_with_only_the_runner_failed_rolls_back_when_paused(self) -> None:
        h = self.harness()
        h.set_timer("inactive")
        self.assert_nonzero_switch_rolls_back(h, "--keep-runner-paused")
        self.assertEqual(h.timer(), "inactive")
        self.assertEqual(h.link("invocations"), "0")

    def test_no_candidate_runner_cycle_within_the_bound_rolls_back(self) -> None:
        h = self.harness()
        h.env["TEST_CYCLE"] = "none"
        h.env["TEST_SHORT_DEADLINE"] = "1"
        result = h.run("--activate")
        self.assert_rollback_verified(h, result)
        self.assertIn("no Runner cycle that started after the switch finished", result.stderr)

    # Switch failures and rollback

    def test_a_switch_with_other_failed_units_rolls_back(self) -> None:
        h = self.harness()
        h.env["SWITCH_STATUS"] = "4"
        h.env["FAILED_UNITS"] = OTHER_FAILED_UNITS
        result = h.run("--activate")
        self.assert_rollback_verified(h, result)
        self.assertIn("containerd.service", result.stderr)
        self.assertIn("rollback-switch switch", h.call_lines())
        self.assertEqual(h.timer(), "active")

    def test_a_switch_failure_without_failed_units_rolls_back(self) -> None:
        h = self.harness()
        h.env["SWITCH_STATUS"] = "1"
        result = h.run("--activate")
        self.assert_rollback_verified(h, result)

    def test_a_failed_profile_reset_is_reported(self) -> None:
        h = self.harness()
        h.env["SWITCH_STATUS"] = "1"
        h.env["TEST_FAIL_PROFILE_RESET"] = "1"
        result = h.run("--activate")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ROLLBACK INCOMPLETE", result.stderr)
        self.assertIn(f"the system profile is {h.system}", result.stderr)
        self.assertNotIn("ROLLBACK VERIFIED", result.stderr)
        self.assertIn("Next:", result.stderr)
        self.assertEqual(h.link("current"), str(h.current))

    def test_a_failed_previous_activation_is_reported(self) -> None:
        h = self.harness()
        h.env["SWITCH_STATUS"] = "1"
        h.env["TEST_FAIL_OLD_SWITCH"] = "1"
        result = h.run("--activate")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ROLLBACK INCOMPLETE", result.stderr)
        self.assertIn(f"/run/current-system is {h.system}", result.stderr)
        self.assertEqual(h.link("profile"), str(h.current))
        self.assertEqual(sum(line.startswith("rollback-switch") for line in h.call_lines()), 1)

    def test_a_failed_timer_restore_is_reported(self) -> None:
        h = self.harness()
        h.env["SWITCH_STATUS"] = "1"
        h.env["TEST_FAIL_TIMER_RESTORE"] = "1"
        result = h.run("--activate")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ROLLBACK INCOMPLETE", result.stderr)
        self.assertIn("restoring finite-saas-runner.timer to active failed", result.stderr)
        self.assertIn("finite-saas-runner.timer is inactive; expected active", result.stderr)
        self.assertEqual(h.link("current"), str(h.current))
        self.assertEqual(h.link("profile"), str(h.current))

    def assert_no_activation_attempted(self, h: DeployHarness, result: subprocess.CompletedProcess[str]) -> None:
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ROLLBACK INCOMPLETE", result.stderr)
        self.assertIn("previous system not activated", result.stderr)
        self.assertNotIn("ROLLBACK VERIFIED", result.stderr)
        self.assertIn("Next:", result.stderr)
        calls = h.call_lines()
        self.assertNotIn("rollback-switch switch", calls)
        self.assertEqual(sum(line.startswith("nix-env ") for line in calls), 1)
        self.assertEqual(h.link("current"), str(h.system))
        self.assertIn(f"observed: /run/current-system is {h.system}", result.stderr)

    def test_rollback_never_crosses_an_in_flight_runner(self) -> None:
        h = self.harness()
        h.env["SWITCH_STATUS"] = "4"
        h.env["FAILED_UNITS"] = OTHER_FAILED_UNITS
        h.env["TEST_RUNNER_AFTER_SWITCH"] = "active"
        h.env["TEST_SHORT_DEADLINE"] = "1"
        result = h.run("--activate")
        self.assert_no_activation_attempted(h, result)
        self.assertIn("observed: finite-saas-runner.service is active", result.stderr)

    def test_rollback_does_not_activate_past_a_timer_it_cannot_stop(self) -> None:
        h = self.harness()
        h.env["SWITCH_STATUS"] = "1"
        h.env["TEST_TIMER_STOP"] = "fail-after-switch"
        result = h.run("--activate")
        self.assert_no_activation_attempted(h, result)
        self.assertIn("observed: finite-saas-runner.timer is active", result.stderr)
        self.assertNotIn("timer is stopped", result.stderr)

    def test_rollback_does_not_activate_past_an_unreadable_timer(self) -> None:
        h = self.harness()
        h.env["SWITCH_STATUS"] = "1"
        h.env["TEST_TIMER_QUERY_FAIL"] = "after-switch"
        result = h.run("--activate")
        self.assert_no_activation_attempted(h, result)
        self.assertIn("observed: finite-saas-runner.timer is unknown", result.stderr)

    def test_rollback_verification_treats_an_unreadable_timer_as_unknown(self) -> None:
        h = self.harness()
        h.set_timer("inactive")
        h.env["SWITCH_STATUS"] = "4"
        h.env["FAILED_UNITS"] = OTHER_FAILED_UNITS
        h.env["TEST_TIMER_QUERY_FAIL"] = "after-rollback"
        result = h.run("--activate", "--keep-runner-paused")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ROLLBACK INCOMPLETE", result.stderr)
        self.assertIn("finite-saas-runner.timer is unknown; expected inactive", result.stderr)
        self.assertNotIn("ROLLBACK VERIFIED", result.stderr)

    # PID approvals

    def test_an_approved_networkd_restart_passes_the_pid_check(self) -> None:
        h = self.harness()
        h.dry_output.write_text("would restart the following units: systemd-networkd.service\n")
        h.env["NETWORKD_PID_AFTER"] = "333"
        result = h.run("--activate", "--allow-unit", "systemd-networkd.service")
        self.assert_deployed(h, result)

    def test_an_unapproved_networkd_restart_rolls_back(self) -> None:
        h = self.harness()
        h.env["NETWORKD_PID_AFTER"] = "333"
        result = h.run("--activate")
        self.assert_rollback_verified(h, result)
        self.assertIn("systemd-networkd.service restarted during activation", result.stderr)

    def test_the_cold_start_retry_is_gone(self) -> None:
        # The old retry sat behind an ERR trap that fired first.
        source = (ROOT / f"scripts/deploy-{self.host}-closure-cache").read_text()
        self.assertNotIn("set +e", source)
        self.assertNotIn("retrying once", source)


def write_reference_trees(h: DeployHarness) -> None:
    """Running and candidate systems that differ only in store hashes."""
    store = h.store
    for system, tag in ((h.current, "1"), (h.system, "2")):
        system_path = store / (tag * 32 + "-system-path")
        h.write_tree(
            system,
            {
                "etc/tmpfiles.d/00-nixos.conf": (
                    "d /data/finite-saas-runner 0700 root root - -\n"
                    f"L+ /run/finite-monitoring/finite-version-static.prom - - - - {store}/{tag * 32}-finite-version-static.prom\n"
                ),
                "etc/dbus-1/system.conf": (
                    '<?xml version="1.0"?>\n<!DOCTYPE busconfig SYSTEM "busconfig.dtd">\n'
                    f"<busconfig><includedir>{system_path}/share/dbus-1/system.d</includedir>"
                    f"<servicedir>{system_path}/share/dbus-1/system-services</servicedir></busconfig>\n"
                ),
                "etc/systemd/system/dbus-broker.service": (
                    f"[Unit]\nX-Reload-Triggers={store}/{tag * 32}-X-Reload-Triggers-dbus-broker\n"
                ),
                "etc/systemd/system/systemd-tmpfiles-resetup.service": (
                    f"[Unit]\nX-Restart-Triggers={store}/{tag * 32}-X-Restart-Triggers-systemd-tmpfiles-resetup\n"
                ),
            },
        )
        h.write_tree(
            system_path,
            {
                "share/dbus-1/system.d/org.freedesktop.login1.conf": '<busconfig><policy user="root"/></busconfig>\n',
                "share/dbus-1/system-services/org.freedesktop.login1.service": (
                    f"[D-BUS Service]\nExec={store}/{tag * 32}-systemd/lib/systemd/systemd-logind\n"
                ),
            },
        )


class FenceTests(unittest.TestCase):
    """Dry-activation fence behavior shared by all four closure deploy scripts."""

    host = ""
    dry_prefix = ""

    def fence_harness(self) -> DeployHarness:
        h = DeployHarness(self, self.host)
        h.dry_output.write_text(self.dry_prefix + REFERENCE_DRY_OUTPUT)
        write_reference_trees(h)
        return h

    def test_reference_only_tmpfiles_and_dbus_changes_are_admitted(self) -> None:
        h = self.fence_harness()
        result = h.run("--prepare")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"==> PREPARED rev={REV}", result.stdout)
        self.assertIn("reference-only systemd-tmpfiles-resetup.service", result.stdout)
        self.assertIn("reference-only dbus-broker.service", result.stdout)

    def test_a_tmpfiles_rule_change_needs_explicit_approval(self) -> None:
        h = self.fence_harness()
        rules = h.system / "etc/tmpfiles.d/00-nixos.conf"
        rules.write_text(rules.read_text() + "R /data/finite-saas-runner - - - - -\n")
        result = h.run("--prepare")
        self.assertEqual(result.returncode, 75, result.stderr)
        self.assertIn("systemd-tmpfiles-resetup.service", result.stderr)
        self.assertNotIn("dbus-broker.service", result.stderr)
        self.assertIn("+R /data/finite-saas-runner", result.stdout)
        result = h.run("--prepare", "--allow-unit", "systemd-tmpfiles-resetup.service")
        self.assertEqual(result.returncode, 0, result.stderr)

    @unittest.skipIf(os.geteuid() == 0, "root reads mode 0 files")
    def test_an_unreadable_tmpfiles_rule_file_needs_approval(self) -> None:
        h = self.fence_harness()
        rules = h.system / "etc/tmpfiles.d/00-nixos.conf"
        rules.write_text(rules.read_text() + "R /data/finite-saas-runner - - - - -\n")
        rules.chmod(0)
        self.addCleanup(rules.chmod, 0o644)
        result = h.run("--prepare")
        self.assertEqual(result.returncode, 75, result.stderr)
        self.assertIn("incomplete systemd-tmpfiles-resetup.service", result.stdout)
        self.assertIn("could not inspect", result.stdout)
        self.assertIn("systemd-tmpfiles-resetup.service", result.stderr)

    def test_a_dbus_policy_change_behind_a_store_reference_needs_approval(self) -> None:
        h = self.fence_harness()
        policy = h.store / ("2" * 32 + "-system-path") / "share/dbus-1/system.d/org.freedesktop.login1.conf"
        policy.write_text('<busconfig><policy context="default"><allow send_destination="*"/></policy></busconfig>\n')
        result = h.run("--prepare")
        self.assertEqual(result.returncode, 75, result.stderr)
        self.assertIn("dbus-broker.service", result.stderr)
        self.assertIn("allow send_destination", result.stdout)

    def test_a_dbus_broker_restart_needs_approval(self) -> None:
        h = self.fence_harness()
        h.dry_output.write_text(
            self.dry_prefix
            + REFERENCE_DRY_OUTPUT.replace("would reload the following units: dbus-broker.service\n", "")
            + "would restart the following units: dbus-broker.service\n"
        )
        result = h.run("--prepare")
        self.assertEqual(result.returncode, 75, result.stderr)
        self.assertIn("dbus-broker.service", result.stderr)

    def test_other_units_are_still_refused(self) -> None:
        h = self.fence_harness()
        h.dry_output.write_text(
            self.dry_prefix + REFERENCE_DRY_OUTPUT + "would restart the following units: alloy.service\n"
        )
        result = h.run("--prepare")
        self.assertEqual(result.returncode, 75, result.stderr)
        self.assertIn("alloy.service", result.stderr)

    def test_a_helper_that_cannot_run_admits_nothing(self) -> None:
        h = self.fence_harness()
        h.env["TEST_COMPARE_EXIT"] = "127"
        result = h.run("--prepare")
        self.assertEqual(result.returncode, 75, result.stderr)
        self.assertIn("configuration comparison exited 127", result.stderr)
        self.assertIn("dbus-broker.service", result.stderr)
        self.assertIn("systemd-tmpfiles-resetup.service", result.stderr)


class Lat3RunnerDeployTests(RunnerActivationTests, FenceTests):
    host = "lat3"


class Lat4RunnerDeployTests(RunnerActivationTests, FenceTests):
    host = "lat4"


class Lat5RunnerDeployTests(RunnerActivationTests, FenceTests):
    host = "lat5"


class Lat2FenceTests(FenceTests):
    host = "lat2"
    dry_prefix = "would restart the following units: finitechat-server.service\n"

    def test_prepare_refuses_a_rev_without_a_dashboard_pin(self) -> None:
        h = self.fence_harness()
        h.dashboard_nix.write_text("{ ... }: { }\n")
        result = h.run("--prepare")
        self.assertEqual(result.returncode, 65, result.stderr)
        self.assertIn("no single dashboard image pin", result.stderr)
        self.assertNotIn("dry-activating", result.stdout)

    def test_prepare_refuses_a_rev_with_two_dashboard_pins(self) -> None:
        h = self.fence_harness()
        h.dashboard_nix.write_text(DASHBOARD_PIN + DASHBOARD_PIN)
        result = h.run("--prepare")
        self.assertEqual(result.returncode, 65, result.stderr)
        self.assertIn("no single dashboard image pin", result.stderr)


class CompareActivationConfigTests(unittest.TestCase):
    """The comparison helper itself, run as the deploy scripts run it."""

    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.h = DeployHarness(self, "lat5")
        self.h.dry_output.write_text(REFERENCE_DRY_OUTPUT)
        write_reference_trees(self.h)
        self.selinuxfs = Path(tmp.name) / "selinuxfs-absent"

    def compare(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                "-",
                "--current",
                str(self.h.current),
                "--store-dir",
                str(self.h.store),
                "--selinuxfs",
                str(self.selinuxfs),
                str(self.h.system),
            ],
            input=COMPARE.read_text(),
            text=True,
            capture_output=True,
            check=False,
        )

    def verdicts(self) -> dict[str, str]:
        result = self.compare()
        self.assertEqual(result.returncode, 0, result.stderr)
        verdicts = {}
        for line in result.stdout.splitlines():
            for verdict in ("reference-only", "content-change", "incomplete"):
                if line.startswith(verdict + " "):
                    verdicts[line.split(" ", 1)[1]] = verdict
        self.output = result.stdout
        return verdicts

    def dbus(self) -> str:
        return self.verdicts()["dbus-broker.service"]

    def tmpfiles(self) -> str:
        return self.verdicts()["systemd-tmpfiles-resetup.service"]

    def system_conf(self, system: Path, body: str) -> None:
        tag = "2" if system == self.h.system else "1"
        system_path = self.h.store / (tag * 32 + "-system-path")
        (system / "etc/dbus-1/system.conf").write_text(
            '<?xml version="1.0"?>\n'
            f"<busconfig><includedir>{system_path}/share/dbus-1/system.d</includedir>{body}</busconfig>\n"
        )

    def both_system_conf(self, body_for) -> None:
        for system, tag in ((self.h.current, "1"), (self.h.system, "2")):
            self.system_conf(system, body_for(self.h.store / (tag * 32 + "-extra")))

    def test_hash_only_changes_are_reference_only(self) -> None:
        self.assertEqual(self.verdicts(), {
            "systemd-tmpfiles-resetup.service": "reference-only",
            "dbus-broker.service": "reference-only",
        })

    def test_a_changed_file_include_behind_a_store_hash_is_a_change(self) -> None:
        self.both_system_conf(lambda extra: f"<include>{extra}/extra-policy.conf</include>")
        for tag, user in (("1", "root"), ("2", "nobody")):
            path = self.h.store / (tag * 32 + "-extra/extra-policy.conf")
            path.parent.mkdir(parents=True)
            path.write_text(f'<busconfig><policy user="{user}"><allow own="*"/></policy></busconfig>\n')
        self.assertEqual(self.dbus(), "content-change")
        self.assertIn('policy user="nobody"', self.output)

    def test_a_nested_include_is_followed(self) -> None:
        self.both_system_conf(lambda extra: f"<include>{extra}/outer.conf</include>")
        for tag, user in (("1", "root"), ("2", "nobody")):
            extra = self.h.store / (tag * 32 + "-extra")
            extra.mkdir()
            (extra / "outer.conf").write_text("<busconfig><include>inner/leaf.conf</include></busconfig>\n")
            (extra / "inner").mkdir()
            (extra / "inner/leaf.conf").write_text(f'<busconfig><policy user="{user}"/></busconfig>\n')
        self.assertEqual(self.dbus(), "content-change")
        self.assertIn('policy user="nobody"', self.output)

    def test_an_include_in_an_included_directory_is_followed(self) -> None:
        for tag, user in (("1", "root"), ("2", "nobody")):
            system_path = self.h.store / (tag * 32 + "-system-path")
            # Only *.conf files are read from an included directory, so the
            # policy below is reached through the include alone.
            (system_path / "share/dbus-1/system.d/org.freedesktop.login1.conf").write_text(
                "<busconfig><include>local.policy</include></busconfig>\n"
            )
            (system_path / "share/dbus-1/system.d/local.policy").write_text(
                f'<busconfig><policy user="{user}"/></busconfig>\n'
            )
        self.assertEqual(self.dbus(), "content-change")

    def test_an_include_cycle_is_incomplete(self) -> None:
        self.both_system_conf(lambda extra: f"<include>{extra}/a.conf</include>")
        for tag in ("1", "2"):
            extra = self.h.store / (tag * 32 + "-extra")
            extra.mkdir()
            (extra / "a.conf").write_text("<busconfig><include>b.conf</include></busconfig>\n")
            (extra / "b.conf").write_text("<busconfig><include>a.conf</include></busconfig>\n")
        self.assertEqual(self.dbus(), "incomplete")
        self.assertIn("include cycle", self.output)

    def test_a_missing_include_is_incomplete_unless_ignore_missing(self) -> None:
        self.both_system_conf(lambda extra: f"<include>{extra}/missing.conf</include>")
        self.assertEqual(self.dbus(), "incomplete")
        self.both_system_conf(lambda extra: f'<include ignore_missing="yes">{extra}/missing.conf</include>')
        self.assertEqual(self.dbus(), "reference-only")

    def test_standard_service_directories_are_incomplete(self) -> None:
        self.both_system_conf(lambda extra: "<standard_system_servicedirs/>")
        self.assertEqual(self.dbus(), "incomplete")

    def test_an_include_outside_the_store_and_etc_is_incomplete(self) -> None:
        self.both_system_conf(lambda extra: "<include>/usr/share/dbus-1/system.conf</include>")
        self.assertEqual(self.dbus(), "incomplete")

    def test_an_selinux_only_include_is_skipped_when_selinux_is_off(self) -> None:
        self.both_system_conf(
            lambda extra: '<include if_selinux_enabled="yes" selinux_root_relative="yes">contexts/dbus_contexts</include>'
        )
        self.assertEqual(self.dbus(), "reference-only")
        self.selinuxfs.mkdir()
        (self.selinuxfs / "enforce").write_text("1\n")
        self.assertEqual(self.dbus(), "incomplete")

    def test_malformed_dbus_xml_is_incomplete(self) -> None:
        (self.h.system / "etc/dbus-1/system.conf").write_text("<busconfig><include>\n")
        self.assertEqual(self.dbus(), "incomplete")

    @unittest.skipIf(os.geteuid() == 0, "root reads mode 0 directories")
    def test_an_unreadable_included_directory_is_incomplete(self) -> None:
        policy_dir = self.h.store / ("2" * 32 + "-system-path") / "share/dbus-1/system.d"
        policy_dir.chmod(0)
        self.addCleanup(policy_dir.chmod, 0o755)
        self.assertEqual(self.dbus(), "incomplete")
        self.assertIn("could not inspect", self.output)

    def test_a_dangling_link_in_the_tmpfiles_tree_is_incomplete(self) -> None:
        (self.h.system / "etc/tmpfiles.d/10-extra.conf").symlink_to(self.h.store / "gone.conf")
        self.assertEqual(self.tmpfiles(), "incomplete")
        self.assertIn("dangling", self.output)

    def test_a_link_loop_in_the_tmpfiles_tree_is_incomplete(self) -> None:
        (self.h.system / "etc/tmpfiles.d/loop").symlink_to(self.h.system / "etc/tmpfiles.d")
        self.assertEqual(self.tmpfiles(), "incomplete")

    def test_a_non_regular_file_in_the_tmpfiles_tree_is_incomplete(self) -> None:
        os.mkfifo(self.h.system / "etc/tmpfiles.d/pipe.conf")
        self.assertEqual(self.tmpfiles(), "incomplete")

    # Absence is only FileNotFoundError; every other error is incomplete. In each
    # case below the running side lacks the optional path, and the candidate's
    # copy sits behind a parent that cannot be inspected.

    def candidate_extra(self) -> Path:
        return self.h.store / ("2" * 32 + "-extra")

    def hide_candidate_extra_behind(self, how: str) -> None:
        extra = self.candidate_extra()
        if how == "loop":
            extra.mkdir()
            (extra / "sub").symlink_to(extra / "sub")
        else:
            (extra / "sub/policy.d").mkdir(parents=True)
            (extra / "sub/policy.d/hidden.conf").write_text('<busconfig><policy user="nobody"/></busconfig>\n')
            (extra / "sub/p.conf").write_text('<busconfig><policy user="nobody"/></busconfig>\n')
            (extra / "sub/services").mkdir()
            extra.chmod(0)
            self.addCleanup(extra.chmod, 0o755)

    @unittest.skipIf(os.geteuid() == 0, "root searches mode 0 directories")
    def test_an_includedir_under_an_inaccessible_parent_is_incomplete(self) -> None:
        self.both_system_conf(lambda extra: f"<includedir>{extra}/sub/policy.d</includedir>")
        self.hide_candidate_extra_behind("mode 0")
        self.assertEqual(self.dbus(), "incomplete")
        self.assertIn("could not inspect (candidate)", self.output)

    @unittest.skipIf(os.geteuid() == 0, "root searches mode 0 directories")
    def test_an_ignore_missing_include_under_an_inaccessible_parent_is_incomplete(self) -> None:
        self.both_system_conf(lambda extra: f'<include ignore_missing="yes">{extra}/sub/p.conf</include>')
        self.hide_candidate_extra_behind("mode 0")
        self.assertEqual(self.dbus(), "incomplete")

    def test_an_includedir_under_a_parent_link_loop_is_incomplete(self) -> None:
        self.both_system_conf(lambda extra: f"<includedir>{extra}/sub/policy.d</includedir>")
        self.hide_candidate_extra_behind("loop")
        self.assertEqual(self.dbus(), "incomplete")

    def test_an_ignore_missing_include_under_a_parent_link_loop_is_incomplete(self) -> None:
        self.both_system_conf(lambda extra: f'<include ignore_missing="yes">{extra}/sub/p.conf</include>')
        self.hide_candidate_extra_behind("loop")
        self.assertEqual(self.dbus(), "incomplete")

    def test_an_optional_service_directory_under_a_parent_link_loop_is_incomplete(self) -> None:
        self.both_system_conf(lambda extra: f"<servicedir>{extra}/sub/services</servicedir>")
        self.hide_candidate_extra_behind("loop")
        self.assertEqual(self.dbus(), "incomplete")

    def test_an_selinux_probe_that_cannot_be_read_is_incomplete(self) -> None:
        self.both_system_conf(
            lambda extra: '<include if_selinux_enabled="yes" selinux_root_relative="yes">contexts/dbus_contexts</include>'
        )
        self.selinuxfs.symlink_to(self.selinuxfs)
        self.assertEqual(self.dbus(), "incomplete")
        self.assertIn("SELinux", self.output)

    def test_a_missing_tmpfiles_directory_is_incomplete(self) -> None:
        for path in (self.h.system / "etc/tmpfiles.d").iterdir():
            path.unlink()
        (self.h.system / "etc/tmpfiles.d").rmdir()
        self.assertEqual(self.tmpfiles(), "incomplete")


# The shared bases carry no host; only the per-host classes run.
del RunnerActivationTests, FenceTests


if __name__ == "__main__":
    unittest.main()
