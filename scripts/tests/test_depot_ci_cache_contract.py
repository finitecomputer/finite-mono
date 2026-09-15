#!/usr/bin/env python3
"""Behavioral contract for cache use in native Depot CI."""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".depot" / "workflows" / "ci.yml"


def job_block(workflow: str, job: str) -> str:
    match = re.search(
        rf"(?ms)^  {re.escape(job)}:\n(.*?)(?=^  [a-z0-9][a-z0-9-]*:\n|\Z)",
        workflow,
    )
    if match is None:
        raise AssertionError(f"missing workflow job: {job}")
    return match.group(0)


class DepotCiCacheContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_native_jobs_do_not_use_github_actions_cache_api(self) -> None:
        self.assertNotIn("actions/cache", self.workflow)
        self.assertNotIn("Swatinem/rust-cache", self.workflow)
        self.assertNotRegex(self.workflow, r"(?m)^\s+cache:\s+(?:pnpm|npm|yarn)\s*$")

    def test_rust_compiler_cache_uses_depot_backed_sccache(self) -> None:
        for job in ("rust", "hermes-bridge", "devfinity-smoke"):
            block = job_block(self.workflow, job)
            self.assertIn("mozilla-actions/sccache-action@v0.0.9", block, job)
            self.assertIn("RUSTC_WRAPPER: sccache", block, job)

    def test_pnpm_store_uses_repository_named_depot_cache_disk(self) -> None:
        dashboard = job_block(self.workflow, "dashboard")
        self.assertIn("depot/cache-mount@v1", dashboard)
        self.assertIn("name: finite-mono-pnpm-store-v1", dashboard)
        self.assertIn("pnpm config set store-dir", dashboard)

    def test_pull_requests_cannot_push_to_cachix(self) -> None:
        self.assertNotIn("PR_HEAD_REPO", self.workflow)
        for job in ("hermes-bridge", "nix-service-packages"):
            block = job_block(self.workflow, job)
            self.assertIn("push|merge_group)", block, job)
            self.assertNotIn("pull_request)", block, job)
            self.assertIn("skipPush: true", block, job)
            self.assertIn("CACHIX_AUTH_TOKEN", block, job)

    def test_devfinity_handoff_is_revision_keyed_and_same_run(self) -> None:
        producer = job_block(self.workflow, "nix-service-packages")
        consumer = job_block(self.workflow, "devfinity-smoke")
        for token in (
            "${{ github.sha }}",
            "${{ github.run_id }}",
            "${{ github.run_attempt }}",
        ):
            self.assertIn(token, producer)
        self.assertIn("actions/upload-artifact@v4", producer)
        self.assertIn("devfinity_handoff_required", producer)
        self.assertIn("actions/download-artifact@v4", consumer)
        self.assertIn(
            "needs['nix-service-packages'].outputs.devfinity_handoff_artifact", consumer
        )
        self.assertIn("devfinity-nix-handoff restore", consumer)
        self.assertIn('--revision "$GITHUB_SHA"', consumer)

    def test_fully_substituted_devfinity_closure_skips_handoff(self) -> None:
        producer = job_block(self.workflow, "nix-service-packages")
        consumer = job_block(self.workflow, "devfinity-smoke")
        self.assertIn("devfinity_handoff_required == 'true'", producer)
        self.assertIn("devfinity_handoff_required != 'true'", consumer)


if __name__ == "__main__":
    unittest.main()
