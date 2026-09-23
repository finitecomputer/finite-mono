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
    ready = all(check["status"] == "ok" for check in checks.values())
    status = "ok" if ready else "degraded"
    return {
        "schema_version": "finite.status.v1",
        "generated_at": isoformat(utc_now()),
        "overall_status": status,
        "exit_code": 0 if ready else 1,
        "sections": {
            "substrate_control_plane": {
                "status": status,
                "context": context,
                "checks": checks,
                "scope": "Deployment convergence only; agent chat, capacity, and recovery are unproven",
            }
        },
    }
