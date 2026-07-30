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
  FFmpeg media work is confined to a 256 MiB ephemeral workspace with four
  concurrent normalization slots. Image, audio, and video semantic canaries
  each run every ten minutes, phased 200 seconds apart to avoid synchronized
  outbound bursts while remaining inside the 15-minute stale-health window.

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

## Embedding plaintext preflight

Semantic embedding is an internal-beta plaintext boundary. It stays disabled
unless two root-owned secret-boundary files exist beside `worker-token` and
`spark-gateway-token`:

- `embedding-plaintext-policy` must contain exactly
  `verified-no-content-logging-no-retention-v1`.
- `embedding-policy-evidence-id` must contain the reviewed, non-secret evidence
  record identifier for the current upstream/model policy.

Create or update those files only after reviewing both the worker and the
upstream `https://inference.finite.computer/v1` configuration. The review must
prove request bodies are not logged and embedding inputs are not retained; an
undocumented provider default is not evidence. Keep the evidence itself in the
approved private operator record, not this public repository. The worker emits
only request status/timing, bounded batch/model identity, opaque request
identifiers, and error categories; never section/query text or bearer tokens.

Runner must additionally set
`FC_RUNNER_FINITE_PRIVATE_SPECIALIZATION_DEPLOYMENT_VERIFIED=true` and
`FC_RUNNER_FINITE_PRIVATE_SPECIALIZATION_POLICY_EVIDENCE_ID` only after this
manifest points at a worker digest containing the verified-policy health gate.
Without both values Runner intentionally withholds the embedding endpoint and
credential from new runtimes, so the currently pinned pre-gate image cannot
receive Brain plaintext through this deployment path.

Before allowing any Runtime to enable semantic search, verify without printing
credentials or plaintext:

```sh
health=$(curl --fail --silent https://specialization.finite.vip/health)
printf '%s' "$health" | jq -e '
  .embedding.enabled == true and
  .embedding.plaintextPolicy == "verified" and
  .embedding.model == "nomic-embed-text-v1-5" and
  (.embedding.policyEvidenceId | type == "string" and length > 0)
' >/dev/null
```

The Agent Runtime receives `FBRAIN_EMBEDDING_ENDPOINT` and the independently
revocable `FBRAIN_EMBEDDING_BEARER_TOKEN` from the existing specialization
bundle secret boundary. Missing/revoked configuration leaves `fbrain` lexical
only and does not block sync. To roll back, revoke/rotate the specialization
worker credential, remove the two embedding policy files, restart the worker,
and verify `.embedding.enabled == false`; BM25 and authoritative Markdown stay
available. Centralized plaintext trust is temporary for the internal beta and
must not become a production default without a new reviewed privacy design.

## Rollback

If rollout or semantic verification fails:

```bash
k3s kubectl -n fc-specializations rollout undo deployment/finite-specialization-worker
k3s kubectl -n fc-specializations rollout status deployment/finite-specialization-worker --timeout=180s
```

Restore the backed-up k3s manifest after the old replica is ready. Disable only
the failing capability in the agentd desired state, leaving successful
capabilities, Hermes text inference, and the Spark AEON route untouched.
