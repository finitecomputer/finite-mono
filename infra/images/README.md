# infra/images — container image definitions

Every first-party image is built by CI from this repo and pushed
digest-pinned to GHCR. Nothing is built on a prod box (the on-host podman
flow on lat1 is deprecated — `infra/hosts/lat1/deploy.md`).

| Image (ghcr.io/finitecomputer/…) | Definition | Built by | Deployed to |
|---|---|---|---|
| `finite-saas-core` | `core.Dockerfile` (context: repo root) | `service-images.yml` | lat1 k3s |
| `finite-saas-dashboard` | `dashboard.Dockerfile` (context: `finitecomputer-v2/`) | `service-images.yml` | lat1 k3s |
| `private-limiter` | `private-limiter.Dockerfile` (context: repo root) | `service-images.yml` | Finite Private Tinfoil CVM (digest pinned in confidential-kimi-k2-6) |
| `agent-runtime` | `finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile` via `finitecomputer-v2/scripts/build_runtime_image.py` (stages finitechat + finite-sites + finite-brain from this tree) | `runtime-image.yml` (self-hosted lat2 runner) | Phala hosted-agent CVMs |
| `hermes-runtime` | `finitechat/containers/agent/Dockerfile` via `finitechat/scripts/hermes-build-runtime-image.py` | `hermes-runtime-smoke.yml` (self-hosted lat2 runner; publish gated on smoke proof) | Docker/Tinfoil canary lanes |

Legacy package names (`finite-private-limiter`, `finite-agent-runtime`,
`finite-chat-hermes-runtime`) are write-locked to the archived repos that
created them. Decision (Paul, 2026-07-09): no cross-grants — those packages
are FROZEN, kept public so already-deployed pins keep pulling (live Phala
CVMs, the deployed Tinfoil limiter). Mono publishes under the mono-owned
names above; consumers repoint at their next natural roll. Never delete the
frozen packages while any deployed digest references them.

Notes:

- `runtime.Dockerfile` stays next to `build_runtime_image.py` because the
  script assembles its own staged build context and references that path.
- The self-hosted-runner workflows queue forever until a lat2 runner is
  registered against finite-mono — cutover checklist in
  `infra/hosts/lat2/runners.md`.
- Version tags are date-based for images (`2026-07-08.1`); every push also
  gets a `sha-<git sha>` tag and the workflow summary prints the pinned
  `name:tag@digest` to use in manifests.
