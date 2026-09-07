# infra/images — container image definitions

Every first-party image is built by CI from this repo and pushed
digest-pinned to GHCR. Nothing is built on a prod box (the pre-cutover
on-host podman flow died with the k3s control plane).

**Post-cutover note (2026-07-09):** lat1 is NixOS now. `finite-saas-core`
runs from the nix-built binary (the `finite-saas-core` package), NOT the
container image — the core image below is retained for provenance / other
contexts. The dashboard runs as a digest-pinned oci-container (podman) on
lat1. `private-limiter` is the Tinfoil surface; the one Agent Runtime image
targets Kata first and Phala next. See `infra/nixos/` for what lat1 actually
runs.

| Image (ghcr.io/finitecomputer/…) | Definition | Built by | Deployed to |
|---|---|---|---|
| `finite-saas-core` | `core.Dockerfile` (context: repo root) | `service-images.yml` | (retained; lat1 runs the nix binary, not this image) |
| `finite-saas-dashboard` | `dashboard.Dockerfile` (context: repo root; includes the shared Finite Chat UI package) | `service-images.yml` | lat1 (podman oci-container, digest-pinned in `modules/dashboard.nix`) |
| `private-limiter` | `private-limiter.Dockerfile` (context: repo root) | `service-images.yml` | Finite Private Tinfoil CVM (digest pinned in confidential-finite-private) |
| `glm-5-3-flash-sglang` | `glm-5.3-flash-sglang.Dockerfile` (context: repo root; wraps the exact upstream amd64 manifest with source labels and fail-closed internal auth) | `glm-5.3-flash-sglang-image.yml` | Live Finite Private GLM-5.3-Flash Tinfoil container |
| `agent-runtime` | `finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile` via `finitecomputer-v2/scripts/build_runtime_image.py` (one staged monorepo + root lockfile) | `runtime-image.yml`, whose build-once smoke proves the exact local image ID before push; `hermes-runtime-smoke.yml` is optional source preflight | local Docker, Kata, Phala, and agent canary lanes |

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
- The Runtime's baseline CLIs are defined by
  `finitecomputer-v2/deploy/finite-computer/images/agent-runtime-toolchains.nix`
  (`.#agent-runtime-toolchains` on the root flake). Its `bins` passthru is the
  single authority for which CLI names the image exposes; the build carries it
  as the `AGENT_RUNTIME_TOOLCHAIN_BINS` build-arg and the Dockerfile and
  workflow probes loop over that list — do not enumerate the names elsewhere.
- `runtime.Dockerfile` is the only Agent Runtime Dockerfile in the tree.
  Component tests that need the image consume `build_runtime_image.py`
  output or a published `agent-runtime` tag; do not add a second
  Dockerfile as a "test fixture" (the former
  `finitechat/containers/agent/Dockerfile` drifted from the product image
  and was deleted in ownership audit O12).
- Image workflows run in native Depot CI with Depot remote builders; lat2 is
  not required for Docker CI. Set `DEPOT_PROJECT_ID` as a Depot repository
  variable, or override by lane with
  `DEPOT_SERVICE_IMAGES_PROJECT_ID`, `DEPOT_RUNTIME_IMAGE_PROJECT_ID`, or
  `DEPOT_DEEPSEEK_VLLM_PROJECT_ID`. The workflows authenticate via
  `depot/setup-action` OIDC.
- Version tags are date-based for images (`2026-07-08.1`). The guarded
  Depot workflows first publish and verify a non-production canary tag from the
  saved OCI build. Production `:<version>` and `:sha-<git sha>` tags are
  promoted from that same saved build only when the explicit production-publish
  input and repository variable are both enabled. Workflow summaries print the
  pinned `name@digest` or `name:tag@digest` to use in manifests.
- After the GitHub/Depot CI cutover, dispatch production image publishers
  through Depot on reviewed GitHub `main`, then verify the resolved source
  revision and record the pinned digest printed by the workflow. For example:

  ```sh
  git fetch origin --prune
  REV="$(git rev-parse HEAD)"
  [[ "$REV" =~ ^[0-9a-f]{40}$ ]]
  git merge-base --is-ancestor "$REV" origin/main
  depot ci dispatch \
    --org scthc5h66g \
    --repo finitecomputer/finite-mono \
    --workflow service-images.yml \
    --ref main \
    --input rev="$REV" \
    --input image=dashboard \
    --input version=2026-07-08.1 \
    --input publish_production=true \
    --output json

  depot ci dispatch \
    --org scthc5h66g \
    --repo finitecomputer/finite-mono \
    --workflow deepseek-v4-vllm-image.yml \
    --ref main \
    --input rev="$REV" \
    --input version=0.25.1-0731-reasoning.1 \
    --input publish_production=true \
    --output json
  ```

  These workflows publish verified GHCR digests only. They do not edit a
  production manifest or host and do not restart a workload; promotion of the
  pinned digest remains a separate runbook-controlled action.
