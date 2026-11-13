"""Avoid reporting green for incomplete emergency mitigations."""

import importlib.util
import shlex
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

spec = importlib.util.spec_from_file_location(
    "tcp_security", Path(__file__).resolve().parents[1] / "finite_status_tcp_security.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class TcpSecurityTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.rule = shlex.join(module.RULE)
        self.unit = {
            "ActiveState": "active", "WantedBy": "multi-user.target",
            "FragmentPath": f"/etc/systemd/system.control/{module.UNIT}",
        }
        source = self.root / "etc/finite/guard.service"
        source.parent.mkdir(parents=True)
        source.write_text("[Service]\nType=oneshot\n")
        self.attach("etc/systemd/system.control", source)

    def attach(self, directory, source):
        directory = self.root / directory
        wants = directory / "multi-user.target.wants"
        wants.mkdir(parents=True)
        (directory / module.UNIT).symlink_to(source)
        (wants / module.UNIT).symlink_to(source)

    def verdict(self, mptcp="0", rules=None, unit=None):
        return module.verdict(
            mptcp, self.rule if rules is None else rules,
            self.unit if unit is None else unit, root=self.root,
        )

    def test_complete_mitigation(self):
        self.assertEqual(self.verdict("0\n")["state"], "mitigated")

    def test_sysctl_alone_cannot_protect_existing_listeners(self):
        self.assertEqual(self.verdict(rules="")["status"], "red")

    def test_earlier_accept_rule_cannot_bypass_barrier(self):
        rules = "-A PREROUTING -j ACCEPT\n" + self.rule
        self.assertEqual(self.verdict(rules=rules)["status"], "red")

    def test_unrelated_drop_is_not_evidence(self):
        rules = self.rule.replace("6:0xf", "7:0xf")
        self.assertEqual(self.verdict(rules=rules)["status"], "red")

    def test_missing_loaded_boot_dependency_is_not_green(self):
        self.assertEqual(self.verdict(unit={**self.unit, "WantedBy": ""})["status"], "red")

    def test_active_enabled_runtime_unit_is_not_persistent(self):
        source = self.root / "run/systemd/system/guard.service"
        source.parent.mkdir(parents=True)
        source.write_text("[Service]\nType=oneshot\n")
        self.attach("run/systemd/system/units", source)
        self.unit["FragmentPath"] = "/run/systemd/system/guard.service"
        self.assertEqual(self.verdict()["status"], "red")

    def test_persistent_unit_with_only_runtime_wants_link_is_red(self):
        link = self.root / f"etc/systemd/system.control/multi-user.target.wants/{module.UNIT}"
        link.unlink()
        self.attach("run/systemd/system", self.root / "etc/finite/guard.service")
        self.assertEqual(self.verdict()["status"], "red")

    def test_etc_alias_to_runtime_unit_is_red(self):
        source = self.root / "run/guard.service"
        source.parent.mkdir()
        source.write_text("[Service]\nType=oneshot\n")
        for relative in (module.UNIT, f"multi-user.target.wants/{module.UNIT}"):
            path = self.root / "etc/systemd/system.control" / relative
            path.unlink()
            path.symlink_to(source)
        self.assertEqual(self.verdict()["status"], "red")

    def test_runtime_backed_wants_directory_is_red(self):
        wants = self.root / "etc/systemd/system.control/multi-user.target.wants"
        runtime = self.root / "run/wants"
        runtime.parent.mkdir()
        wants.rename(runtime)
        wants.symlink_to(runtime)
        self.assertEqual(self.verdict()["status"], "red")

    def nixos_handoff(self):
        for relative in (module.UNIT, f"multi-user.target.wants/{module.UNIT}"):
            (self.root / "etc/systemd/system.control" / relative).unlink()
        source = self.root / f"nix/store/unit-guard/{module.UNIT}"
        source.parent.mkdir(parents=True)
        source.write_text("[Service]\nType=oneshot\n")
        directory = self.root / "nix/store/system/etc/systemd/system"
        directory.mkdir(parents=True)
        (self.root / "etc/systemd/system").symlink_to(directory)
        self.attach("etc/systemd/system", source)
        self.unit["FragmentPath"] = f"/nix/store/unit-guard/{module.UNIT}"

    def test_declarative_nixos_unit_and_wants_symlinks_are_persistent(self):
        self.nixos_handoff()
        self.assertEqual(self.verdict()["status"], "green")

    def test_higher_priority_override_cannot_be_ignored(self):
        self.nixos_handoff()
        override = self.root / "etc/finite/override.service"
        override.write_text("[Service]\nType=oneshot\n")
        (self.root / f"etc/systemd/system.control/{module.UNIT}").symlink_to(override)
        self.assertEqual(self.verdict()["status"], "red")

    def test_boot_link_must_resolve_to_the_loaded_unit(self):
        link = self.root / f"etc/systemd/system.control/multi-user.target.wants/{module.UNIT}"
        other = self.root / "etc/finite/other.service"
        other.write_text("[Service]\nType=oneshot\n")
        link.unlink()
        link.symlink_to(other)
        self.assertEqual(self.verdict()["status"], "red")

    def test_collector_rejects_enabled_runtime_with_wantedby_present(self):
        properties = (
            "ActiveState=active\nWantedBy=multi-user.target\n"
            f"FragmentPath=/run/systemd/system/{module.UNIT}\n"
        )
        outputs = ["0\n", self.rule, properties]
        with mock.patch.object(module.subprocess, "run", side_effect=[
            subprocess.CompletedProcess([], 0, stdout=text) for text in outputs
        ]):
            result = module.collect(root=self.root)
        self.assertEqual(result["overall_status"], "red")
