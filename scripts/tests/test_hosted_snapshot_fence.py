"""Synthetic proof of the hosted Recovery Snapshot write fence.

Renders the production snapshot script from the NixOS configuration and runs
it against a modelled systemctl, synthetic SQLite state, and a runner that
connects to the private proxy sockets while the copies run. The model keeps
the systemd behaviour that broke the fence on 2026-09-18 (FIN-95): a
listening socket activates its proxy service, whose Requires= starts the
fenced writer again.
"""

from __future__ import annotations

import os
from pathlib import Path
import shutil
import sqlite3
import stat
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SNAPSHOT_SCRIPT = "config.systemd.services.finite-hosted-web-chat-snapshot.script"
PROXIES = ("finite-core-private-proxy", "finite-identity-private-proxy")
SOCKET_LOOP = "for socket in finite-core-private-proxy finite-identity-private-proxy; do"
SOCKET_STOP = 'for socket in "${sockets_were_active[@]}"; do fence_stop "$socket.socket"; done'
WRITERS = (
    "finite-saas-core.service",
    "finite-brain-app.service",
    "finitechat-hosted-device.service",
    "finite-identity.service",
    "finitechat-server.service",
)
LIVE_PATHS = {
    "/data/recovery-snapshots/hosted-web-chat": "snapshots",
    "/var/lib/private/finitechat-hosted-device": "live/hosted-device",
    "/var/lib/private/finite-chat/data/server.sqlite3": "live/chat/server.sqlite3",
    "/var/lib/private/finitebrain/finite-brain.sqlite3": "live/brain/finite-brain.sqlite3",
    "/var/lib/finite-identity/identity.db": "live/identity/identity.db",
}

# systemctl model: unit state files, a per-unit start counter standing in for
# InactiveExitTimestampMonotonic, and the proxy services' Requires= edges.
FAKE_SYSTEMD = r"""
unit_state() { cat "$FAKE/units/$1" 2>/dev/null || echo missing; }
fake_start() {
  local unit=$1
  [ "$(unit_state "$unit")" != missing ] || return 5
  case $unit in
    finite-core-private-proxy.service) fake_start finite-saas-core.service ;;
    finite-identity-private-proxy.service) fake_start finite-identity.service ;;
  esac
  if [ "$(unit_state "$unit")" != active ]; then
    echo active > "$FAKE/units/$unit"
    echo $(( $(cat "$FAKE/exits/$unit") + 1 )) > "$FAKE/exits/$unit"
    echo "start $unit" >> "$FAKE/log"
  fi
}
fake_stop() {
  local unit=$1 required_by
  [ "$(unit_state "$unit")" != missing ] || return 5
  case $unit in
    finite-saas-core.service) required_by=finite-core-private-proxy.service ;;
    finite-identity.service) required_by=finite-identity-private-proxy.service ;;
    *) required_by= ;;
  esac
  if [ -n "$required_by" ] && [ "$(unit_state "$required_by")" != missing ]; then
    fake_stop "$required_by"
  fi
  if [ "$(unit_state "$unit")" = active ]; then
    echo inactive > "$FAKE/units/$unit"
    echo "stop $unit" >> "$FAKE/log"
  fi
}
systemctl() {
  case $1 in
    is-active) [ "$(unit_state "$3")" = active ] ;;
    start) fake_start "$2" ;;
    stop) fake_stop "$2" ;;
    show) cat "$FAKE/exits/$4" ;;
    *) return 1 ;;
  esac
}
runner_lease_attempt() {
  local proxy
  for proxy in finite-core-private-proxy finite-identity-private-proxy; do
    if [ "$(unit_state "$proxy.socket")" = active ]; then fake_start "$proxy.service"; fi
  done
}
runuser() {
  runner_lease_attempt
  case ${FENCE_SCENARIO:-} in
    unfenced-start) fake_start finite-identity.service ;;
    start-then-die) fake_start finite-saas-core.service; fake_stop finite-saas-core.service ;;
  esac
  echo synthetic-dump
}
pg_restore() { :; }
"""


def nix_eval_raw(host: str, attribute: str) -> str:
    return subprocess.run(
        ["nix", "eval", "--raw", f".#nixosConfigurations.{host}.{attribute}"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def modern_bash() -> str:
    bash = shutil.which("bash")
    if bash is None:
        raise AssertionError("bash is required")
    major = subprocess.run(
        [bash, "-c", "echo ${BASH_VERSINFO[0]}"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if int(major) < 4:
        raise AssertionError(f"{bash} is bash {major}; run through scripts/with-dev-env")
    return bash


class HostedSnapshotFenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.bash = modern_bash()
        cls.scripts = {
            host: nix_eval_raw(host, SNAPSHOT_SCRIPT)
            for host in ("finite-lat-2", "finite-lat-1")
        }

    def setUp(self) -> None:
        self.scratch = tempfile.TemporaryDirectory()

    def tearDown(self) -> None:
        for path in Path(self.scratch.name).rglob("*"):
            if not path.is_symlink():
                path.chmod(path.stat().st_mode | stat.S_IWUSR)
        self.scratch.cleanup()

    def make_live_state(self) -> None:
        self.root = Path(tempfile.mkdtemp(dir=self.scratch.name))
        live = self.root / "live"
        client = live / "hosted-device" / "users" / "user-a" / "chat" / "client.sqlite3"
        identity = live / "hosted-device" / "users" / "user-a" / "finite-home" / "identity" / "identity.json"
        for database in (
            client,
            live / "chat" / "server.sqlite3",
            live / "brain" / "finite-brain.sqlite3",
            live / "identity" / "identity.db",
        ):
            database.parent.mkdir(parents=True, exist_ok=True)
            connection = sqlite3.connect(database)
            connection.execute("CREATE TABLE proof (value TEXT NOT NULL)")
            connection.execute("INSERT INTO proof VALUES ('synthetic')")
            connection.commit()
            connection.close()
        identity.parent.mkdir(parents=True)
        identity.write_text('{"kind":"synthetic"}\n', encoding="utf-8")
        (self.root / "snapshots").mkdir()

    def make_units(self, units: dict[str, str]) -> Path:
        fake = self.root / "systemd"
        (fake / "units").mkdir(parents=True)
        (fake / "exits").mkdir()
        (fake / "log").write_text("", encoding="utf-8")
        for unit, state in units.items():
            (fake / "units" / unit).write_text(f"{state}\n", encoding="utf-8")
            (fake / "exits" / unit).write_text("1\n", encoding="utf-8")
        return fake

    def lat2_units(self) -> dict[str, str]:
        units = {unit: "active" for unit in WRITERS}
        for proxy in PROXIES:
            units[f"{proxy}.socket"] = "active"
            units[f"{proxy}.service"] = "active"
        return units

    def run_snapshot(
        self, script: str, units: dict[str, str], scenario: str = ""
    ) -> tuple[subprocess.CompletedProcess[str], list[str], dict[str, str]]:
        self.make_live_state()
        fake = self.make_units(units)
        for live, relative in LIVE_PATHS.items():
            self.assertIn(live, script)
            script = script.replace(live, str(self.root / relative))
        program = self.root / "snapshot.sh"
        program.write_text(FAKE_SYSTEMD + script, encoding="utf-8")
        result = subprocess.run(
            [self.bash, "-e", str(program)],
            capture_output=True,
            check=False,
            env={**os.environ, "FAKE": str(fake), "FENCE_SCENARIO": scenario},
            text=True,
        )
        log = (fake / "log").read_text(encoding="utf-8").splitlines()
        states = {
            unit.name: unit.read_text(encoding="utf-8").strip()
            for unit in (fake / "units").iterdir()
        }
        return result, log, states

    def sealed_snapshots(self) -> list[Path]:
        return sorted(
            path
            for path in (self.root / "snapshots").iterdir()
            if path.name != "latest"
        )

    def assert_nothing_sealed(self) -> None:
        self.assertEqual(self.sealed_snapshots(), [])
        self.assertFalse((self.root / "snapshots" / "latest").is_symlink())

    def test_every_host_importing_backups_fences_both_private_proxy_sockets(self) -> None:
        for host, script in self.scripts.items():
            with self.subTest(host=host):
                self.assertIn(SOCKET_LOOP, script)
                self.assertIn(SOCKET_STOP, script)
                stop_sockets = script.index(SOCKET_STOP)
                stop_core = script.index("fence_stop finite-saas-core.service")
                stop_identity = script.index("fence_stop finite-identity.service")
                self.assertLess(stop_sockets, stop_core)
                self.assertLess(stop_sockets, stop_identity)
                proof = script.index("write fence broken")
                self.assertLess(script.index("pg_restore --list"), proof)
                self.assertLess(proof, script.index('chmod -R a-w -- "$staging"'))

    def test_fence_holds_while_runners_connect_and_trap_restores_exactly(self) -> None:
        units = self.lat2_units()
        result, log, states = self.run_snapshot(self.scripts["finite-lat-2"], units)

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(
            "write fence held: "
            "finite-core-private-proxy.socket finite-identity-private-proxy.socket "
            "finite-core-private-proxy.service finite-identity-private-proxy.service "
            + " ".join(WRITERS),
            result.stdout,
        )
        self.assertEqual(
            log,
            [
                "stop finite-core-private-proxy.socket",
                "stop finite-identity-private-proxy.socket",
                "stop finite-core-private-proxy.service",
                "stop finite-identity-private-proxy.service",
                "stop finite-saas-core.service",
                "stop finite-brain-app.service",
                "stop finitechat-hosted-device.service",
                "stop finite-identity.service",
                "stop finitechat-server.service",
                "start finitechat-server.service",
                "start finite-identity.service",
                "start finite-saas-core.service",
                "start finite-brain-app.service",
                "start finitechat-hosted-device.service",
                "start finite-core-private-proxy.socket",
                "start finite-identity-private-proxy.socket",
                "start finite-core-private-proxy.service",
                "start finite-identity-private-proxy.service",
            ],
        )
        self.assertEqual(states, units)
        [snapshot] = self.sealed_snapshots()
        self.assertIn(f"Hosted Recovery Snapshot sealed: {snapshot}", result.stdout)
        self.assertLess(
            result.stdout.index("write fence held"),
            result.stdout.index("Hosted Recovery Snapshot sealed"),
        )
        self.assertEqual(
            os.readlink(self.root / "snapshots" / "latest"), snapshot.name
        )
        check = subprocess.run(
            [str(ROOT / "scripts" / "verify-hosted-snapshot"), str(snapshot)],
            capture_output=True,
            check=False,
            text=True,
        )
        self.assertEqual(check.returncode, 0, check.stdout + check.stderr)

    def test_listening_socket_reproduces_fin95_and_the_fence_proof_fails_closed(self) -> None:
        script = self.scripts["finite-lat-2"]
        self.assertIn(SOCKET_STOP, script)
        units = self.lat2_units()
        result, log, states = self.run_snapshot(script.replace(SOCKET_STOP, ""), units)

        self.assertNotEqual(result.returncode, 0)
        # The runner's connection restarted Core after the fence stopped it.
        self.assertLess(
            log.index("stop finite-saas-core.service"),
            log.index("start finite-saas-core.service"),
        )
        self.assertIn("write fence broken: finite-saas-core.service", result.stderr)
        self.assertIn("write fence broken: finite-identity.service", result.stderr)
        self.assertNotIn("write fence held", result.stdout)
        self.assertNotIn("Hosted Recovery Snapshot sealed", result.stdout)
        self.assert_nothing_sealed()
        self.assertEqual(states, units)

    def test_any_writer_start_during_the_copies_fails_the_snapshot(self) -> None:
        for scenario, unit in (
            ("unfenced-start", "finite-identity.service"),
            ("start-then-die", "finite-saas-core.service"),
        ):
            with self.subTest(scenario=scenario):
                units = self.lat2_units()
                result, _, states = self.run_snapshot(
                    self.scripts["finite-lat-2"], units, scenario
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"write fence broken: {unit}", result.stderr)
                self.assert_nothing_sealed()
                self.assertEqual(states, units)

    def test_hosts_without_the_sockets_snapshot_unchanged(self) -> None:
        script = self.scripts["finite-lat-2"].replace(SOCKET_LOOP, "for socket in ; do")
        units = {unit: "active" for unit in WRITERS}
        result, log, states = self.run_snapshot(script, units)

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("write fence held: " + " ".join(WRITERS), result.stdout)
        self.assertFalse(any("private-proxy" in line for line in log))
        self.assertEqual(states, units)
        self.assertEqual(len(self.sealed_snapshots()), 1)


if __name__ == "__main__":
    unittest.main()
