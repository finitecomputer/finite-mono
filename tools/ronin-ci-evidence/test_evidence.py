"""Small synthetic contract suite; every negative asserts a nonpassing result."""

import copy
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from contract import (
    CAPABILITY,
    ContractError,
    check_manifest,
    read_json,
    validate_evidence,
)
from run import Execution, FakeBudget, run_fixture

ROOT = Path(__file__).resolve().parent
MANIFEST = read_json(ROOT / "fixtures/manifest.json")
COMPLETE = read_json(ROOT / "fixtures/complete.json")


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")


def changed(path, value):
    result = copy.deepcopy(COMPLETE)
    cursor = result
    for key in path[:-1]:
        cursor = cursor[key]
    cursor[path[-1]] = value
    return encoded(result)


def negative_evidence():
    """Yield independently mutated input, expected disposition, and exit code."""
    for label, raw in (
        ("empty", b""),
        ("whitespace", b" \n"),
        ("printed-pass", b"PASS\n"),
        ("malformed", b'{"complete":'),
        ("invalid-utf8", b"\xff"),
        ("null-root", b"null"),
        ("list-root", b"[]"),
        ("empty-object", b"{}"),
        ("trailing-pass", encoded(COMPLETE) + b"\nPASS"),
        ("duplicate-key", encoded(COMPLETE)[:-1] + b',"success":true}'),
        (
            "non-json-nan",
            encoded(COMPLETE).replace(b'"success":true', b'"success":NaN', 1),
        ),
    ):
        yield label, raw, "ERROR", 65
    for field in COMPLETE:
        result = copy.deepcopy(COMPLETE)
        del result[field]
        yield f"missing-{field}", encoded(result), "ERROR", 65
    for path in (("identity",), ("cleanup",), ("capabilities",), ("cases", 0)):
        cursor = COMPLETE
        for key in path:
            cursor = cursor[key]
        for field in cursor:
            result = copy.deepcopy(COMPLETE)
            parent = result
            for key in path:
                parent = parent[key]
            del parent[field]
            label = "missing-" + "-".join(map(str, path)) + "-" + field
            yield label, encoded(result), "ERROR", 65
    for path in (
        ("success",),
        ("complete",),
        ("cases", 0, "success"),
        ("cases", 0, "complete"),
        ("cleanup", "resolved"),
        ("capabilities", CAPABILITY),
    ):
        for index, value in enumerate((1, 0, "true", None)):
            label = "non-boolean-" + "-".join(map(str, path)) + f"-{index}"
            yield label, changed(path, value), "ERROR", 65
    mutations = (
        ("boolean-version", ("schema_version",), True, "ERROR", 65),
        ("unknown-version", ("schema_version",), 2, "ERROR", 65),
        ("guest-kind", ("kind",), "GUEST_EXECUTION", "ERROR", 65),
        ("stale-nonce", ("run_nonce",), "synthetic-fin174-run-previous", "ERROR", 65),
        ("source-mismatch", ("identity", "source_revision"), "b" * 40, "ERROR", 65),
        ("binary-mismatch", ("identity", "binary_sha256"), "b" * 64, "ERROR", 65),
        ("malformed-source", ("identity", "source_revision"), "base", "ERROR", 65),
        ("malformed-binary", ("identity", "binary_sha256"), "binary", "ERROR", 65),
        ("omitted-case", ("cases",), COMPLETE["cases"][:1], "ERROR", 65),
        (
            "extra-case",
            ("cases",),
            COMPLETE["cases"] + COMPLETE["cases"][:1],
            "ERROR",
            65,
        ),
        ("reordered-cases", ("cases",), list(reversed(COMPLETE["cases"])), "ERROR", 65),
        ("duplicate-case-id", ("cases", 1, "id"), "json-contract", "ERROR", 65),
        ("unknown-case", ("cases", 0, "id"), "unknown", "ERROR", 65),
        ("missing-progress", ("cases", 0, "progress"), [], "INCONCLUSIVE", 78),
        (
            "partial-progress",
            ("cases", 0, "progress"),
            ["FIXTURE_INPUT_READ"],
            "INCONCLUSIVE",
            78,
        ),
        (
            "non-list-progress",
            ("cases", 0, "progress"),
            "FIXTURE_INPUT_READ",
            "ERROR",
            65,
        ),
        ("unknown-progress", ("cases", 0, "progress"), ["GUEST_READY"], "ERROR", 65),
        (
            "reordered-progress",
            ("cases", 0, "progress"),
            list(reversed(COMPLETE["cases"][0]["progress"])),
            "ERROR",
            65,
        ),
        (
            "extra-progress",
            ("cases", 0, "progress"),
            COMPLETE["cases"][0]["progress"] + ["FIXTURE_CLEANUP_RESOLVED"],
            "ERROR",
            65,
        ),
        (
            "duplicate-progress",
            ("cases", 0, "progress"),
            ["FIXTURE_INPUT_READ"] * 2,
            "ERROR",
            65,
        ),
        (
            "unsupported-topology",
            ("topology",),
            "unverified-topology",
            "UNSUPPORTED",
            77,
        ),
        (
            "unsupported-capability",
            ("capabilities", CAPABILITY),
            False,
            "UNSUPPORTED",
            77,
        ),
        ("unknown-capability", ("capabilities",), {"guest": True}, "ERROR", 65),
        ("partial-phase", ("phase",), "PARTIAL", "INCONCLUSIVE", 78),
        ("unknown-phase", ("phase",), "BOOTED", "ERROR", 65),
        ("incomplete", ("complete",), False, "INCONCLUSIVE", 78),
        ("incomplete-case", ("cases", 0, "complete"), False, "INCONCLUSIVE", 78),
        ("unsuccessful", ("success",), False, "ERROR", 65),
        ("unsuccessful-case", ("cases", 0, "success"), False, "ERROR", 65),
        ("unresolved-cleanup", ("cleanup", "resolved"), False, "INCONCLUSIVE", 78),
        (
            "pending-cleanup",
            ("cleanup", "pending"),
            ["fixture-handle"],
            "INCONCLUSIVE",
            78,
        ),
        ("non-list-cleanup", ("cleanup", "pending"), "none", "ERROR", 65),
        ("unknown-outcome", ("outcome",), "PASS", "ERROR", 65),
    )
    for label, path, value, status, exit_code in mutations:
        yield label, changed(path, value), status, exit_code
    for outcome, status, exit_code in (
        ("UNSUPPORTED", "UNSUPPORTED", 77),
        ("INCONCLUSIVE", "INCONCLUSIVE", 78),
        ("ERROR", "ERROR", 65),
    ):
        result = copy.deepcopy(COMPLETE)
        result.update(outcome=outcome, success=False)
        yield f"outcome-{outcome}", encoded(result), status, exit_code
        yield f"contradictory-{outcome}", changed(("outcome",), outcome), "ERROR", 65
    for path in ((), ("identity",), ("cleanup",), ("cases", 0)):
        result = copy.deepcopy(COMPLETE)
        cursor = result
        for key in path:
            cursor = cursor[key]
        cursor["unknown"] = True
        yield "unknown-field-" + "-".join(map(str, path)), encoded(result), "ERROR", 65


class OfflineEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.work = Path(self.temp.name)

    def cli(
        self, label, raw, *, inner=0, stderr="", delay_ms=0, extra=(), manifest=MANIFEST
    ):
        # Actual OS child exit statuses, using only the fixed inert fake tool.
        manifest_path = self.work / "manifest.json"
        scenario_path = self.work / "scenario.json"
        manifest_path.write_bytes(encoded(manifest))
        scenario = {
            "exit_code": inner,
            "stdout": raw.decode("utf-8") if b"\xff" not in raw else "",
            "stderr": stderr,
            "delay_ms": delay_ms,
        }
        # Invalid UTF-8 is tested through the injected executor, because the
        # simple fake scenario schema deliberately permits Unicode text only.
        scenario_path.write_bytes(encoded(scenario))
        command = [
            sys.executable,
            "-B",
            str(ROOT / "run.py"),
            "--manifest",
            str(manifest_path),
            "--scenario",
            str(scenario_path),
            *extra,
        ]
        completed = subprocess.run(command, capture_output=True, timeout=5)
        self.assertEqual(completed.stderr, b"")
        report = json.loads(completed.stdout)
        self.assertEqual(completed.returncode, report["outer_exit_code"])
        self.assertIs(report["real_mechanism_qualified"], False)
        record_dir = os.environ.get("RONIN_CI_TEST_ARTIFACTS")
        if record_dir:
            destination = Path(record_dir)
            destination.mkdir(parents=True, exist_ok=True)
            (destination / f"{label}.json").write_bytes(
                encoded(
                    {
                        "label": label,
                        "outer_command": command,
                        "outer_actual_returncode": completed.returncode,
                        "outer_stdout": completed.stdout.decode("utf-8"),
                        "outer_stderr": completed.stderr.decode("utf-8"),
                        "manifest": manifest,
                        "scenario": scenario,
                        "report": report,
                    }
                )
                + b"\n"
            )
        return report

    def test_complete_control_is_only_a_fixture_pass(self):
        self.assertEqual(
            validate_evidence(MANIFEST, encoded(COMPLETE)).status, "FIXTURE_PASS"
        )
        report = self.cli(
            "complete-control",
            encoded(COMPLETE),
            stderr="PASS is just diagnostic text\n",
        )
        self.assertEqual(report["status"], "FIXTURE_PASS")
        self.assertIs(report["fixture_pass"], True)
        self.assertEqual(
            (report["inner_returncode"], report["outer_exit_code"]), (0, 0)
        )

    def test_each_negative_is_rejected_by_validator_and_wrapper(self):
        for label, raw, status, exit_code in negative_evidence():
            with self.subTest(label=label):
                self.assertEqual(validate_evidence(MANIFEST, raw).status, status)
                injected = run_fixture(
                    MANIFEST,
                    "unused",
                    executor=lambda command, budget: Execution(0, raw),
                )
                self.assertIs(injected["fixture_pass"], False)
                self.assertEqual(
                    (injected["status"], injected["outer_exit_code"]),
                    (status, exit_code),
                )
                if label == "invalid-utf8":
                    continue
                report = self.cli(label, raw)
                self.assertIs(report["fixture_pass"], False)
                self.assertEqual(report["status"], status)
                self.assertEqual(
                    (report["inner_returncode"], report["outer_exit_code"]),
                    (0, exit_code),
                )

    def test_nonzero_inner_exits_are_preserved_despite_complete_evidence(self):
        for inner in (42, 126, 137):
            with self.subTest(inner=inner):
                report = self.cli(f"exit-{inner}", encoded(COMPLETE), inner=inner)
                self.assertEqual(report["evidence_status"], "FIXTURE_PASS")
                self.assertIs(report["fixture_pass"], False)
                self.assertEqual(report["status"], "ERROR")
                self.assertEqual(
                    (report["inner_returncode"], report["outer_exit_code"]),
                    (inner, inner),
                )

    def test_signed_signal_status_is_retained_in_injected_result(self):
        report = run_fixture(
            MANIFEST,
            "unused",
            executor=lambda command, budget: Execution(-9, encoded(COMPLETE)),
        )
        self.assertIs(report["fixture_pass"], False)
        self.assertEqual(
            (report["inner_returncode"], report["outer_exit_code"]), (-9, 137)
        )

    def test_timeout_is_inconclusive_with_actual_inner_termination(self):
        report = self.cli(
            "timeout",
            encoded(COMPLETE),
            delay_ms=1000,
            extra=("--fake-timeout-seconds", "0.2"),
        )
        self.assertIs(report["timed_out"], True)
        self.assertIs(report["fixture_pass"], False)
        self.assertEqual(report["status"], "INCONCLUSIVE")
        self.assertEqual(
            (report["inner_returncode"], report["outer_exit_code"]), (-9, 124)
        )

    def test_combined_output_limit_cannot_hide_behind_valid_stdout(self):
        report = self.cli("output-limit", encoded(COMPLETE), stderr="x" * 70000)
        self.assertIs(report["fixture_pass"], False)
        self.assertIs(report["output_limited"], True)
        self.assertEqual((report["status"], report["outer_exit_code"]), ("ERROR", 125))
        self.assertLessEqual(
            len(report["stdout"].encode()) + len(report["stderr"].encode()), 65536
        )
        self.assertIsInstance(report["inner_returncode"], int)

    def test_fake_scenario_error_is_not_an_empty_success(self):
        report = self.cli("scenario-error", encoded(COMPLETE), inner=256)
        self.assertIs(report["fixture_pass"], False)
        self.assertEqual(
            (report["inner_returncode"], report["outer_exit_code"]), (64, 64)
        )

    def test_executor_errors_and_missing_status_are_nonpassing(self):
        for execution in (
            Execution(None),
            Execution(True, encoded(COMPLETE)),
            Execution(None, error="spawn failed"),
        ):
            with self.subTest(execution=execution):
                report = run_fixture(
                    MANIFEST, "unused", executor=lambda command, budget: execution
                )
                self.assertIs(report["fixture_pass"], False)
                self.assertEqual(
                    (report["status"], report["outer_exit_code"]), ("ERROR", 127)
                )

        def broken(command, budget):
            raise OSError("injected spawn failure")

        report = run_fixture(MANIFEST, "unused", executor=broken)
        self.assertEqual(report["outer_exit_code"], 127)

    def test_only_none_executor_error_allows_complete_fixture_pass(self):
        # Injected API controls: no fake process or real mechanism is observed.
        for error, status, outer, fixture_pass in (
            (None, "FIXTURE_PASS", 0, True),
            ("", "ERROR", 127, False),
            ("synthetic executor failure", "ERROR", 127, False),
        ):
            with self.subTest(error=error):
                report = run_fixture(
                    MANIFEST,
                    "unused",
                    executor=lambda command, budget: Execution(
                        0, encoded(COMPLETE), error=error
                    ),
                )
                self.assertEqual(report["evidence_status"], "FIXTURE_PASS")
                self.assertEqual(
                    (
                        report["status"],
                        report["inner_returncode"],
                        report["outer_exit_code"],
                    ),
                    (status, 0, outer),
                )
                self.assertIs(report["fixture_pass"], fixture_pass)
                self.assertIs(report["real_mechanism_qualified"], False)
                if error is not None:
                    self.assertEqual(report["reasons"], [error])

    def test_invalid_typed_executor_results_cannot_pass(self):
        for execution in (
            None,
            Execution(0, "not bytes"),
            Execution(0, encoded(COMPLETE), timed_out="false"),
            Execution(0, encoded(COMPLETE), output_limited=0),
            Execution(0, encoded(COMPLETE), error=False),
        ):
            with self.subTest(execution=execution):
                report = run_fixture(
                    MANIFEST, "unused", executor=lambda command, budget: execution
                )
                self.assertIs(report["fixture_pass"], False)
                self.assertEqual(
                    (report["status"], report["outer_exit_code"]), ("ERROR", 127)
                )

    def test_invalid_manifests_fail_before_executor_is_called(self):
        variants = []
        for field in MANIFEST:
            variant = copy.deepcopy(MANIFEST)
            del variant[field]
            variants.append((f"missing-{field}", variant))
        for field, value in (
            ("schema_version", True),
            ("kind", "GUEST_EXECUTION"),
            ("run_nonce", ""),
            ("identity", {}),
            ("topology", "unknown"),
            ("required_capabilities", []),
            ("required_capabilities", ["unknown"]),
            ("cases", []),
            ("cases", MANIFEST["cases"] * 2),
            ("cases", [{"id": "x", "required_progress": ["GUEST_READY"]}]),
        ):
            variant = copy.deepcopy(MANIFEST)
            variant[field] = value
            variants.append((f"bad-{field}-{len(variants)}", variant))
        variant = copy.deepcopy(MANIFEST)
        variant["unknown"] = True
        variants.append(("unknown-field", variant))

        def forbidden_executor(command, budget):
            self.fail("invalid manifest invoked executor")

        for label, manifest in variants:
            with self.subTest(label=label):
                with self.assertRaises(ContractError):
                    check_manifest(manifest)
                report = run_fixture(manifest, "unused", executor=forbidden_executor)
                self.assertEqual(
                    (report["inner_returncode"], report["outer_exit_code"]), (None, 64)
                )
                self.assertIs(report["fixture_pass"], False)
                actual = self.cli(
                    "manifest-" + label, encoded(COMPLETE), manifest=manifest
                )
                self.assertEqual(
                    (actual["inner_returncode"], actual["outer_exit_code"]), (None, 64)
                )

    def test_fake_budgets_are_finite_and_bounded(self):
        for seconds in (0, -1, float("inf"), float("nan"), 3, True):
            with self.subTest(seconds=seconds), self.assertRaises(ValueError):
                FakeBudget(seconds=seconds)
        for output_bytes in (0, -1, 65537, True, 1.5):
            with self.subTest(output_bytes=output_bytes), self.assertRaises(ValueError):
                FakeBudget(output_bytes=output_bytes)


if __name__ == "__main__":
    unittest.main()
