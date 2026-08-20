#!/usr/bin/env python3
"""Contract tests for the post-switch service-exe verification in
scripts/deploy-lat1-closure-cache.

The verification runs remotely over ssh as the VERIFY heredoc. These tests
extract that exact heredoc body from the deploy script and execute it locally
against a fake systemd/nix environment (fake systemctl/readlink/nix-store on
PATH), covering the twice-observed production race: a service active but
running its OLD store-path binary after the switch.
"""

from __future__ import annotations

import os
from pathlib import Path
import re
import subprocess
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[2]
DEPLOY = ROOT / "scripts" / "deploy-lat1-closure-cache"

NEW_SYSTEM = "/nix/store/" + "b" * 32 + "-nixos-system-finite-lat-1-25.11.test"
LONG_RUNNING = [
    "finite-saas-core",
    "finitechat-server",
    "finitechat-hosted-device",
    "finite-brain-app",
    "finite-saas-sites",
    "finite-identity",
]
RUN_ONCE = "finite-saas-runner"


def new_exe(unit: str) -> str:
    return f"/nix/store/{'n' * 32}-{unit}-1.0/bin/{unit}"


def old_exe(unit: str) -> str:
    return f"/nix/store/{'o' * 32}-{unit}-0.9/bin/{unit}"


def extract_verify_heredoc() -> str:
    source = DEPLOY.read_text(encoding="utf-8")
    match = re.search(r"<<'VERIFY'\n(.*?)\nVERIFY\n", source, re.DOTALL)
    if match is None:
        raise AssertionError("VERIFY heredoc not found in deploy script")
    return match.group(1)


FAKE_SYSTEMCTL = """\
#!/usr/bin/env bash
set -euo pipefail
dir="$FAKE_UNITS_DIR"
unit="${!#}"
case "${1:-}" in
  cat)
    cat "$dir/$unit.cat"
    ;;
  show)
    cat "$dir/$unit.mainpid"
    ;;
  restart)
    echo "restart $unit" >>"$FAKE_RESTART_LOG"
    if [[ -f "$dir/$unit.exe.after-restart" ]]; then
      cp "$dir/$unit.exe.after-restart" "$dir/exe-by-pid/$(cat "$dir/$unit.mainpid")"
    fi
    ;;
  is-active)
    [[ "$(cat "$dir/$unit.isactive")" == active ]]
    ;;
  *)
    echo "unexpected systemctl invocation: $*" >&2
    exit 2
    ;;
esac
"""

FAKE_READLINK = """\
#!/usr/bin/env bash
set -euo pipefail
target="${!#}"
if [[ "$target" == /proc/*/exe ]]; then
  pid="${target#/proc/}"
  pid="${pid%/exe}"
  cat "$FAKE_UNITS_DIR/exe-by-pid/$pid"
elif [[ "$target" == /nix/store/* ]]; then
  # Store paths are not materialized in the test sandbox; identity resolution
  # is sufficient because both sides of the comparison resolve the same way.
  printf '%s\\n' "$target"
else
  echo "unexpected readlink target: $target" >&2
  exit 2
fi
"""

FAKE_NIX_STORE = """\
#!/usr/bin/env bash
set -euo pipefail
cat "$FAKE_UNITS_DIR/closure.txt"
"""


class ServiceExeVerificationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.units = self.root / "units"
        (self.units / "exe-by-pid").mkdir(parents=True)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name, body in (
            ("systemctl", FAKE_SYSTEMCTL),
            ("readlink", FAKE_READLINK),
            ("nix-store", FAKE_NIX_STORE),
        ):
            fake = self.bin / name
            fake.write_text(body, encoding="utf-8")
            fake.chmod(0o755)
        self.script = self.root / "verify.sh"
        self.script.write_text(extract_verify_heredoc(), encoding="utf-8")
        self.restart_log = self.root / "restarts.log"

    def fixture(
        self,
        *,
        stale: str | None = None,
        restart_fixes: bool = True,
        runner_in_closure: bool = True,
    ) -> None:
        for index, unit in enumerate(LONG_RUNNING):
            pid = str(4100 + index)
            (self.units / f"{unit}.cat").write_text(
                f"# /etc/systemd/system/{unit}.service\n"
                f"[Service]\nExecStart={new_exe(unit)} serve --flag\n",
                encoding="utf-8",
            )
            (self.units / f"{unit}.mainpid").write_text(pid, encoding="utf-8")
            (self.units / f"{unit}.isactive").write_text("active", encoding="utf-8")
            running = old_exe(unit) if unit == stale else new_exe(unit)
            (self.units / "exe-by-pid" / pid).write_text(running, encoding="utf-8")
            if unit == stale and restart_fixes:
                (self.units / f"{unit}.exe.after-restart").write_text(
                    new_exe(unit), encoding="utf-8"
                )
        (self.units / f"{RUN_ONCE}.cat").write_text(
            f"[Service]\nType=oneshot\nExecStart={new_exe(RUN_ONCE)} run-once\n",
            encoding="utf-8",
        )
        closure_paths = [f"/nix/store/{'n' * 32}-{RUN_ONCE}-1.0"] if runner_in_closure else []
        (self.units / "closure.txt").write_text(
            "".join(f"{path}\n" for path in closure_paths), encoding="utf-8"
        )

    def run_verify(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(self.script), NEW_SYSTEM],
            env={
                **os.environ,
                "PATH": f"{self.bin}:{os.environ['PATH']}",
                "FAKE_UNITS_DIR": str(self.units),
                "FAKE_RESTART_LOG": str(self.restart_log),
            },
            text=True,
            capture_output=True,
            check=False,
        )

    def restarts(self) -> list[str]:
        if not self.restart_log.exists():
            return []
        return self.restart_log.read_text(encoding="utf-8").splitlines()

    def test_all_services_on_new_closure_pass(self) -> None:
        self.fixture()
        result = self.run_verify()
        self.assertEqual(result.returncode, 0, result.stderr)
        for unit in LONG_RUNNING + [RUN_ONCE]:
            self.assertIn(f"unit={unit} exe={new_exe(unit)} status=OK", result.stdout)
        self.assertEqual(self.restarts(), [])

    def test_stale_service_is_restarted_then_passes(self) -> None:
        self.fixture(stale="finite-identity")
        result = self.run_verify()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(
            f"unit=finite-identity exe={new_exe('finite-identity')} status=RESTARTED",
            result.stdout,
        )
        self.assertEqual(self.restarts(), ["restart finite-identity"])
        self.assertIn(old_exe("finite-identity"), result.stderr)

    def test_still_stale_after_restart_fails_loudly(self) -> None:
        self.fixture(stale="finite-saas-core", restart_fixes=False)
        result = self.run_verify()
        self.assertEqual(result.returncode, 1)
        self.assertIn("DEPLOY FAILED: finite-saas-core", result.stderr)
        self.assertIn(old_exe("finite-saas-core"), result.stderr)
        self.assertIn(new_exe("finite-saas-core"), result.stderr)
        self.assertEqual(self.restarts(), ["restart finite-saas-core"])

    def test_run_once_execstart_outside_new_closure_fails(self) -> None:
        self.fixture(runner_in_closure=False)
        result = self.run_verify()
        self.assertEqual(result.returncode, 1)
        self.assertIn(f"DEPLOY FAILED: {RUN_ONCE}", result.stderr)

    def test_verification_sits_between_system_path_and_is_active_checks(self) -> None:
        # Plumbing guard: the heredoc must run after the current-system path
        # equality check and before the existing is-active check in the
        # deploy script itself.
        source = DEPLOY.read_text(encoding="utf-8")
        order = [
            source.index('test "$ACTUAL" = "$SYSTEM"'),
            source.index("<<'VERIFY'"),
            source.index("systemctl is-active finite-saas-core podman-finite-saas-dashboard"),
        ]
        self.assertEqual(order, sorted(order))


if __name__ == "__main__":
    unittest.main()
