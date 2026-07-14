# AEON specialization worker deployment

This host directory owns the first-party worker manifest deployed to the legacy
clawland k3s cluster. The host remains outside the general finite-mono deploy
plane; this narrow exception replaces the legacy worker source with a
digest-pinned finite-mono image.

## Safety boundary

- Back up `/var/lib/rancher/k3s/server/manifests/fc-specializations.yaml`.
- Confirm the current worker is ready and an authenticated image smoke passes.
- Apply `finite-specialization-worker.yaml` with `kubectl apply`.
- Wait for the deployment rollout before changing any Hermes configuration.
- Do not mount the old relay/Core token. Reconciliation belongs to
  `finite-agentd`, not this request worker.
- The container runs as root only because the existing host-managed secret
  files are root-owned mode `0400`. It has no Linux capabilities, no service
  account token, no privilege escalation, and a read-only root filesystem.
  FFmpeg media work is confined to a 256 MiB ephemeral workspace with two
  concurrent normalization slots.

## Verification

```bash
k3s kubectl -n fc-specializations rollout status deployment/finite-specialization-worker --timeout=180s
k3s kubectl -n fc-specializations get pod -l app.kubernetes.io/name=finite-specialization-worker
curl --fail --silent http://127.0.0.1:30998/health
curl --fail --silent https://specialization.finite.vip/health
curl --fail --silent http://127.0.0.1:30998/metrics
```

Run authenticated `/v1/chat/completions` requests for one deterministic image,
audio clip, and video clip using the existing worker token from the host secret
directory. Verify the normal Chat Completions answer and the top-level
`specialization_result` fields without printing either credential. Confirm
that `/metrics` reports fresh, independent `image`, `audio`, and `video`
capability health.

## Rollback

If rollout or semantic verification fails:

```bash
k3s kubectl -n fc-specializations rollout undo deployment/finite-specialization-worker
k3s kubectl -n fc-specializations rollout status deployment/finite-specialization-worker --timeout=180s
```

Restore the backed-up k3s manifest after the old replica is ready. Disable only
the failing capability in the agentd desired state, leaving successful
capabilities, Hermes text inference, and the Spark AEON route untouched.
