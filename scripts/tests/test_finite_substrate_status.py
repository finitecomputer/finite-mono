import copy
import json
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import finite_substrate_status as status


class SubstrateStatusTests(unittest.TestCase):
    def deployment(self):
        return {
            "metadata": {"name": "ate-api-server", "generation": 2},
            "spec": {
                "replicas": 2,
                "template": {"spec": {"containers": [{"image": "test@sha256:123"}]}},
            },
            "status": {
                "observedGeneration": 2,
                "updatedReplicas": 2,
                "availableReplicas": 2,
                "replicas": 2,
            },
        }

    def test_rollout_requires_current_generation_and_all_replicas(self):
        deployment = self.deployment()
        self.assertEqual(status.deployment_evidence(deployment)["status"], "ok")
        for field, value in (
            ("observedGeneration", 1),
            ("updatedReplicas", 1),
            ("availableReplicas", 1),
            ("replicas", 3),
        ):
            candidate = copy.deepcopy(deployment)
            candidate["status"][field] = value
            self.assertEqual(
                status.deployment_evidence(candidate)["status"], "degraded", field
            )
        deployment["spec"]["replicas"] = 0
        self.assertEqual(status.deployment_evidence(deployment)["status"], "degraded")

    def test_node_restart_ip_drift_and_pod_replacement_are_not_healthy(self):
        worker = {
            "metadata": {"name": "worker-1"},
            "workerNamespace": "agents",
            "workerPod": "worker-pod",
            "workerPodUid": "pod-uid",
            "ip": "10.244.0.33",
        }
        pod = {
            "metadata": {"namespace": "agents", "name": "worker-pod", "uid": "pod-uid"},
            "status": {"podIP": "10.244.0.33"},
        }
        self.assertEqual(
            status.worker_evidence([worker], [pod])["worker-1"]["status"], "ok"
        )
        pod["status"]["podIP"] = "10.244.0.16"
        self.assertEqual(
            status.worker_evidence([worker], [pod])["worker-1"]["status"], "degraded"
        )
        pod["status"]["podIP"] = worker["ip"]
        pod["metadata"]["uid"] = "replacement-pod"
        self.assertEqual(
            status.worker_evidence([worker], [pod])["worker-1"]["status"], "degraded"
        )
        self.assertEqual(
            status.worker_evidence([worker], [])["worker-1"]["status"], "degraded"
        )

    def test_healthy_deployments_do_not_mask_missing_workers(self):
        deployments = []
        for name in status.DEPLOYMENTS:
            deployment = self.deployment()
            deployment["metadata"]["name"] = name
            deployments.append(deployment)
        results = [
            subprocess.CompletedProcess([], 0, json.dumps(value), "")
            for value in ({"items": deployments}, {"workers": []}, {"items": []})
        ]
        with patch.object(status, "run_read_only", side_effect=results):
            report = status.collect("test-context")
        self.assertEqual(report["sections"]["substrate_control_plane"]["status"], "ok")
        self.assertEqual(report["overall_status"], "degraded")
        self.assertEqual(report["exit_code"], 1)

    def test_missing_control_plane_is_not_healthy(self):
        result = subprocess.CompletedProcess(
            [], 0, json.dumps({"items": [self.deployment()]}), ""
        )
        with patch.object(status, "run_read_only", return_value=result):
            with self.assertRaises(status.CollectionError):
                status.collect("test-context")

    def test_query_failure_does_not_echo_credentials(self):
        result = subprocess.CompletedProcess([], 1, "", "credential-plugin-secret")
        with patch.object(status, "run_read_only", return_value=result) as run:
            with self.assertRaises(status.CollectionError) as raised:
                status.collect("test-context")
            self.assertNotIn("credential-plugin-secret", str(raised.exception))
            self.assertIn("test-context", run.call_args.args[0])


if __name__ == "__main__":
    unittest.main()
