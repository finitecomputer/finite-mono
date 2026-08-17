from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

RUNTIME_METRICS = ROOT / "infra/nixos/scripts/finite_runtime_metrics.py"
spec = importlib.util.spec_from_file_location("finite_runtime_metrics", RUNTIME_METRICS)
assert spec is not None and spec.loader is not None
finite_runtime_metrics = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = finite_runtime_metrics
spec.loader.exec_module(finite_runtime_metrics)


class FiniteRuntimeMetricsTests(unittest.TestCase):
    def test_renders_promoted_and_stale_runtime_artifacts(self) -> None:
        core = {
            "artifacts": [
                {
                    "id": "artifact-v2",
                    "reference": "ghcr.io/finite/runtime@sha256:2222",
                    "version_label": "v2",
                    "source_git_sha": "git-v2",
                    "promoted_at": "2026-08-01T00:00:00Z",
                    "retired_at": "",
                },
                {
                    "id": "artifact-v1",
                    "reference": "ghcr.io/finite/runtime@sha256:1111",
                    "version_label": "v1",
                    "source_git_sha": "git-v1",
                    "promoted_at": "2026-07-01T00:00:00Z",
                    "retired_at": "",
                },
            ],
            "runtimes": [
                {
                    "source_host_id": "finite-lat-1",
                    "runtime_artifact_id": "artifact-v2",
                    "link_state": "active",
                },
                {
                    "source_host_id": "finite-lat-1",
                    "runtime_artifact_id": "artifact-v2",
                    "link_state": "active",
                },
                {
                    "source_host_id": "finite-lat-3",
                    "runtime_artifact_id": "artifact-v1",
                    "link_state": "active",
                },
                {
                    "source_host_id": "finite-lat-3",
                    "runtime_artifact_id": "artifact-v1",
                    "link_state": "active",
                },
            ],
        }

        metrics = finite_runtime_metrics.render(core)

        self.assertIn(
            'finite_runtime_artifact_info{source_host_id="finite-lat-1",artifact_id="artifact-v2",version_label="v2",promoted="true"} 1',
            metrics,
        )
        self.assertIn(
            'finite_runtime_artifact_active_agents{source_host_id="finite-lat-1",artifact_id="artifact-v2",version_label="v2",promoted="true"} 2',
            metrics,
        )
        self.assertIn(
            'finite_runtime_artifact_active_agents{source_host_id="finite-lat-3",artifact_id="artifact-v1",version_label="v1",promoted="false"} 2',
            metrics,
        )
        self.assertIn(
            'finite_component_build_info{host="finite-lat-1",component="finite-agent-runtime",version="v2",git_sha="git-v2",image_digest="sha256:2222",source="core"} 1',
            metrics,
        )
        self.assertIn(
            'finite_component_version_mismatch{host="finite-lat-3",component="finite-agent-runtime"} 1',
            metrics,
        )
        self.assertIn(
            'finite_component_version_mismatched_active_agents{host="finite-lat-1",component="finite-agent-runtime"} 0',
            metrics,
        )
        self.assertIn(
            'finite_component_version_mismatched_active_agents{host="finite-lat-3",component="finite-agent-runtime"} 2',
            metrics,
        )

    def test_incomplete_artifact_identity_is_a_gauge_not_a_crash(self) -> None:
        # Pre-artifact-era rows (NULL artifact id, e.g. old smoke runtimes)
        # must not take the exporter down; they surface as their own gauge
        # while the healthy fleet keeps publishing.
        core = {
            "artifacts": [
                {
                    "id": "artifact-v2",
                    "reference": "ghcr.io/finite/runtime@sha256:2222",
                    "version_label": "v2",
                    "source_git_sha": "git-v2",
                    "promoted_at": "2026-08-01T00:00:00Z",
                    "retired_at": "",
                },
            ],
            "runtimes": [
                {
                    "source_host_id": "finite-lat-1",
                    "runtime_artifact_id": "artifact-v2",
                    "link_state": "active",
                },
                {
                    "source_host_id": "finite-lat-2",
                    "runtime_artifact_id": "",
                    "link_state": "active",
                },
                {
                    "source_host_id": "finite-lat-2",
                    "runtime_artifact_id": "",
                    "link_state": "active",
                },
                {
                    "source_host_id": "",
                    "runtime_artifact_id": "artifact-unknown",
                    "link_state": "active",
                },
            ],
        }

        metrics = finite_runtime_metrics.render(core)

        self.assertIn(
            'finite_runtime_incomplete_artifact_identity{source_host_id="finite-lat-2"} 2',
            metrics,
        )
        self.assertIn(
            'finite_runtime_incomplete_artifact_identity{source_host_id="unknown"} 1',
            metrics,
        )
        self.assertIn(
            'finite_runtime_artifact_info{source_host_id="finite-lat-1",artifact_id="artifact-v2",version_label="v2",promoted="true"} 1',
            metrics,
        )
        self.assertIn(
            'finite_runtime_artifact_active_agents{source_host_id="finite-lat-1",artifact_id="artifact-v2",version_label="v2",promoted="true"} 1',
            metrics,
        )
        self.assertIn(
            'finite_component_version_mismatched_active_agents{host="finite-lat-1",component="finite-agent-runtime"} 0',
            metrics,
        )

    def test_atomic_write_replaces_the_complete_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "runtime.prom"
            output.write_text("old\n", encoding="utf-8")

            finite_runtime_metrics.write_atomic(output, "new\n")

            self.assertEqual(output.read_text(encoding="utf-8"), "new\n")
            self.assertEqual(output.stat().st_mode & 0o777, 0o640)
            self.assertEqual(list(output.parent.glob("*.tmp.*")), [])


if __name__ == "__main__":
    unittest.main()
