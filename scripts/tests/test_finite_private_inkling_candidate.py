from __future__ import annotations

from copy import deepcopy
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

import yaml

from scripts.check_finite_private_inkling_candidate import (
    BASELINE,
    CANDIDATE,
    IMAGE_STOP,
    MPK_STOP,
    ROOT_STOP,
    check_repository,
)


ROOT = Path(__file__).resolve().parents[2]


class CandidateTests(unittest.TestCase):
    def check_changed(self, change, *, artifacts_ready=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / BASELINE).parent.mkdir(parents=True)
            (root / BASELINE).write_text((ROOT / BASELINE).read_text())
            candidate = yaml.safe_load((ROOT / CANDIDATE).read_text())
            change(candidate)
            (root / CANDIDATE).write_text(yaml.safe_dump(candidate))
            return check_repository(root, artifacts_ready=artifacts_ready)

    def test_preparation_passes_but_artifacts_are_explicitly_blocked(self):
        self.assertEqual(check_repository(ROOT), [])
        violations = check_repository(ROOT, artifacts_ready=True)
        self.assertEqual(len(violations), 2)
        self.assertIn("image", violations[0])
        self.assertIn("MPK", violations[1])

    def test_ready_requires_digest_and_matching_modelwrap_mount(self):
        def pin(candidate):
            candidate["containers"][0]["image"] = (
                "ghcr.io/finitecomputer/inkling-2-small-sglang@sha256:" + "a" * 64
            )
            candidate["models"][0]["mpk"] = (
                "b" * 64 + "_393509649692_11111111-2222-3333-4444-555555555555"
            )
            command = candidate["containers"][0]["command"]
            command[1] = command[1].replace(ROOT_STOP, "b" * 64)

        self.assertEqual(self.check_changed(pin, artifacts_ready=True), [])

        def mismatch(candidate):
            pin(candidate)
            candidate["containers"][0]["command"][1] = "/tinfoil/mpk/mpk-" + "c" * 64

        self.assertTrue(self.check_changed(mismatch, artifacts_ready=True))

    def test_security_admission_aliases_and_limits_cannot_drift(self):
        original = yaml.safe_load((ROOT / CANDIDATE).read_text())
        variants = []
        for path, value in (
            (("security", "allow-unencrypted-nvlink"), False),
            (("cvm-version",), "0.10.8"),
            (("gpus",), 4),
            (("containers", 0, "ipc"), "host"),
            (("containers", 0, "secrets"), []),
            (("containers", 1, "secrets"), []),
            (("shim", "paths"), ["/v1/chat/completions"]),
            (("models", 0, "repo"), "thinkingmachines/Inkling-2-Small@main"),
        ):
            candidate = deepcopy(original)
            target = candidate
            for key in path[:-1]:
                target = target[key]
            target[path[-1]] = value
            variants.append(candidate)
        for candidate in variants:
            with self.subTest(candidate=candidate):
                self.assertTrue(self.check_changed(lambda d: d.update(candidate)))
        self.assertTrue(
            self.check_changed(
                lambda d: d["containers"][1]["env"].append(
                    {"FINITE_PRIVATE_MODEL_ALIASES": "*"}
                )
            )
        )

    def test_unmatched_placeholders_and_mutable_images_fail_even_in_prep(self):
        for image in ("lmsysorg/sglang:latest", IMAGE_STOP + "-typo"):
            self.assertTrue(
                self.check_changed(lambda d: d["containers"][0].update(image=image))
            )
        self.assertTrue(
            self.check_changed(lambda d: d["models"][0].update(mpk=MPK_STOP + "-typo"))
        )

    def test_duplicate_security_key_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / CANDIDATE).parent.mkdir(parents=True)
            (root / BASELINE).write_text((ROOT / BASELINE).read_text())
            (root / CANDIDATE).write_text(
                (ROOT / CANDIDATE).read_text()
                + "\nsecurity:\n  allow-unencrypted-nvlink: false\n"
            )
            self.assertTrue(check_repository(root))

    def test_wrapper_requires_key_and_preserves_server_arguments(self):
        entrypoint = ROOT / "infra/images/inkling-2-small-sglang-entrypoint.sh"
        with tempfile.TemporaryDirectory() as directory:
            fake = Path(directory) / "sglang"
            fake.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n')
            fake.chmod(0o755)
            env = {"PATH": directory + os.pathsep + os.defpath}
            closed = subprocess.run(
                ["sh", str(entrypoint), "--help"],
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(closed.returncode, 0)
            self.assertIn("VLLM_API_KEY is required", closed.stderr)
            opened = subprocess.run(
                ["sh", str(entrypoint), "--tp", "8"],
                env=dict(env, VLLM_API_KEY="synthetic-key"),
                capture_output=True,
                text=True,
                check=True,
            )
            self.assertEqual(
                opened.stdout.splitlines(),
                ["serve", "--api-key", "synthetic-key", "--tp", "8"],
            )


if __name__ == "__main__":
    unittest.main()
