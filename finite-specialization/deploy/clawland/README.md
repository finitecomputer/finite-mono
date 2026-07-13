# AEON specialization worker deployment

This directory owns the first-party worker manifest deployed to the legacy
clawland k3s cluster. The host remains outside the general finite-mono deploy
plane; this narrow exception replaces the legacy worker source with a
digest-pinned finite-mono image.

## Safety boundary

- Back up `/var/lib/rancher/k3s/server/manifests/fc-specializations.yaml`.
- Confirm the current worker is ready and an authenticated image smoke passes.
- Apply `finite-specialization-worker.yaml` with `kubectl apply --server-side`.
- Wait for the deployment rollout before changing any Hermes configuration.
- Do not mount the old relay/Core token. Reconciliation belongs to
  `finite-agentd`, not this request worker.
- The container runs as root only because the existing host-managed secret
  files are root-owned mode `0400`. It has no Linux capabilities, no service
  account token, no privilege escalation, and a read-only root filesystem.

## Verification

```bash
k3s kubectl -n fc-specializations rollout status deployment/finite-specialization-worker --timeout=180s
k3s kubectl -n fc-specializations get pod -l app.kubernetes.io/name=finite-specialization-worker
curl --fail --silent http://127.0.0.1:30998/health
curl --fail --silent http://127.0.0.1:30998/metrics
```

Run an authenticated `/v1/chat/completions` image request using the existing
worker token from the host secret directory. Verify the normal Chat
Completions answer and the top-level `specialization_result` fields without
printing either credential.

## Rollback

If rollout or semantic verification fails:

```bash
k3s kubectl -n fc-specializations rollout undo deployment/finite-specialization-worker
k3s kubectl -n fc-specializations rollout status deployment/finite-specialization-worker --timeout=180s
```

Restore the backed-up k3s manifest after the old replica is ready. A worker
failure is capability-local: leave Hermes text inference and the Spark AEON
route untouched.
