import hashlib
import json
import subprocess
import unittest
from unittest.mock import patch

from scripts import finite_status


class RuntimeRouteStatusTests(unittest.TestCase):
    def report(
        self,
        published,
        direct,
        expected=None,
        *,
        ports=None,
        stopped_owner=False,
        unlabelled_owner=False,
    ):
        inspected = {
            "labels": {
                "computer.finite.v2.runtime": "true",
                "computer.finite.v2.source_machine_id": "machine-test",
            },
            "state": "running",
            "ports": {"8080/tcp": ports or [{"HostPort": "49153"}]},
            "networks": {"bridge": {"IPAddress": "10.4.0.8"}},
        }
        values = [inspected, {"agent_npub": published}, {"agent_npub": direct}]
        results = [
            subprocess.CompletedProcess([], 0, json.dumps(v), "") for v in values
        ]
        results.append(subprocess.CompletedProcess([], 0, "", ""))
        if stopped_owner:
            owner = {
                **inspected,
                "name": "stopped-agent",
                "state": "exited",
                "ports": {},
            }
            if unlabelled_owner:
                owner["labels"] = None
            results.extend(
                [
                    subprocess.CompletedProcess([], 0, "stopped-agent\n", ""),
                    subprocess.CompletedProcess([], 0, json.dumps(owner), ""),
                    subprocess.CompletedProcess(
                        [], 0, "8080/tcp -> 10.254.3.5:49153\n", ""
                    ),
                ]
            )
        else:
            results.append(subprocess.CompletedProcess([], 0, "", ""))
        with (
            patch.object(finite_status, "run_read_only", side_effect=results) as run,
            patch.object(
                finite_status,
                "read_environment_values",
                return_value={"FC_RUNNER_KATA_HOST_ADDRESS": "10.254.3.5"},
            ),
        ):
            report = finite_status.collect_runtime_route("machine-test", expected)
        self.assertEqual(
            run.call_args_list[1].args[0][-1], "http://10.254.3.5:49153/contact"
        )
        self.assertEqual(
            run.call_args_list[2].args[0][-1], "http://10.4.0.8:8080/contact"
        )
        return report

    def test_same_principal_on_both_routes_matches_expected(self):
        principal = "npub1" + "a" * 58
        report = self.report(
            principal, principal, hashlib.sha256(principal.encode()).hexdigest()
        )
        self.assertEqual(report["overall_status"], "green")
        self.assertNotIn(principal, json.dumps(report))

    def test_stopped_container_reservation_is_visible_when_inspect_ports_are_empty(
        self,
    ):
        principal = "npub1" + "a" * 58
        report = self.report(principal, principal, stopped_owner=True)
        owners = report["sections"]["runtime_route"]["port_owners"]
        self.assertEqual(owners["binding_source"], "nerdctl port")
        self.assertEqual(owners["containers"][0]["state"], "exited")
        self.assertEqual(owners["containers"][0]["bindings"][0]["HostPort"], "49153")

    def test_saved_binding_parser_rejects_partial_or_malformed_evidence(self):
        with self.assertRaises(ValueError):
            finite_status.runtime_saved_port_bindings(
                "8080/tcp -> 10.4.0.8:49153\ntruncated", {49153}
            )

    def test_unlabelled_container_still_reserves_its_saved_port(self):
        principal = "npub1" + "a" * 58
        report = self.report(
            principal, principal, stopped_owner=True, unlabelled_owner=True
        )
        owners = report["sections"]["runtime_route"]["port_owners"]
        self.assertEqual(owners["status"], "observed")
        self.assertEqual(len(owners["containers"]), 1)
        self.assertIsNone(owners["containers"][0]["project_id"])
        self.assertEqual(owners["containers"][0]["bindings"][0]["HostPort"], "49153")

    def test_saved_binding_parser_distinguishes_protocol_and_ipv6(self):
        bindings = finite_status.runtime_saved_port_bindings(
            "8080/tcp -> [::1]:49153\n8080/udp -> 127.0.0.1:49153\n8080/tcp -> 127.0.0.1:49154\n",
            {49153},
        )
        self.assertEqual(len(bindings), 1)
        self.assertEqual(bindings[0]["HostIp"], "::1")

    def test_published_port_serving_another_principal_is_red(self):
        report = self.report("npub1" + "a" * 58, "npub1" + "q" * 58)
        self.assertEqual(report["overall_status"], "red")

    def test_agreeing_routes_cannot_override_expected_principal(self):
        report = self.report("npub1" + "a" * 58, "npub1" + "a" * 58, "0" * 64)
        self.assertEqual(report["overall_status"], "red")

    def test_unavailable_contact_is_unknown(self):
        self.assertEqual(
            self.report(None, "npub1" + "a" * 58)["overall_status"], "unknown"
        )

    def test_ambiguous_ports_fail_without_contact_request(self):
        with self.assertRaises(finite_status.CollectionError):
            self.report(
                None, None, ports=[{"HostPort": "49153"}, {"HostPort": "49154"}]
            )

    def test_option_cannot_be_a_nerdctl_flag(self):
        with self.assertRaises(SystemExit):
            finite_status.parse_args(["--runtime-route=-a"])

    def test_port_rules_preserve_competing_rule_order_without_other_routes(self):
        first = "-A CNI-HOSTPORT-DNAT -p tcp --dport 49153 -j CNI-DN-old"
        second = "-A CNI-HOSTPORT-DNAT -p tcp --dport 49153 -j CNI-DN-current"
        old = "-A CNI-DN-old -p tcp -j DNAT --to-destination 10.4.0.9:8080"
        current = "-A CNI-DN-current -p tcp -j DNAT --to-destination 10.4.0.26:8080"
        other = "-A CNI-HOSTPORT-DNAT -p tcp --dport 49154 -j CNI-DN-other"
        self.assertEqual(
            finite_status.runtime_port_rules(
                "\n".join([first, second, old, current, other]), 49153
            ),
            [first, second, old, current],
        )
