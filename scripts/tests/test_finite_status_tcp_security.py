"""Avoid reporting green for incomplete emergency mitigations."""

import importlib.util
import shlex
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "tcp_security", Path(__file__).resolve().parents[1] / "finite_status_tcp_security.py"
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class TcpSecurityTests(unittest.TestCase):
    def setUp(self):
        self.rule = shlex.join(module.RULE)
        self.unit = {"ActiveState": "active", "WantedBy": "multi-user.target"}

    def test_complete_mitigation(self):
        self.assertEqual(module.verdict("0\n", self.rule, self.unit)["state"], "mitigated")

    def test_sysctl_alone_cannot_protect_existing_listeners(self):
        self.assertEqual(module.verdict("0", "", self.unit)["status"], "red")

    def test_earlier_accept_rule_cannot_bypass_barrier(self):
        rules = "-A PREROUTING -j ACCEPT\n" + self.rule
        self.assertEqual(module.verdict("0", rules, self.unit)["status"], "red")

    def test_unrelated_drop_is_not_evidence(self):
        rules = self.rule.replace("6:0xf", "7:0xf")
        self.assertEqual(module.verdict("0", rules, self.unit)["status"], "red")

    def test_runtime_only_unit_is_not_persistent(self):
        self.assertEqual(module.verdict("0", self.rule, {"ActiveState": "active"})["status"], "red")
