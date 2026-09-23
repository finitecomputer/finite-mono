"""Read-only Substrate deployment evidence for scripts/finite-status."""

import json

from finite_status import CollectionError, isoformat, run_read_only, utc_now

DEPLOYMENTS = ("ate-api-server", "ate-controller", "atenet-router", "atenet-egress")


def deployment_evidence(deployment):
    metadata = deployment["metadata"]
    spec = deployment["spec"]
    status = deployment.get("status", {})
    desired = spec.get("replicas", 1)
    ready = (
        desired > 0
        and status.get("observedGeneration", 0) >= metadata["generation"]
        and status.get("updatedReplicas", 0) == desired
        and status.get("availableReplicas", 0) == desired
        and status.get("replicas", 0) == desired
    )
    return {
        "status": "ok" if ready else "degraded",
        "generation": metadata["generation"],
        "observed_generation": status.get("observedGeneration", 0),
        "desired_replicas": desired,
        "available_replicas": status.get("availableReplicas", 0),
        "images": [c["image"] for c in spec["template"]["spec"]["containers"]],
    }


def worker_evidence(workers, pods):
    by_name = {
        (pod["metadata"]["namespace"], pod["metadata"]["name"]): pod for pod in pods
    }
    checks = {}
    for worker in workers:
        pod = by_name.get((worker["workerNamespace"], worker["workerPod"]), {})
        metadata = pod.get("metadata", {})
        current_ip = pod.get("status", {}).get("podIP")
        ready = bool(current_ip) and (
            worker["workerPodUid"] == metadata.get("uid") and worker["ip"] == current_ip
        )
        checks[worker["metadata"]["name"]] = {
            "status": "ok" if ready else "degraded",
            "namespace": worker["workerNamespace"],
            "pod": worker["workerPod"],
            "registered_ip": worker["ip"],
            "pod_ip": current_ip,
            "pod_uid_matches": worker["workerPodUid"] == metadata.get("uid"),
        }
    return checks


def worker_checks(context):
    workers = run_read_only(
        ["kubectl", "ate", "--context", context, "get", "workers", "-o", "json"]
    )
    pods = run_read_only(
        [
            "kubectl",
            "--context",
            context,
            "--request-timeout=10s",
            "get",
            "pods",
            "-A",
            "-l",
            "ate.dev/worker-pool",
            "-o",
            "json",
        ]
    )
    if workers.returncode or pods.returncode:
        raise CollectionError(
            "Substrate worker query failed; check kubectl ate and Kubernetes access"
        )
    try:
        return worker_evidence(
            json.loads(workers.stdout).get("workers", []),
            json.loads(pods.stdout)["items"],
        )
    except (AttributeError, KeyError, TypeError, ValueError) as error:
        raise CollectionError("Invalid Substrate worker evidence") from error


def collect(context):
    result = run_read_only(
        [
            "kubectl",
            "--context",
            context,
            "--request-timeout=10s",
            "-n",
            "ate-system",
            "get",
            "deployments",
            *DEPLOYMENTS,
            "-o",
            "json",
        ]
    )
    if result.returncode:
        raise CollectionError(
            "Substrate deployment query failed; check Kubernetes access"
        )
    try:
        checks = {
            item["metadata"]["name"]: deployment_evidence(item)
            for item in json.loads(result.stdout)["items"]
        }
        if set(checks) != set(DEPLOYMENTS):
            raise ValueError("missing deployment")
    except (KeyError, TypeError, ValueError) as error:
        raise CollectionError("Invalid Substrate deployment evidence") from error
    workers = worker_checks(context)
    workers_ready = bool(workers) and all(
        check["status"] == "ok" for check in workers.values()
    )
    deployments_ready = all(check["status"] == "ok" for check in checks.values())
    ready = deployments_ready and workers_ready
    status = "ok" if ready else "degraded"
    return {
        "schema_version": "finite.status.v1",
        "generated_at": isoformat(utc_now()),
        "overall_status": status,
        "exit_code": 0 if ready else 1,
        "sections": {
            "substrate_control_plane": {
                "status": "ok" if deployments_ready else "degraded",
                "context": context,
                "checks": checks,
                "scope": "Deployment convergence only; agent chat, capacity, and recovery are unproven",
            },
            "substrate_worker_routing": {
                "status": "ok" if workers_ready else "degraded",
                "context": context,
                "checks": workers,
                "scope": "Registered worker/pod UID and IP agreement only; live actor routing is unproven",
            },
        },
    }
